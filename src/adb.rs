//! Android devices over adb, like chrome://inspect/#devices: keeps the adb server running
//! (no need to open Android Studio first), tracks connected devices and lists the DevTools
//! targets of each one (Chrome tabs and debuggable WebViews).
//!
//! It talks to the adb server with its socket protocol (127.0.0.1:5037); the `adb` binary
//! is only used to start the server.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc::UnboundedSender, watch};

const SERVER: &str = "127.0.0.1:5037";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Problem with adb itself (not installed, the server won't start…); empty when it works.
    pub error: String,
    pub devices: Vec<Device>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub serial: String,
    pub model: String,
    /// "device" (ready), "unauthorized", "offline", "no permissions"…
    pub state: String,
    pub browsers: Vec<RemoteBrowser>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteBrowser {
    /// Package: com.android.chrome, or the app that owns the WebView.
    pub name: String,
    /// "Chrome/120.0…" or "WebView/…".
    pub version: String,
    /// Browser-level DevTools websocket through the adb forward.
    pub ws: String,
    pub targets: Vec<Target>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub id: String,
    pub title: String,
    pub url: String,
    /// WebViews report whether they are on screen; Chrome tabs are always `true`.
    pub visible: bool,
}

/// `active`: list the targets (every 2 s, it runs adb shell); otherwise only the devices
/// are tracked, which costs nothing.
pub fn spawn(tx: UnboundedSender<Snapshot>, active: watch::Receiver<bool>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(run(tx, active));
    });
}

type DeviceList = std::result::Result<Vec<(String, String, String)>, String>;

async fn run(tx: UnboundedSender<Snapshot>, mut active: watch::Receiver<bool>) {
    let (dev_tx, mut devices) = watch::channel::<DeviceList>(Ok(vec![]));
    tokio::spawn(track(dev_tx));
    // (serial, socket) → local port of its `adb forward`.
    let mut forwards: HashMap<(String, String), u16> = HashMap::new();
    let mut last: Option<Snapshot> = None;
    loop {
        let list = devices.borrow_and_update().clone();
        let listing = *active.borrow_and_update();
        let snap = match list {
            Err(error) => Snapshot {
                error,
                devices: vec![],
            },
            Ok(list) => {
                let mut out = vec![];
                let mut seen = HashSet::new();
                for (serial, model, state) in list {
                    let browsers = if listing && state == "device" {
                        browsers(&serial, &mut forwards, &mut seen)
                            .await
                            .unwrap_or_else(|e| {
                                eprintln!("[modev] adb {serial}: {e:#}");
                                vec![]
                            })
                    } else {
                        vec![]
                    };
                    out.push(Device {
                        serial,
                        model,
                        state,
                        browsers,
                    });
                }
                // Forwards of sockets that are gone (closed app, unplugged device). Only
                // while listing: otherwise `seen` is empty and the one being inspected would go.
                let stale: Vec<_> = forwards
                    .keys()
                    .filter(|k| listing && !seen.contains(*k))
                    .cloned()
                    .collect();
                for key in stale {
                    if let Some(port) = forwards.remove(&key) {
                        let _ =
                            request(&format!("host-serial:{}:killforward:tcp:{port}", key.0)).await;
                    }
                }
                Snapshot {
                    error: String::new(),
                    devices: out,
                }
            }
        };
        if last.as_ref() != Some(&snap) {
            if tx.send(snap.clone()).is_err() {
                break;
            }
            last = Some(snap);
        }
        tokio::select! {
            r = devices.changed() => if r.is_err() { break },
            r = active.changed() => if r.is_err() { break },
            _ = tokio::time::sleep(Duration::from_secs(2)), if listing => {}
        }
    }
}

/// `host:track-devices-l`: the server pushes the full list on every change. If the server
/// is not running (or dies) it is started again.
async fn track(out: watch::Sender<DeviceList>) {
    loop {
        let res = async {
            let mut s = match connect().await {
                Ok(s) => s,
                Err(_) => {
                    start_server().await?;
                    connect().await?
                }
            };
            send(&mut s, "host:track-devices-l").await?;
            let _ = out.send_replace(Ok(vec![]));
            loop {
                let list = read_msg(&mut s).await?;
                let _ = out.send_replace(Ok(parse_devices(&list)));
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(e) = res {
            if out.is_closed() {
                return;
            }
            // A dropped track connection is normal when the server restarts.
            let e = format!("{e:#}");
            if e.contains("adb not found") || e.contains("adb server") {
                let _ = out.send_replace(Err(e));
            } else {
                let _ = out.send_replace(Ok(vec![]));
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
}

/// Lines `SERIAL  STATE  product:… model:Pixel_7 device:… transport_id:…`.
fn parse_devices(list: &str) -> Vec<(String, String, String)> {
    list.lines()
        .filter_map(|l| {
            let (serial, rest) = l.split_once(char::is_whitespace)?;
            let rest = rest.trim_start();
            // "no permissions (…)" has spaces: the state ends where the key:value pairs start.
            let state_end = rest
                .split_whitespace()
                .find(|w| w.contains(':') && !w.starts_with('('))
                .and_then(|w| rest.find(w))
                .unwrap_or(rest.len());
            let state = rest[..state_end].trim().to_string();
            let model = rest
                .split_whitespace()
                .find_map(|w| w.strip_prefix("model:"))
                .map(|m| m.replace('_', " "))
                .unwrap_or_else(|| serial.to_string());
            Some((serial.to_string(), model, state))
        })
        .collect()
}

/// Looks for adb where Android Studio installs it if it is not in the PATH.
fn adb_path() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut sdks: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
    if let Some(h) = &home {
        sdks.push(h.join("Android/Sdk"));
        sdks.push(h.join("Library/Android/sdk"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        sdks.push(PathBuf::from(local).join("Android/Sdk"));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|d| d.join(exe))
        .chain(sdks.iter().map(|s| s.join("platform-tools").join(exe)))
        .find(|p| p.is_file())
}

async fn start_server() -> Result<()> {
    let adb =
        adb_path().context("adb not found: install Android platform-tools or set ANDROID_HOME")?;
    // The daemon outlives the command: no pipes, or waiting for them would never end.
    let status = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(&adb)
            .arg("start-server")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status(),
    )
    .await
    .context("adb server: start-server timed out")?
    .context("adb server: could not run adb")?;
    if !status.success() {
        bail!("adb server: start-server failed ({status})");
    }
    Ok(())
}

async fn connect() -> Result<TcpStream> {
    Ok(TcpStream::connect(SERVER).await?)
}

/// Sends a request and waits for OKAY (FAIL carries the error message).
async fn send(s: &mut TcpStream, req: &str) -> Result<()> {
    s.write_all(format!("{:04x}{req}", req.len()).as_bytes())
        .await?;
    let mut status = [0u8; 4];
    s.read_exact(&mut status).await?;
    match &status {
        b"OKAY" => Ok(()),
        b"FAIL" => bail!("{}", read_msg(s).await.unwrap_or_default()),
        _ => bail!(
            "unexpected adb reply {:?}",
            String::from_utf8_lossy(&status)
        ),
    }
}

/// Length-prefixed (4 hex digits) message.
async fn read_msg(s: &mut TcpStream) -> Result<String> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await?;
    let len = usize::from_str_radix(std::str::from_utf8(&len)?, 16)?;
    let mut buf = vec![0u8; len];
    s.read_exact(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// One-shot request; returns whatever the server sent after OKAY.
async fn request(req: &str) -> Result<String> {
    let mut s = connect().await?;
    send(&mut s, req).await?;
    let mut rest = vec![];
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest)).await??;
    Ok(String::from_utf8_lossy(&rest).into_owned())
}

async fn shell(serial: &str, cmd: &str) -> Result<String> {
    let mut s = connect().await?;
    send(&mut s, &format!("host:transport:{serial}")).await?;
    send(&mut s, &format!("shell:{cmd}")).await?;
    let mut out = vec![];
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await??;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// `adb forward tcp:0 localabstract:SOCKET`; returns the local port.
async fn forward(serial: &str, socket: &str) -> Result<u16> {
    let reply = request(&format!(
        "host-serial:{serial}:forward:tcp:0;localabstract:{socket}"
    ))
    .await?;
    // Newer servers send a second OKAY and then the port, length-prefixed.
    let reply = reply.strip_prefix("OKAY").unwrap_or(&reply);
    let port = reply.get(4..).unwrap_or(reply).trim();
    port.parse()
        .with_context(|| format!("adb forward: unexpected reply {reply:?}"))
}

/// DevTools sockets of the device (`@chrome_devtools_remote`, `@webview_devtools_remote_<pid>`…)
/// with their targets.
async fn browsers(
    serial: &str,
    forwards: &mut HashMap<(String, String), u16>,
    seen: &mut HashSet<(String, String)>,
) -> Result<Vec<RemoteBrowser>> {
    let unix = shell(serial, "cat /proc/net/unix").await?;
    let mut sockets: Vec<&str> = unix
        .lines()
        .filter_map(|l| l.split_whitespace().last()?.strip_prefix('@'))
        .filter(|s| s.contains("devtools_remote"))
        .collect();
    sockets.sort_unstable();
    sockets.dedup();

    let mut out = vec![];
    for socket in sockets {
        let key = (serial.to_string(), socket.to_string());
        let port = match forwards.get(&key) {
            Some(p) => *p,
            None => match forward(serial, socket).await {
                Ok(p) => {
                    forwards.insert(key.clone(), p);
                    p
                }
                Err(e) => {
                    eprintln!("[modev] adb forward {socket}: {e:#}");
                    continue;
                }
            },
        };
        seen.insert(key);
        let Ok(version) = http_json(port, "/json/version").await else {
            continue;
        };
        let Ok(list) = http_json(port, "/json/list").await else {
            continue;
        };
        let name = match version["Android-Package"].as_str() {
            Some(p) => p.to_string(),
            None => match socket.strip_prefix("webview_devtools_remote_") {
                Some(pid) => shell(serial, &format!("cat /proc/{pid}/cmdline"))
                    .await
                    .map(|c| c.trim_end_matches('\0').trim().to_string())
                    .ok()
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| socket.to_string()),
                None => socket.to_string(),
            },
        };
        // The URL names the host as Chrome saw it in the request: use the forward instead.
        let ws = version["webSocketDebuggerUrl"]
            .as_str()
            .and_then(|u| u.find("/devtools/").map(|i| &u[i..]))
            .unwrap_or("/devtools/browser")
            .to_string();
        let targets = list
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|t| t["type"] == "page")
                    .map(|t| {
                        let desc: serde_json::Value = t["description"]
                            .as_str()
                            .and_then(|d| serde_json::from_str(d).ok())
                            .unwrap_or_default();
                        Target {
                            id: t["id"].as_str().unwrap_or_default().to_string(),
                            title: t["title"].as_str().unwrap_or_default().to_string(),
                            url: t["url"].as_str().unwrap_or_default().to_string(),
                            visible: desc["visible"].as_bool().unwrap_or(true),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(RemoteBrowser {
            name,
            version: version["Browser"].as_str().unwrap_or_default().to_string(),
            ws: format!("ws://127.0.0.1:{port}{ws}"),
            targets,
        });
    }
    Ok(out)
}

/// Minimal HTTP GET against the DevTools endpoint. Android's closes HTTP/1.0 requests
/// without answering, and with 1.1 keeps the connection open (ignores `Connection: close`):
/// the body is read up to its Content-Length.
async fn http_json(port: u16, path: &str) -> Result<serde_json::Value> {
    let fut = async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await?;
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await?;
        let mut buf = vec![];
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let n = s.read(&mut chunk).await?;
            if n == 0 {
                bail!("connection closed before the response ended");
            }
            buf.extend_from_slice(&chunk[..n]);
            let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .context("response without Content-Length")?;
            if buf.len() >= end + 4 + len {
                return Ok(serde_json::from_slice(&buf[end + 4..end + 4 + len])?);
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), fut).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices() {
        let list = "emulator-5554          device product:sdk_phone model:Pixel_7 device:generic transport_id:1\n\
                    R58M123                unauthorized usb:1-1 transport_id:2\n\
                    ZY22                   no permissions (missing udev rules?); see [http://developer.android.com/tools/device.html] usb:1-2 transport_id:3\n";
        let d = parse_devices(list);
        assert_eq!(
            d[0],
            ("emulator-5554".into(), "Pixel 7".into(), "device".into())
        );
        assert_eq!(d[1].2, "unauthorized");
        assert!(d[2].2.starts_with("no permissions"));
    }
}
