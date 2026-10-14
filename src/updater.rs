//! Update check: reads the `latest.json` manifest that the release workflow
//! (`.github/workflows/release.yml`) attaches to every GitHub release and compares
//! its version with this build. When modev was installed from one of the release's
//! packages, `install` downloads the new one and puts it in place, and `restart`
//! relaunches modev on it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// `releases/latest/download/…` always resolves to the newest published (non-draft,
/// non-prerelease) release.
const MANIFEST_URL: &str =
    "https://github.com/yosoyjaff/modev/releases/latest/download/latest.json";

#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    notes: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    os: String,
    arch: String,
    kind: String,
    url: String,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub version: String,
    /// Release page (notes and every package).
    pub notes: String,
    /// Package to install for this OS/arch and the way modev was installed, if any.
    pub package: Option<Package>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    url: String,
    kind: String,
    sha256: String,
}

/// An update already in place (or, on Windows, ready to run): `restart` finishes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub version: String,
    /// Windows: the msi, run once modev has exited (its files are in use until then).
    msi: Option<PathBuf>,
}

/// Blocking: run it off the main thread. `Ok(None)` = already up to date.
pub fn check() -> Result<Option<Update>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .new_agent();
    let mut res = match agent
        .get(MANIFEST_URL)
        .header("User-Agent", concat!("modev/", env!("CARGO_PKG_VERSION")))
        .call()
    {
        // No published release (or one without a manifest): nothing to offer.
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        res => res.context("could not reach GitHub")?,
    };
    let body = res
        .body_mut()
        .read_to_string()
        .context("could not read the release manifest")?;
    let manifest: Manifest = serde_json::from_str(&body).context("invalid release manifest")?;
    Ok(update_from(
        manifest,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        install_kind(),
    ))
}

/// `kind`: package kind matching how this copy was installed (`install_kind`).
fn update_from(
    m: Manifest,
    current: &str,
    os: &str,
    arch: &str,
    kind: Option<&str>,
) -> Option<Update> {
    if !is_newer(&m.version, current) {
        return None;
    }
    let package = kind.and_then(|kind| {
        m.assets
            .into_iter()
            .find(|a| a.os == os && a.arch == arch && a.kind == kind)
            .map(|a| Package {
                url: a.url,
                kind: a.kind,
                sha256: a.sha256,
            })
    });
    Some(Update {
        version: m.version,
        notes: m.notes,
        package,
    })
}

/// Which release package this copy came from, i.e. the one that can replace it.
/// `None` (a dev build, the portable zip, an unknown layout): only the release page.
fn install_kind() -> Option<&'static str> {
    if cfg!(debug_assertions) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    match std::env::consts::OS {
        "macos" => app_bundle(&exe).map(|_| "dmg"),
        "windows" => {
            let installed = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
                .iter()
                .filter_map(std::env::var_os)
                .any(|dir| exe.starts_with(dir));
            installed.then_some("msi")
        }
        _ => {
            let owned_by = |cmd: &str, flag: &str| {
                Command::new(cmd)
                    .arg(flag)
                    .arg(&exe)
                    .output()
                    .is_ok_and(|o| o.status.success())
            };
            if owned_by("dpkg", "-S") {
                Some("deb")
            } else if owned_by("rpm", "-qf") {
                Some("rpm")
            } else {
                // The tarball: the binary is replaced in place, so it must be writable.
                let dir = exe.parent()?;
                let probe = dir.join(".modev-update-probe");
                let writable = std::fs::File::create(&probe).is_ok();
                let _ = std::fs::remove_file(probe);
                writable.then_some("tarball")
            }
        }
    }
}

/// `…/modev.app` for `…/modev.app/Contents/MacOS/modev`.
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// Blocking: downloads the package, checks its SHA-256 and installs it. On Windows it
/// only downloads: the msi runs on `restart`.
pub fn install(update: &Update) -> Result<Installed> {
    let pkg = update
        .package
        .as_ref()
        .context("no package for this installation")?;
    let dir = std::env::temp_dir().join(format!("modev-update-{}", update.version));
    std::fs::create_dir_all(&dir)?;
    let name = pkg.url.rsplit('/').next().unwrap_or("package");
    let file = dir.join(name);
    download(pkg, &file)?;
    let exe = std::env::current_exe()?;
    let mut msi = None;
    match pkg.kind.as_str() {
        "dmg" => install_dmg(&file, &dir, &exe)?,
        "msi" => msi = Some(file),
        "deb" => run(Command::new("pkexec").args(["dpkg", "-i"]).arg(&file))?,
        "rpm" => run(Command::new("pkexec").args(["rpm", "-U"]).arg(&file))?,
        "tarball" => install_tarball(&file, &dir, &exe)?,
        kind => bail!("cannot install a {kind} package"),
    }
    Ok(Installed {
        version: update.version.clone(),
        msi,
    })
}

fn download(pkg: &Package, to: &Path) -> Result<()> {
    let agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .build()
        .new_agent();
    let res = agent
        .get(&pkg.url)
        .header("User-Agent", concat!("modev/", env!("CARGO_PKG_VERSION")))
        .call()
        .context("could not download the update")?;
    let mut body = res.into_body().into_reader();
    let mut out = std::fs::File::create(to)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = body.read(&mut buf).context("download interrupted")?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !got.eq_ignore_ascii_case(&pkg.sha256) {
        let _ = std::fs::remove_file(to);
        bail!("the download is corrupt (checksum mismatch)");
    }
    Ok(())
}

/// Mounts the dmg and swaps the running `.app` for the one inside. The running
/// process keeps its already-open files; the new bundle is used from the restart on.
fn install_dmg(dmg: &Path, dir: &Path, exe: &Path) -> Result<()> {
    let app = app_bundle(exe).context("modev is not running from an .app")?;
    let mount = dir.join("mnt");
    std::fs::create_dir_all(&mount)?;
    run(Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-quiet", "-mountpoint"])
        .arg(&mount)
        .arg(dmg))?;
    let staged = app.with_extension("app.new");
    let _ = std::fs::remove_dir_all(&staged);
    let copied = app
        .file_name()
        .map(|name| mount.join(name))
        .context("bad bundle path")
        .and_then(|src| run(Command::new("ditto").arg(src).arg(&staged)));
    let _ = run(Command::new("hdiutil")
        .args(["detach", "-quiet"])
        .arg(&mount));
    copied?;
    let old = app.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(&app, &old).context("could not replace modev.app")?;
    if let Err(e) = std::fs::rename(&staged, &app) {
        let _ = std::fs::rename(&old, &app);
        return Err(e).context("could not replace modev.app");
    }
    let _ = std::fs::remove_dir_all(old);
    Ok(())
}

/// Extracts the tarball and renames its binary over the running one (Linux allows it:
/// the process keeps the old inode).
fn install_tarball(tarball: &Path, dir: &Path, exe: &Path) -> Result<()> {
    let out = dir.join("x");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    run(Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(&out))?;
    let bin = std::fs::read_dir(&out)?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("bin").join("modev"))
        .find(|p| p.is_file())
        .context("no modev binary in the tarball")?;
    let staged = exe.with_extension("new");
    std::fs::copy(&bin, &staged)?;
    std::fs::rename(&staged, exe).context("could not replace the modev binary")?;
    Ok(())
}

fn run(cmd: &mut Command) -> Result<()> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .output()
        .with_context(|| format!("could not run {name}"))?;
    if !out.status.success() {
        bail!(
            "{name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Starts a helper that waits for this process to exit, then (Windows) runs the msi
/// and relaunches modev with the same arguments. The caller quits right after.
pub fn restart(installed: &Installed) -> Result<()> {
    let pid = std::process::id();
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cfg!(windows) {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let mut script = format!("Wait-Process -Id {pid} -ErrorAction SilentlyContinue; ");
        if let Some(msi) = &installed.msi {
            script += &format!(
                "Start-Process msiexec -Wait -ArgumentList '/i',{},'/passive','/norestart'; ",
                quote(&format!("\"{}\"", msi.display()))
            );
        }
        script += &format!("Start-Process {}", quote(&exe.to_string_lossy()));
        if !args.is_empty() {
            let list: Vec<String> = args.iter().map(|a| quote(a)).collect();
            script += &format!(" -ArgumentList {}", list.join(","));
        }
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn().context("could not start the updater")?;
    } else {
        // `open` relaunches the bundle as a new app on macOS; elsewhere the binary itself.
        let launch = match app_bundle(&exe) {
            Some(app) if cfg!(target_os = "macos") => vec![
                "open".into(),
                "-n".into(),
                app.to_string_lossy().into_owned(),
                "--args".into(),
            ],
            _ => vec![exe.to_string_lossy().into_owned()],
        };
        Command::new("sh")
            .arg("-c")
            .arg("while kill -0 \"$0\" 2>/dev/null; do sleep 0.2; done; exec \"$@\"")
            .arg(pid.to_string())
            .args(launch)
            .args(args)
            .spawn()
            .context("could not start the updater")?;
    }
    Ok(())
}

/// `major.minor.patch` comparison. A pre-release (`1.2.0-beta`) is older than its
/// release and is never offered over the same version.
fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<([u64; 3], bool)> {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((core, _)) => (core, true),
            None => (v, false),
        };
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        let n = [parts.next()??, parts.next()??, parts.next()??];
        parts.next().is_none().then_some((n, pre))
    }
    match (parse(latest), parse(current)) {
        (Some((l, l_pre)), Some((c, c_pre))) => l > c || (l == c && c_pre && !l_pre),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> Manifest {
        let asset = |os: &str, arch: &str, kind: &str| Asset {
            os: os.into(),
            arch: arch.into(),
            kind: kind.into(),
            url: format!("https://dl/{os}-{arch}.{kind}"),
            sha256: "abc".into(),
        };
        Manifest {
            version: version.into(),
            notes: "https://notes".into(),
            assets: vec![
                asset("macos", "aarch64", "dmg"),
                asset("windows", "x86_64", "zip"),
                asset("windows", "x86_64", "msi"),
                asset("linux", "x86_64", "tarball"),
                asset("linux", "x86_64", "deb"),
            ],
        }
    }

    fn url(u: Option<Update>) -> Option<String> {
        u.unwrap().package.map(|p| p.url)
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert!(!is_newer("1.2", "0.1.0"));
    }

    #[test]
    fn picks_the_package_for_this_installation() {
        let u = update_from(manifest("9.0.0"), "0.1.0", "macos", "aarch64", Some("dmg"));
        assert_eq!(u.as_ref().unwrap().version, "9.0.0");
        assert_eq!(url(u).as_deref(), Some("https://dl/macos-aarch64.dmg"));
        let u = update_from(manifest("9.0.0"), "0.1.0", "windows", "x86_64", Some("msi"));
        assert_eq!(url(u).as_deref(), Some("https://dl/windows-x86_64.msi"));
        let u = update_from(manifest("9.0.0"), "0.1.0", "linux", "x86_64", Some("deb"));
        assert_eq!(url(u).as_deref(), Some("https://dl/linux-x86_64.deb"));
        // No package for this arch or kind, or an unknown installation: only the release page.
        let u = update_from(
            manifest("9.0.0"),
            "0.1.0",
            "windows",
            "aarch64",
            Some("msi"),
        );
        assert_eq!(url(u), None);
        let u = update_from(manifest("9.0.0"), "0.1.0", "linux", "x86_64", Some("rpm"));
        assert_eq!(url(u), None);
        let u = update_from(manifest("9.0.0"), "0.1.0", "windows", "x86_64", None);
        assert_eq!(url(u), None);
    }

    #[test]
    fn up_to_date_is_none() {
        assert_eq!(
            update_from(manifest("0.1.0"), "0.1.0", "macos", "aarch64", Some("dmg")),
            None
        );
    }

    #[test]
    fn parses_the_release_manifest() {
        let json = r#"{"version":"0.2.0","tag":"v0.2.0","notes":"https://n","assets":[
            {"os":"macos","arch":"aarch64","kind":"dmg","name":"modev-macos-aarch64.dmg",
             "url":"https://u","sha256":"abc","size":1}]}"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.assets.len(), 1);
        let u = update_from(m, "0.1.0", "macos", "aarch64", Some("dmg"));
        assert_eq!(url(u).as_deref(), Some("https://u"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn replaces_the_binary_from_the_tarball() {
        let root = std::env::temp_dir().join(format!("modev-test-{}", std::process::id()));
        let src = root.join("src/modev-linux-x86_64/bin");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("modev"), "new").unwrap();
        let tarball = root.join("modev.tar.gz");
        run(Command::new("tar")
            .arg("-czf")
            .arg(&tarball)
            .arg("-C")
            .arg(root.join("src"))
            .arg("modev-linux-x86_64"))
        .unwrap();
        let exe = root.join("installed/modev");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "old").unwrap();
        install_tarball(&tarball, &root.join("work"), &exe).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finds_the_app_bundle() {
        let exe = Path::new("/Applications/modev.app/Contents/MacOS/modev");
        assert_eq!(
            app_bundle(exe),
            Some(PathBuf::from("/Applications/modev.app"))
        );
        assert_eq!(app_bundle(Path::new("/usr/bin/modev")), None);
    }
}
