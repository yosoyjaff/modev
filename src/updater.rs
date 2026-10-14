//! Update check: reads the `latest.json` manifest that the release workflow
//! (`.github/workflows/release.yml`) attaches to every GitHub release and compares
//! its version with this build. It only informs: downloading is left to the browser.

use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::Deserialize;

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
}

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub version: String,
    /// Release page (notes and every package).
    pub notes: String,
    /// Package for this OS/arch, if the release has one.
    pub download: Option<String>,
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
    ))
}

fn update_from(m: Manifest, current: &str, os: &str, arch: &str) -> Option<Update> {
    if !is_newer(&m.version, current) {
        return None;
    }
    // Linux has two package kinds and no way to know how modev was installed:
    // the release page lets the user choose.
    let kind = match os {
        "macos" => "dmg",
        "windows" => "msi",
        _ => "",
    };
    let download = m
        .assets
        .into_iter()
        .find(|a| a.os == os && a.arch == arch && a.kind == kind)
        .map(|a| a.url);
    Some(Update {
        version: m.version,
        notes: m.notes,
        download,
    })
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
    fn picks_the_package_for_this_platform() {
        let u = update_from(manifest("9.0.0"), "0.1.0", "macos", "aarch64").unwrap();
        assert_eq!(u.version, "9.0.0");
        assert_eq!(u.download.as_deref(), Some("https://dl/macos-aarch64.dmg"));
        let u = update_from(manifest("9.0.0"), "0.1.0", "windows", "x86_64").unwrap();
        assert_eq!(u.download.as_deref(), Some("https://dl/windows-x86_64.msi"));
        // No package for this arch, or Linux: only the release page.
        assert_eq!(
            update_from(manifest("9.0.0"), "0.1.0", "windows", "aarch64")
                .unwrap()
                .download,
            None
        );
        assert_eq!(
            update_from(manifest("9.0.0"), "0.1.0", "linux", "x86_64")
                .unwrap()
                .download,
            None
        );
    }

    #[test]
    fn up_to_date_is_none() {
        assert_eq!(
            update_from(manifest("0.1.0"), "0.1.0", "macos", "aarch64"),
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
        assert_eq!(
            update_from(m, "0.1.0", "macos", "aarch64")
                .unwrap()
                .download
                .as_deref(),
            Some("https://u")
        );
    }
}
