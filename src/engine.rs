//! Headless Chromium driven by CDP. It runs in its own tokio runtime and
//! communicates with the (GPUI) UI through channels.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use chromiumoxide::cdp::browser_protocol::emulation::{
    SafeAreaInsets, ScreenOrientation, ScreenOrientationType, SetDeviceMetricsOverrideParams,
    SetSafeAreaInsetsOverrideParams, SetTouchEmulationEnabledParams, SetUserAgentOverrideParams,
};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    DispatchTouchEventType, InsertTextParams, TouchPoint,
};
use chromiumoxide::cdp::browser_protocol::io::{CloseParams as IoClose, ReadParams as IoRead};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams as NetworkEnable, EventDataReceived, EventLoadingFailed, EventLoadingFinished,
    EventRequestWillBeSent, EventRequestWillBeSentExtraInfo, EventResponseReceived,
    EventResponseReceivedExtraInfo, GetResponseBodyParams, Headers, LoadNetworkResourceOptions,
    LoadNetworkResourceParams, RequestId, SetCacheDisabledParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    EventFrameNavigated, EventLoadEventFired, EventNavigatedWithinDocument, EventScreencastFrame,
    GetFrameTreeParams, NavigateParams, ReloadParams, ScreencastFrameAckParams,
    StartScreencastFormat, StartScreencastParams, StopScreencastParams,
};
use chromiumoxide::cdp::js_protocol::runtime::{
    EnableParams as RuntimeEnable, EvaluateParams, EventConsoleApiCalled, EventExceptionThrown,
    RemoteObject,
};
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use gpui_kit::{Image, ImageFormat, RenderImage};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::device::DeviceProfile;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Throttle {
    None,
    Fast4G,
    Slow4G,
    Slow3G,
    Offline,
}

impl Throttle {
    pub const ALL: [Throttle; 5] = [
        Throttle::None,
        Throttle::Fast4G,
        Throttle::Slow4G,
        Throttle::Slow3G,
        Throttle::Offline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Throttle::None => "No throttling",
            Throttle::Fast4G => "Fast 4G",
            Throttle::Slow4G => "Slow 4G",
            Throttle::Slow3G => "Slow 3G",
            Throttle::Offline => "Offline",
        }
    }

    /// (offline, latency ms, download bytes/s, upload bytes/s). Chrome DevTools presets.
    fn params(self) -> (bool, f64, f64, f64) {
        let mbps = |m: f64| m * 1_000_000.0 / 8.0;
        match self {
            Throttle::None => (false, 0.0, -1.0, -1.0),
            Throttle::Fast4G => (false, 60.0 * 2.75, mbps(9.0) * 0.9, mbps(1.5) * 0.9),
            Throttle::Slow4G => (false, 150.0 * 3.75, mbps(1.6) * 0.9, mbps(0.75) * 0.9),
            Throttle::Slow3G => (false, 400.0 * 5.0, mbps(0.4) * 0.9, mbps(0.4) * 0.9),
            Throttle::Offline => (true, 0.0, 0.0, 0.0),
        }
    }
}

pub enum Command {
    Touch(DispatchTouchEventType, f64, f64),
    Wheel(f64, f64, f64, f64),
    InsertText(String),
    Key {
        key: String,
        code: String,
        vk: i64,
        text: Option<String>,
    },
    Navigate(String),
    Reload,
    /// Changes the device and reloads the page.
    SetDevice(DeviceProfile),
    /// Applies size/DPR/safe areas without reloading (responsive mode).
    Resize(DeviceProfile),
    Throttle(Throttle),
    FetchBody(String),
    /// Replays a (possibly edited) request from the page, with its origin and cookies.
    Replay {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<String>,
    },
    Evaluate(String),
    SetCacheDisabled(bool),
    /// Capture scale: screen pixels per CSS pixel at which the web is displayed
    /// (zoom × window scale). Capturing more than that only adds work.
    CaptureScale(f64),
    /// The web's `prefers-color-scheme`: "light", "dark" or "" (the system's).
    ColorScheme(&'static str),
    /// PNG screenshot at device resolution.
    Screenshot(std::path::PathBuf),
    /// Records the screencast to MP4 with ffmpeg; `touches`: draw the touches on the video.
    StartRecording {
        path: std::path::PathBuf,
        touches: bool,
    },
    /// Pointer position over the web (CSS px) for recordings; `None` on leave.
    Pointer(Option<(f64, f64)>),
    StopRecording,
    /// Application panel: read/edit of the origin's storage.
    Storage {
        origin: String,
        req: Box<crate::storage::StorageReq>,
    },
    /// Inspects a page of an Android device (`ws`: its browser endpoint through the adb
    /// forward): the screen and the inspector follow it instead of the local Chrome.
    Attach {
        ws: String,
        target: String,
    },
    /// Back to the local Chrome.
    Detach,
    /// The remote session `u64` stopped answering (sent by the engine itself).
    RemoteClosed(u64),
}

#[derive(Clone, Debug)]
pub struct NetEntry {
    pub id: String,
    pub method: String,
    pub url: String,
    pub resource_type: String,
    pub initiator: String,
    /// Document navigation request (start of a new page).
    pub is_navigation: bool,
    pub request_headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    pub status: Option<i64>,
    pub status_text: String,
    pub mime: String,
    pub response_headers: Vec<(String, String)>,
    pub remote_addr: Option<String>,
    pub protocol: Option<String>,
    pub wall_time: f64,
    pub start_ts: f64,
    pub end_ts: Option<f64>,
    /// Transferred bytes (compressed, with headers).
    pub size: f64,
    /// Uncompressed body size (Firefox's "Size" column).
    pub resource_size: f64,
    /// "VeryHigh", "High", "Medium"…
    pub priority: String,
    /// Cookies sent and Set-Cookie headers received (they arrive in the ExtraInfo events).
    pub cookies: Vec<(String, String)>,
    pub set_cookies: Vec<String>,
    pub error: Option<String>,
    pub body: Option<String>,
    /// Preview if the response is an image.
    pub image: Option<Arc<Image>>,
    /// Rendered pages if the response is a PDF (`Some(vec![])` if it failed).
    pub pdf: Option<Arc<Vec<PdfPage>>>,
}

/// A PDF page rasterized to PNG, with its size in pixels.
#[derive(Clone, Debug)]
pub struct PdfPage {
    pub image: Arc<Image>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LogLevel {
    /// Expression typed into the console.
    Command,
    /// Result of evaluating an expression.
    Result,
    Log,
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level: LogLevel,
    pub text: String,
    pub source: Option<String>,
    pub wall_time: f64,
}

pub enum Event {
    Frame(Arc<RenderImage>),
    /// Main frame navigation (to clear Network/Console).
    Navigated(String),
    /// URL change without loading a document (`pushState`: SPA routes, Ionic tabs).
    UrlChanged(String),
    NetStarted(Box<NetEntry>),
    NetResponse {
        id: String,
        status: i64,
        status_text: String,
        mime: String,
        headers: Vec<(String, String)>,
        remote_addr: Option<String>,
        protocol: Option<String>,
    },
    NetFinished {
        id: String,
        ts: f64,
        size: f64,
    },
    NetFailed {
        id: String,
        ts: f64,
        error: String,
    },
    /// Real request headers (includes Cookie) and sent cookies. May arrive before NetStarted.
    NetRequestExtra {
        id: String,
        headers: Vec<(String, String)>,
        cookies: Vec<(String, String)>,
    },
    /// Real response headers (includes Set-Cookie). May arrive before NetResponse.
    NetResponseExtra {
        id: String,
        headers: Vec<(String, String)>,
    },
    NetData {
        id: String,
        len: f64,
    },
    /// `bytes`: the original binary body (images, fonts…), if it was binary.
    NetBody {
        id: String,
        body: String,
        bytes: Option<Vec<u8>>,
    },
    Log(LogEntry),
    Status(String),
    Screenshot {
        path: std::path::PathBuf,
        png: Vec<u8>,
    },
    /// Recording finished (video path or error).
    Recorded(std::result::Result<std::path::PathBuf, String>),
    Storage(crate::storage::StorageData),
    StorageError(String),
    /// Local/Session Storage changed from the page.
    DomStorageChanged {
        local: bool,
    },
    /// Title and favicon of the page (when loading finishes).
    PageInfo {
        title: String,
        icon: Option<Arc<Image>>,
    },
    /// Inspecting an Android page: its viewport (CSS px) and DPR.
    Attached {
        width: u32,
        height: u32,
        dpr: f64,
        url: String,
    },
    /// Back to the local Chrome; `Some`: why the remote session ended.
    Detached(Option<String>),
    /// Viewport of the screencast (CSS px); on a real device it changes when rotating.
    Viewport(u32, u32),
    /// No Chrome, Chromium or Edge found: the engine did not start.
    ChromeMissing,
}

pub fn spawn(
    device: DeviceProfile,
    url: String,
    events: UnboundedSender<Event>,
    commands: UnboundedReceiver<Command>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            if let Err(err) = run(device, url, events.clone(), commands).await {
                let _ = events.send(Event::Status(format!("Engine error: {err:#}")));
            }
        });
    });
}

async fn run(
    mut device: DeviceProfile,
    url: String,
    events: UnboundedSender<Event>,
    mut commands: UnboundedReceiver<Command>,
) -> Result<()> {
    let config = BrowserConfig::builder()
        .new_headless_mode()
        .viewport(None)
        // In headless the default window is 800x600 and crops the emulated viewport.
        .window_size(1600, 2000)
        // chromiumoxide prepends "--" to each arg: they go without the prefix.
        .arg("hide-scrollbars")
        // Headless fixes a virtual 800x600 screen that crops the emulated viewport
        // (e.g. 874 px tall). The virtual screen is made larger.
        .arg(("screen-info", "{1600x2000}"))
        .arg(("ozone-override-screen-size", "1600,2000"))
        // The screencast captures at the virtual screen's scale (1x), not the emulated
        // DPR: without this the frames arrive at CSS size and look blurry.
        // The screencast's max_width/max_height shrink them to each device's DPR.
        .arg(("force-device-scale-factor", "3"))
        .build();
    // `build` only fails when it cannot find a Chrome/Chromium/Edge executable.
    let Ok(config) = config else {
        let _ = events.send(Event::ChromeMissing);
        return Ok(());
    };
    let (browser, mut handler) = Browser::launch(config).await.context("launching Chrome")?;
    tokio::spawn(async move { while handler.next().await.is_some() {} });

    let local = browser.new_page("about:blank").await?;
    local.execute(NetworkEnable::default()).await?;
    local.execute(RuntimeEnable::default()).await?;
    // Page receiving the commands: the local one, or the Android one being inspected.
    let mut page = local.clone();
    let mut remote: Option<Remote> = None;
    let mut session = 0u64;
    // Commands the engine sends itself (remote session lost).
    let (self_tx, mut self_rx) = tokio::sync::mpsc::unbounded_channel();

    // Latest screencast frame (base64 PNG): decoded by the UI and also used by the recorder.
    let (latest, _) = tokio::sync::watch::channel::<Option<Arc<str>>>(None);
    spawn_decoder(&events, &latest);
    // The local page keeps running while a device is inspected: its events are muted
    // so they don't mix with the device's in the inspector.
    let muted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (local_tx, mut local_rx) = tokio::sync::mpsc::unbounded_channel();
    {
        let (events, muted) = (events.clone(), muted.clone());
        tokio::spawn(async move {
            while let Some(ev) = local_rx.recv().await {
                if !muted.load(std::sync::atomic::Ordering::Relaxed) && events.send(ev).is_err() {
                    break;
                }
            }
        });
    }
    spawn_frames(&page, &local_tx, &latest).await?;
    spawn_listeners(&page, &local_tx).await?;
    apply_device(&page, &device).await?;
    let mut capture = f64::MAX;
    let mut recorder: Option<tokio::sync::oneshot::Sender<()>> = None;
    let touch_mark: Arc<std::sync::Mutex<TouchMark>> = Default::default();
    start_screencast(&page, &device, capture, false).await?;
    let _ = events.send(Event::Status(format!("Loading {url}")));
    page.execute(NavigateParams::new(url.clone())).await?;

    let mut start_url = url;
    // Dragging the border sends many Resize commands in a row: only the last one is applied.
    let mut pending = std::collections::VecDeque::new();
    loop {
        let cmd = match pending.pop_front() {
            Some(c) => c,
            None => tokio::select! {
                c = commands.recv() => match c {
                    Some(c) => c,
                    None => break,
                },
                Some(c) = self_rx.recv() => c,
            },
        };
        // Chrome takes a while to confirm each input event: if they queue up, scrolling
        // lags. Consecutive ones are merged (wheels summed, only the last touch move kept).
        let cmd = match cmd {
            Command::Resize(mut d) => {
                while let Ok(next) = commands.try_recv() {
                    match next {
                        Command::Resize(n) => d = n,
                        other => pending.push_back(other),
                    }
                }
                Command::Resize(d)
            }
            Command::Wheel(x, y, mut dx, mut dy) if pending.is_empty() => {
                while let Ok(next) = commands.try_recv() {
                    match next {
                        Command::Wheel(_, _, ndx, ndy) => {
                            dx += ndx;
                            dy += ndy;
                        }
                        other => {
                            pending.push_back(other);
                            break;
                        }
                    }
                }
                Command::Wheel(x, y, dx, dy)
            }
            Command::Touch(DispatchTouchEventType::TouchMove, mut x, mut y)
                if pending.is_empty() =>
            {
                while let Ok(next) = commands.try_recv() {
                    match next {
                        Command::Touch(DispatchTouchEventType::TouchMove, nx, ny) => {
                            (x, y) = (nx, ny)
                        }
                        other => {
                            pending.push_back(other);
                            break;
                        }
                    }
                }
                Command::Touch(DispatchTouchEventType::TouchMove, x, y)
            }
            c => c,
        };
        let res: Result<()> = async {
            match cmd {
                Command::Touch(kind, x, y) => {
                    if let Ok(mut m) = touch_mark.lock() {
                        *m = match kind {
                            DispatchTouchEventType::TouchStart | DispatchTouchEventType::TouchMove => {
                                TouchMark { x, y, down: true, released: None, hover: Some((x, y)) }
                            }
                            _ => TouchMark { down: false, released: Some(std::time::Instant::now()), ..*m },
                        };
                    }
                    let (kind, touch_points) = match kind {
                        DispatchTouchEventType::TouchStart => ("touchStart", vec![TouchPoint::new(x, y)]),
                        DispatchTouchEventType::TouchMove => ("touchMove", vec![TouchPoint::new(x, y)]),
                        DispatchTouchEventType::TouchEnd => ("touchEnd", vec![]),
                        DispatchTouchEventType::TouchCancel => ("touchCancel", vec![]),
                    };
                    page.execute(DispatchTouch { kind, touch_points }).await?;
                }
                Command::Wheel(x, y, dx, dy) => {
                    let p = DispatchMouseEventParams::builder()
                        .r#type(DispatchMouseEventType::MouseWheel)
                        .x(x)
                        .y(y)
                        .delta_x(dx)
                        .delta_y(dy)
                        .build()
                        .map_err(anyhow::Error::msg)?;
                    page.execute(p).await?;
                }
                Command::InsertText(text) => {
                    page.execute(InsertTextParams::new(text)).await?;
                }
                Command::Key { key, code, vk, text } => {
                    let down = if text.is_some() {
                        DispatchKeyEventType::KeyDown
                    } else {
                        DispatchKeyEventType::RawKeyDown
                    };
                    for kind in [down, DispatchKeyEventType::KeyUp] {
                        let mut b = DispatchKeyEventParams::builder()
                            .r#type(kind.clone())
                            .key(key.clone())
                            .code(code.clone())
                            .windows_virtual_key_code(vk);
                        if kind != DispatchKeyEventType::KeyUp
                            && let Some(t) = &text
                        {
                            b = b.text(t.clone());
                        }
                        page.execute(b.build().map_err(anyhow::Error::msg)?).await?;
                    }
                }
                Command::Navigate(u) => {
                    if remote.is_none() {
                        start_url = u.clone();
                    }
                    page.execute(NavigateParams::new(u)).await?;
                }
                Command::Reload => {
                    page.execute(ReloadParams::default()).await?;
                }
                // While inspecting a real device, emulation applies when going back.
                Command::SetDevice(d) | Command::Resize(d) if remote.is_some() => {
                    if let Some(r) = &mut remote {
                        r.local_device = d;
                    }
                }
                Command::SetDevice(d) => {
                    device = d;
                    page.execute(StopScreencastParams::default()).await?;
                    apply_device(&page, &device).await?;
                    start_screencast(&page, &device, capture, false).await?;
                    page.execute(NavigateParams::new(start_url.clone())).await?;
                }
                Command::Resize(d) => {
                    let restart = (d.width, d.height, d.dpr) != (device.width, device.height, device.dpr);
                    device = d;
                    apply_device(&page, &device).await?;
                    // With a different size, the screencast needs new maximums.
                    if restart {
                        page.execute(StopScreencastParams::default()).await?;
                        start_screencast(&page, &device, capture, false).await?;
                    }
                }
                // Deprecated in favor of emulateNetworkConditionsByRule, but Chrome still supports it.
                #[allow(deprecated)]
                Command::Throttle(t) => {
                    let (offline, latency, down, up) = t.params();
                    use chromiumoxide::cdp::browser_protocol::network::EmulateNetworkConditionsParams;
                    page.execute(EmulateNetworkConditionsParams::new(offline, latency, down, up))
                        .await?;
                }
                Command::Evaluate(expr) => {
                    let _ = events.send(Event::Log(LogEntry {
                        level: LogLevel::Command,
                        text: expr.clone(),
                        source: None,
                        wall_time: now(),
                    }));
                    let params = EvaluateParams::builder()
                        .expression(expr)
                        .include_command_line_api(true)
                        .repl_mode(true)
                        .generate_preview(true)
                        .user_gesture(true)
                        .await_promise(true)
                        .build()
                        .map_err(anyhow::Error::msg)?;
                    let r = page.execute(params).await?;
                    let (level, text) = match &r.result.exception_details {
                        Some(d) => (
                            LogLevel::Error,
                            d.exception
                                .as_ref()
                                .and_then(|e| e.description.clone())
                                .unwrap_or_else(|| d.text.clone()),
                        ),
                        None => (LogLevel::Result, remote_to_string(&r.result.result)),
                    };
                    let _ = events.send(Event::Log(LogEntry { level, text, source: None, wall_time: now() }));
                }
                Command::Storage { origin, req } => {
                    // In another task: IndexedDB can be slow and must not block input.
                    let (p, tx) = (page.clone(), events.clone());
                    tokio::spawn(async move {
                        let ev = match crate::storage::handle(&p, &origin, *req).await {
                            Ok(data) => Event::Storage(data),
                            Err(e) => Event::StorageError(format!("{e:#}")),
                        };
                        let _ = tx.send(ev);
                    });
                }
                Command::CaptureScale(scale) => {
                    if (scale - capture).abs() > 0.01 {
                        capture = scale;
                        page.execute(StopScreencastParams::default()).await?;
                        start_screencast(&page, &device, capture, remote.is_some()).await?;
                    }
                }
                Command::ColorScheme(value) => {
                    use chromiumoxide::cdp::browser_protocol::emulation::{MediaFeature, SetEmulatedMediaParams};
                    let p = SetEmulatedMediaParams::builder()
                        .feature(MediaFeature::new("prefers-color-scheme", value))
                        .build();
                    page.execute(p).await?;
                }
                Command::Screenshot(path) => {
                    use chromiumoxide::cdp::browser_protocol::page::{CaptureScreenshotFormat, CaptureScreenshotParams};
                    let p = CaptureScreenshotParams::builder().format(CaptureScreenshotFormat::Png).build();
                    let r = page.execute(p).await?;
                    let png = base64::engine::general_purpose::STANDARD.decode(r.result.data.as_ref() as &str)?;
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&path, &png)?;
                    let _ = events.send(Event::Screenshot { path, png });
                }
                Command::StartRecording { path, touches } => {
                    if recorder.is_none() {
                        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
                        recorder = Some(stop_tx);
                        let (frames, tx) = (latest.subscribe(), events.clone());
                        let touches = touches.then(|| (touch_mark.clone(), device.width as f64));
                        tokio::spawn(async move {
                            let res = record(path, frames, stop_rx, touches).await.map_err(|e| format!("{e:#}"));
                            let _ = tx.send(Event::Recorded(res));
                        });
                    }
                }
                Command::Pointer(pos) => {
                    if let Ok(mut m) = touch_mark.lock() {
                        m.hover = pos;
                    }
                }
                Command::StopRecording => {
                    if let Some(stop) = recorder.take() {
                        let _ = stop.send(());
                    }
                }
                Command::SetCacheDisabled(disabled) => {
                    page.execute(SetCacheDisabledParams::new(disabled)).await?;
                }
                Command::FetchBody(id) => {
                    let (body, bytes) = fetch_body(&page, &id).await;
                    let _ = events.send(Event::NetBody { id, body, bytes });
                }
                Command::Replay { method, url, headers, body } => {
                    // HTTP/2 pseudo-headers are not valid names for fetch().
                    let headers: serde_json::Map<String, serde_json::Value> = headers
                        .into_iter()
                        .filter(|(k, _)| !k.starts_with(':'))
                        .map(|(k, v)| (k, v.into()))
                        .collect();
                    let body = body.filter(|_| !matches!(method.as_str(), "GET" | "HEAD"));
                    let init = serde_json::json!({
                        "method": method,
                        "headers": headers,
                        "body": body,
                        "credentials": "include",
                    });
                    let expr = format!(
                        "fetch({}, {init}).catch(e => console.error('[modev] Replay failed:', e))",
                        serde_json::to_string(&url)?
                    );
                    let params = EvaluateParams::builder()
                        .expression(expr)
                        .user_gesture(true)
                        .build()
                        .map_err(anyhow::Error::msg)?;
                    page.execute(params).await?;
                }
                Command::Attach { ws, target } => {
                    session += 1;
                    let (r, rpage, d, url) = match attach(&ws, &target, &device, session, &self_tx).await {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = events.send(Event::Status(format!("Could not inspect the device: {e:#}")));
                            return Ok(());
                        }
                    };
                    let _ = page.execute(StopScreencastParams::default()).await;
                    let local_device = match remote.take() {
                        Some(old) => old.close(),
                        None => device.clone(),
                    };
                    remote = Some(Remote { local_device, ..r });
                    muted.store(true, std::sync::atomic::Ordering::Relaxed);
                    page = rpage;
                    device = d;
                    spawn_frames(&page, &events, &latest).await?;
                    spawn_listeners(&page, &events).await?;
                    // Attached empties Console/Network: it goes before the replay below.
                    let _ = events.send(Event::Attached {
                        width: device.width,
                        height: device.height,
                        dpr: device.dpr,
                        url,
                    });
                    // chromiumoxide enabled Runtime when connecting, before these listeners:
                    // enabling it again makes Chrome replay the page's earlier console
                    // messages and exceptions, like DevTools does when it opens.
                    use chromiumoxide::cdp::js_protocol::runtime::DisableParams as RuntimeDisable;
                    page.execute(RuntimeDisable::default()).await?;
                    page.execute(RuntimeEnable::default()).await?;
                    start_screencast(&page, &device, capture, true).await?;
                    if let Ok(info) = page_info(&page).await {
                        let _ = events.send(info);
                    }
                }
                Command::Detach | Command::RemoteClosed(_) => {
                    let lost = match cmd {
                        Command::RemoteClosed(s) if s != session => return Ok(()),
                        Command::RemoteClosed(_) => Some("The device page is no longer available".into()),
                        _ => None,
                    };
                    if let Some(r) = remote.take() {
                        (page, device) = (local.clone(), r.close());
                        muted.store(false, std::sync::atomic::Ordering::Relaxed);
                        apply_device(&page, &device).await?;
                        start_screencast(&page, &device, capture, false).await?;
                        let _ = events.send(Event::Detached(lost));
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = res {
            let _ = events.send(Event::Status(format!("CDP: {e:#}")));
        }
    }
    Ok(())
}

/// `Input.dispatchTouchEvent` that always serializes `touchPoints`. chromiumoxide's omits
/// it when empty and Chrome rejects a `touchEnd` without it (-32602 Invalid parameters):
/// the touch never ended and there were no clicks.
#[derive(Debug, serde::Serialize)]
struct DispatchTouch {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "touchPoints")]
    touch_points: Vec<TouchPoint>,
}

impl chromiumoxide::Method for DispatchTouch {
    fn identifier(&self) -> chromiumoxide::types::MethodId {
        "Input.dispatchTouchEvent".into()
    }
}

impl chromiumoxide::Command for DispatchTouch {
    type Response = serde_json::Value;
}

async fn apply_device(page: &Page, d: &DeviceProfile) -> Result<()> {
    let metrics = SetDeviceMetricsOverrideParams::builder()
        .width(d.width as i64)
        .height(d.height as i64)
        .device_scale_factor(d.dpr)
        .mobile(true)
        .screen_width(d.width as i64)
        .screen_height(d.height as i64)
        .screen_orientation(ScreenOrientation::new(
            ScreenOrientationType::PortraitPrimary,
            0,
        ))
        .build()
        .map_err(anyhow::Error::msg)?;
    page.execute(metrics).await?;

    let insets = SafeAreaInsets::builder()
        .top(d.safe_top as i64)
        .top_max(d.safe_top as i64)
        .bottom(d.safe_bottom as i64)
        .bottom_max(d.safe_bottom as i64)
        .left(0)
        .left_max(0)
        .right(0)
        .right_max(0)
        .build();
    page.execute(SetSafeAreaInsetsOverrideParams::new(insets))
        .await?;

    let ua = SetUserAgentOverrideParams::builder()
        .user_agent(d.user_agent.clone())
        .platform(d.platform.clone())
        .accept_language("es-ES,es")
        .build()
        .map_err(anyhow::Error::msg)?;
    page.execute(ua).await?;

    page.execute(
        SetTouchEmulationEnabledParams::builder()
            .enabled(true)
            .max_touch_points(5)
            .build()
            .map_err(anyhow::Error::msg)?,
    )
    .await?;
    Ok(())
}

/// `capture`: pixels per CSS pixel being displayed; never more than the device's DPR.
/// The frame (PNG) arrives at the exact on-screen size: Chrome downsamples from the full-DPR
/// render with good filtering, while scaling in GPUI (bilinear, no mipmaps) blurs the text.
/// `remote`: a real device over USB, JPEG (PNG frames are too heavy for adb) and room for
/// both orientations.
async fn start_screencast(
    page: &Page,
    d: &DeviceProfile,
    capture: f64,
    remote: bool,
) -> Result<()> {
    let scale = d.dpr.min(capture).max(0.25);
    let (mut w, mut h) = (d.width as f64 * scale, d.height as f64 * scale);
    if remote {
        (w, h) = (w.max(h), w.max(h));
    }
    let p = StartScreencastParams::builder()
        // PNG instead of JPEG: JPEG stores color at half resolution and leaves halos on
        // text (especially light-on-colored-buttons) at 1×. The frames are small anyway.
        .format(if remote {
            StartScreencastFormat::Jpeg
        } else {
            StartScreencastFormat::Png
        })
        .quality(90)
        .max_width(w.round() as i64)
        .max_height(h.round() as i64)
        .every_nth_frame(1)
        .build();
    page.execute(p).await?;
    Ok(())
}

/// A page of an Android device being inspected.
struct Remote {
    /// Handler of the connection (and the watchdog): aborting it closes the websocket.
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Emulated device to restore when going back to the local Chrome.
    local_device: DeviceProfile,
}

impl Remote {
    /// Returns the device to restore.
    fn close(self) -> DeviceProfile {
        for t in self.tasks {
            t.abort();
        }
        self.local_device
    }
}

/// Connects to the browser of the device and takes the page `target` (without emulation:
/// it is a real device). Returns the session, its page, its viewport as a profile and its URL.
async fn attach(
    ws: &str,
    target: &str,
    current: &DeviceProfile,
    session: u64,
    self_tx: &UnboundedSender<Command>,
) -> Result<(Remote, Page, DeviceProfile, String)> {
    use chromiumoxide::cdp::browser_protocol::target::TargetId;
    use chromiumoxide::handler::HandlerConfig;
    let config = HandlerConfig {
        ignore_https_errors: false,
        ..Default::default()
    };
    let (mut browser, mut handler) = Browser::connect_with_config(ws, config)
        .await
        .context("connecting to the device")?;
    let handler = tokio::spawn(async move { while handler.next().await.is_some() {} });
    let mut tasks = vec![handler];
    let res = async {
        browser.fetch_targets().await?;
        // The targets become pages asynchronously.
        let id = TargetId::from(target.to_string());
        let mut page = None;
        for _ in 0..50 {
            if let Ok(p) = browser.get_page(id.clone()).await {
                page = Some(p);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let page = page.context("the page is no longer open on the device")?;
        page.execute(NetworkEnable::default()).await?;
        page.execute(RuntimeEnable::default()).await?;
        let (w, h, dpr): (f64, f64, f64) = page
            .evaluate("[innerWidth, innerHeight, devicePixelRatio]")
            .await?
            .into_value()?;
        let url = page.url().await?.unwrap_or_default();
        Ok::<_, anyhow::Error>((page, w, h, dpr, url))
    }
    .await;
    let (page, w, h, dpr, url) = match res {
        Ok(v) => v,
        Err(e) => {
            for t in tasks {
                t.abort();
            }
            return Err(e);
        }
    };
    // The handler does not end when the websocket drops (unplugged cable, closed app):
    // a cheap evaluate tells whether the page still answers.
    let (p, tx) = (page.clone(), self_tx.clone());
    tasks.push(tokio::spawn(async move {
        let mut failures = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let alive = tokio::time::timeout(std::time::Duration::from_secs(5), p.evaluate("1"))
                .await
                .is_ok_and(|r| r.is_ok());
            failures = if alive { 0 } else { failures + 1 };
            if failures >= 3 {
                let _ = tx.send(Command::RemoteClosed(session));
                break;
            }
        }
    }));
    // Keeps the browser alive with the tasks (dropping it closes the channel to the handler).
    tasks.push(tokio::spawn(async move {
        let _browser = browser;
        std::future::pending::<()>().await;
    }));
    let d = DeviceProfile {
        name: "Android".into(),
        os_label: "USB".into(),
        width: w.round() as u32,
        height: h.round() as u32,
        dpr,
        ppi: 0.0,
        safe_top: 0,
        safe_bottom: 0,
        ..current.clone()
    };
    let remote = Remote {
        tasks,
        local_device: current.clone(),
    };
    Ok((remote, page, d, url))
}

/// The only PNG/JPEG -> BGRA decoder: it always takes the latest frame (if decoding falls
/// behind, intermediate frames are dropped and none arrive out of order).
fn spawn_decoder(
    events: &UnboundedSender<Event>,
    latest: &tokio::sync::watch::Sender<Option<Arc<str>>>,
) {
    let tx = events.clone();
    let mut rx = latest.subscribe();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let Some(data) = rx.borrow_and_update().clone() else {
                continue;
            };
            let img = tokio::task::spawn_blocking(move || decode_frame(&data)).await;
            if let Ok(Some(img)) = img
                && tx.send(Event::Frame(img)).is_err()
            {
                break;
            }
        }
    });
}

/// Screencast frames of `page`, acknowledged right away, into `latest`.
async fn spawn_frames(
    page: &Page,
    events: &UnboundedSender<Event>,
    latest: &tokio::sync::watch::Sender<Option<Arc<str>>>,
) -> Result<()> {
    let mut frames = page.event_listener::<EventScreencastFrame>().await?;
    let (tx, p) = (events.clone(), page.clone());
    let latest = latest.clone();
    tokio::spawn(async move {
        let mut last_dims = (0, 0);
        while let Some(ev) = frames.next().await {
            let _ = p
                .execute(ScreencastFrameAckParams::new(ev.session_id))
                .await;
            let m = &ev.metadata;
            let dims = (m.device_width as i64, m.device_height as i64);
            if dims != last_dims {
                last_dims = dims;
                eprintln!(
                    "[modev] screencast: viewport {}x{} css, page_scale {}, offset_top {}",
                    m.device_width, m.device_height, m.page_scale_factor, m.offset_top
                );
                let _ = tx.send(Event::Viewport(dims.0 as u32, dims.1 as u32));
            }
            let data: &str = ev.data.as_ref();
            latest.send_replace(Some(Arc::from(data)));
        }
    });

    Ok(())
}

/// Latest touch (in CSS px) to draw it in recordings.
#[derive(Clone, Copy, Default)]
struct TouchMark {
    x: f64,
    y: f64,
    down: bool,
    released: Option<std::time::Instant>,
    /// Pointer over the web (always drawn, even without a touch).
    hover: Option<(f64, f64)>,
}

/// On release, the circle fades out in this time (a quick tap is still visible).
const TOUCH_FADE: f32 = 0.3;

impl TouchMark {
    /// Indicator opacity right now (0 = not drawn).
    fn alpha(&self) -> f32 {
        match (self.down, self.released) {
            (true, _) => 1.0,
            (false, Some(t)) => (1.0 - t.elapsed().as_secs_f32() / TOUCH_FADE).max(0.0),
            _ => 0.0,
        }
    }
}

/// Draws the pointer (dot) and the touch (big circle, like iOS) over a PNG frame.
fn draw_marks(png: &[u8], css_width: f64, m: TouchMark) -> Option<Vec<u8>> {
    let mut img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .into_rgb8();
    let s = img.width() as f64 / css_width.max(1.0);
    let (w, h) = (img.width() as i64, img.height() as i64);
    let mut circle = |cx: f64, cy: f64, r: f64, alpha: f32| {
        let (cx, cy, r, ring) = (cx * s, cy * s, r * s, 1.5 * s);
        let (x0, x1) = (
            ((cx - r).floor() as i64).max(0),
            ((cx + r).ceil() as i64).min(w - 1),
        );
        let (y0, y1) = (
            ((cy - r).floor() as i64).max(0),
            ((cy + r).ceil() as i64).min(h - 1),
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
                if d > r {
                    continue;
                }
                let (color, a) = if d > r - ring {
                    (60.0, 0.6 * alpha)
                } else {
                    (255.0, 0.5 * alpha)
                };
                let px = img.get_pixel_mut(x as u32, y as u32);
                for c in px.0.iter_mut() {
                    *c = (*c as f32 * (1.0 - a) + color * a).round() as u8;
                }
            }
        }
    };
    let touch = m.alpha();
    if touch > 0.0 {
        circle(m.x, m.y, 20.0, touch);
    }
    if let Some((x, y)) = m.hover.filter(|_| !m.down) {
        circle(x, y, 7.0, 1.0);
    }
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    img.write_with_encoder(encoder).ok()?;
    Some(out)
}

/// Records the latest screencast frame at 30 fps (repeated if the web doesn't change) with ffmpeg.
/// `touches`: latest touch and device CSS width, to draw the touches.
async fn record(
    path: std::path::PathBuf,
    mut frames: tokio::sync::watch::Receiver<Option<Arc<str>>>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
    touches: Option<(Arc<std::sync::Mutex<TouchMark>>, f64)>,
) -> Result<std::path::PathBuf> {
    use tokio::io::AsyncWriteExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut child = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "-framerate",
            "30",
            "-i",
            "-",
        ])
        // H.264 requires even dimensions.
        .args([
            "-vf",
            "scale=trunc(iw/2)*2:trunc(ih/2)*2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
        ])
        .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
        .arg(&path)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to run ffmpeg (is it installed?)")?;
    let mut stdin = child.stdin.take().context("ffmpeg has no stdin")?;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(1000 / 30));
    let mut png = Vec::new();
    // Generation of the current frame and latest composition with marks (to avoid re-encoding).
    let mut frame_no = 0u64;
    type ComposeKey = (u64, Option<(u64, u64)>, bool, u32);
    let mut composed: Option<(ComposeKey, Vec<u8>)> = None;
    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = tick.tick() => {}
        }
        if frames.has_changed().unwrap_or(false)
            && let Some(data) = frames.borrow_and_update().clone()
        {
            png = base64::engine::general_purpose::STANDARD.decode(data.as_bytes())?;
            frame_no += 1;
        }
        if png.is_empty() {
            continue;
        }
        let mark = touches
            .as_ref()
            .and_then(|(m, w)| Some((*m.lock().ok()?, *w)));
        let frame = match mark {
            Some((m, css_width)) if m.alpha() > 0.0 || m.hover.is_some() => {
                // Same frame and same still pointer: reuse the previous composition.
                let key = (
                    frame_no,
                    m.hover.map(|(x, y)| (x.to_bits(), y.to_bits())),
                    m.down,
                    m.alpha().to_bits(),
                );
                match &composed {
                    Some((k, bytes)) if *k == key => Some(bytes.clone()),
                    _ => {
                        let src = png.clone();
                        let out =
                            tokio::task::spawn_blocking(move || draw_marks(&src, css_width, m))
                                .await
                                .ok()
                                .flatten();
                        composed = out.clone().map(|b| (key, b));
                        out
                    }
                }
            }
            _ => None,
        };
        if stdin
            .write_all(frame.as_ref().unwrap_or(&png))
            .await
            .is_err()
        {
            break;
        }
    }
    drop(stdin);
    let out = child.wait_with_output().await?;
    if !out.status.success() {
        anyhow::bail!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(path)
}

/// Local/Session Storage changes made by the page (the Application panel refreshes itself).
async fn spawn_dom_storage(page: &Page, events: &UnboundedSender<Event>) -> Result<()> {
    use chromiumoxide::cdp::browser_protocol::dom_storage::{
        EnableParams as DomStorageEnable, EventDomStorageItemAdded, EventDomStorageItemRemoved,
        EventDomStorageItemUpdated, EventDomStorageItemsCleared,
    };
    page.execute(DomStorageEnable::default()).await?;
    let mut added = page.event_listener::<EventDomStorageItemAdded>().await?;
    let mut removed = page.event_listener::<EventDomStorageItemRemoved>().await?;
    let mut updated = page.event_listener::<EventDomStorageItemUpdated>().await?;
    let mut cleared = page.event_listener::<EventDomStorageItemsCleared>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        loop {
            let local = tokio::select! {
                Some(e) = added.next() => e.storage_id.is_local_storage,
                Some(e) = removed.next() => e.storage_id.is_local_storage,
                Some(e) = updated.next() => e.storage_id.is_local_storage,
                Some(e) = cleared.next() => e.storage_id.is_local_storage,
                else => break,
            };
            if tx.send(Event::DomStorageChanged { local }).is_err() {
                break;
            }
        }
    });
    Ok(())
}

async fn spawn_listeners(page: &Page, events: &UnboundedSender<Event>) -> Result<()> {
    spawn_dom_storage(page, events).await?;

    let mut sent = page.event_listener::<EventRequestWillBeSent>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = sent.next().await {
            if ev.request.url.starts_with("data:") {
                continue;
            }
            let id = ev.request_id.inner().clone();
            // Redirect: Chrome reuses the id; the previous entry is closed with its 3xx.
            if let Some(r) = &ev.redirect_response {
                let _ = tx.send(Event::NetResponse {
                    id: id.clone(),
                    status: r.status,
                    status_text: r.status_text.clone(),
                    mime: r.mime_type.clone(),
                    headers: headers(&r.headers),
                    remote_addr: None,
                    protocol: r.protocol.clone(),
                });
                let _ = tx.send(Event::NetFinished {
                    id: id.clone(),
                    ts: *ev.timestamp.inner(),
                    size: r.encoded_data_length,
                });
            }
            let is_document = matches!(
                ev.r#type,
                Some(chromiumoxide::cdp::browser_protocol::network::ResourceType::Document)
            );
            let _ = tx.send(Event::NetStarted(Box::new(NetEntry {
                is_navigation: is_document && ev.request_id.inner() == ev.loader_id.inner(),
                id,
                method: ev.request.method.clone(),
                url: ev.request.url.clone(),
                resource_type: ev
                    .r#type
                    .as_ref()
                    .map(|t| format!("{t:?}"))
                    .unwrap_or_default(),
                initiator: initiator_label(&ev.initiator),
                request_headers: headers(&ev.request.headers),
                post_data: ev.request.post_data_entries.as_ref().map(|e| {
                    e.iter()
                        .filter_map(|p| p.bytes.as_ref())
                        .filter_map(|b| {
                            base64::engine::general_purpose::STANDARD
                                .decode(b.as_ref() as &str)
                                .ok()
                        })
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .collect::<String>()
                }),
                status: None,
                status_text: String::new(),
                mime: String::new(),
                response_headers: vec![],
                remote_addr: None,
                protocol: None,
                wall_time: *ev.wall_time.inner(),
                start_ts: *ev.timestamp.inner(),
                end_ts: None,
                size: 0.0,
                resource_size: 0.0,
                priority: format!("{:?}", ev.request.initial_priority),
                cookies: vec![],
                set_cookies: vec![],
                error: None,
                body: None,
                image: None,
                pdf: None,
            })));
        }
    });

    let mut extra = page
        .event_listener::<EventRequestWillBeSentExtraInfo>()
        .await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = extra.next().await {
            let cookies = ev
                .associated_cookies
                .iter()
                .filter(|c| c.blocked_reasons.is_empty())
                .map(|c| (c.cookie.name.clone(), c.cookie.value.clone()))
                .collect();
            let _ = tx.send(Event::NetRequestExtra {
                id: ev.request_id.inner().clone(),
                headers: headers(&ev.headers),
                cookies,
            });
        }
    });

    let mut extra = page
        .event_listener::<EventResponseReceivedExtraInfo>()
        .await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = extra.next().await {
            let _ = tx.send(Event::NetResponseExtra {
                id: ev.request_id.inner().clone(),
                headers: headers(&ev.headers),
            });
        }
    });

    let mut data = page.event_listener::<EventDataReceived>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = data.next().await {
            let _ = tx.send(Event::NetData {
                id: ev.request_id.inner().clone(),
                len: ev.data_length as f64,
            });
        }
    });

    let mut load = page.event_listener::<EventLoadEventFired>().await?;
    let (tx, p) = (events.clone(), page.clone());
    tokio::spawn(async move {
        while load.next().await.is_some() {
            if let Ok(info) = page_info(&p).await {
                let _ = tx.send(info);
            }
        }
    });

    let mut nav = page.event_listener::<EventFrameNavigated>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = nav.next().await {
            if ev.frame.parent_id.is_none() {
                // A failed load commits `chrome-error://chromewebdata/`: show the URL that failed.
                let url = ev.frame.unreachable_url.as_ref().unwrap_or(&ev.frame.url);
                let _ = tx.send(Event::Navigated(url.clone()));
            }
        }
    });

    let mut within = page
        .event_listener::<EventNavigatedWithinDocument>()
        .await?;
    let (tx, main) = (events.clone(), page.target_id().inner().clone());
    tokio::spawn(async move {
        while let Some(ev) = within.next().await {
            // Chrome's main frame id is the target id.
            if *ev.frame_id.inner() == main {
                let _ = tx.send(Event::UrlChanged(ev.url.clone()));
            }
        }
    });

    let mut resp = page.event_listener::<EventResponseReceived>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = resp.next().await {
            let r = &ev.response;
            let _ = tx.send(Event::NetResponse {
                id: ev.request_id.inner().clone(),
                status: r.status,
                status_text: r.status_text.clone(),
                mime: r.mime_type.clone(),
                headers: headers(&r.headers),
                remote_addr: r
                    .remote_ip_address
                    .as_ref()
                    .map(|ip| format!("{ip}:{}", r.remote_port.unwrap_or(0))),
                protocol: r.protocol.clone(),
            });
        }
    });

    let mut fin = page.event_listener::<EventLoadingFinished>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = fin.next().await {
            let _ = tx.send(Event::NetFinished {
                id: ev.request_id.inner().clone(),
                ts: *ev.timestamp.inner(),
                size: ev.encoded_data_length,
            });
        }
    });

    let mut failed = page.event_listener::<EventLoadingFailed>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = failed.next().await {
            let _ = tx.send(Event::NetFailed {
                id: ev.request_id.inner().clone(),
                ts: *ev.timestamp.inner(),
                error: ev.error_text.clone(),
            });
        }
    });

    let mut console = page.event_listener::<EventConsoleApiCalled>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        use chromiumoxide::cdp::js_protocol::runtime::ConsoleApiCalledType as T;
        while let Some(ev) = console.next().await {
            let level = match ev.r#type {
                T::Error | T::Assert => LogLevel::Error,
                T::Warning => LogLevel::Warn,
                T::Info => LogLevel::Info,
                T::Debug => LogLevel::Debug,
                _ => LogLevel::Log,
            };
            let text = ev
                .args
                .iter()
                .map(remote_to_string)
                .collect::<Vec<_>>()
                .join(" ");
            let source = ev
                .stack_trace
                .as_ref()
                .and_then(|s| s.call_frames.first())
                .map(|f| {
                    format!(
                        "{}:{}",
                        f.url.rsplit('/').next().unwrap_or(&f.url),
                        f.line_number + 1
                    )
                });
            let _ = tx.send(Event::Log(LogEntry {
                level,
                text,
                source,
                wall_time: *ev.timestamp.inner() / 1000.0,
            }));
        }
    });

    let mut exc = page.event_listener::<EventExceptionThrown>().await?;
    let tx = events.clone();
    tokio::spawn(async move {
        while let Some(ev) = exc.next().await {
            let d = &ev.exception_details;
            let text = d
                .exception
                .as_ref()
                .and_then(|e| e.description.clone())
                .unwrap_or_else(|| d.text.clone());
            let _ = tx.send(Event::Log(LogEntry {
                level: LogLevel::Error,
                text,
                source: d.url.as_ref().map(|u| {
                    format!(
                        "{}:{}",
                        u.rsplit('/').next().unwrap_or(u),
                        d.line_number + 1
                    )
                }),
                wall_time: *ev.timestamp.inner() / 1000.0,
            }));
        }
    });
    Ok(())
}

async fn fetch_body(page: &Page, id: &str) -> (String, Option<Vec<u8>>) {
    match page
        .execute(GetResponseBodyParams::new(RequestId::new(id.to_string())))
        .await
    {
        Ok(r) if r.result.base64_encoded => {
            match base64::engine::general_purpose::STANDARD.decode(&r.result.body) {
                Ok(b) => (format!("<binary, {} bytes>", b.len()), Some(b)),
                Err(_) => ("<binary>".into(), None),
            }
        }
        Ok(r) => (r.result.body.clone(), None),
        Err(e) => (format!("<no body: {e}>"), None),
    }
}

pub fn is_pdf(mime: &str, url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    mime.to_ascii_lowercase().contains("pdf") || path.ends_with(".pdf")
}

/// Rasterizes the first pages of a PDF (CPU-bound, can take a while: call off the UI thread).
pub fn render_pdf(bytes: Vec<u8>) -> Vec<PdfPage> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings};

    const MAX_PAGES: usize = 50;
    // 2x: sharp on HiDPI screens with the page at ~560 px wide.
    const SCALE: f32 = 2.0;

    let Ok(pdf) = Pdf::new(bytes) else {
        return Vec::new();
    };
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let pixmap = PixmapSettings {
        x_scale: SCALE,
        y_scale: SCALE,
        bg_color: WHITE,
    };
    pdf.pages()
        .iter()
        .take(MAX_PAGES)
        .filter_map(|page| {
            let pm = hayro::render(page, &cache, &settings, &RenderSettings::default(), &pixmap);
            let (width, height) = (pm.width() as u32, pm.height() as u32);
            let png = pm.into_png().ok()?;
            Some(PdfPage {
                image: Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                width,
                height,
            })
        })
        .collect()
}

/// Image format by MIME or extension; `None` if it doesn't look like an image.
pub fn image_format(mime: &str, url: &str) -> Option<ImageFormat> {
    let mime = mime.to_ascii_lowercase();
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let is = |m: &str, exts: &[&str]| mime.contains(m) || exts.iter().any(|e| path.ends_with(e));
    Some(if is("svg", &[".svg"]) {
        ImageFormat::Svg
    } else if is("png", &[".png"]) {
        ImageFormat::Png
    } else if is("jpeg", &[".jpg", ".jpeg"]) {
        ImageFormat::Jpeg
    } else if is("webp", &[".webp"]) {
        ImageFormat::Webp
    } else if is("gif", &[".gif"]) {
        ImageFormat::Gif
    } else if is("bmp", &[".bmp"]) {
        ImageFormat::Bmp
    } else if is("icon", &[".ico"]) {
        ImageFormat::Ico
    } else {
        return None;
    })
}

/// Title and favicon. The favicon is downloaded with `Network.loadNetworkResource`
/// (requested by the browser, not the page): it doesn't pollute the Network panel.
async fn page_info(page: &Page) -> Result<Event> {
    let expr = r#"JSON.stringify([document.title, [...document.querySelectorAll('link[rel~="icon"]')].pop()?.href || new URL('/favicon.ico', location.href).href])"#;
    let params = EvaluateParams::builder()
        .expression(expr)
        .return_by_value(true)
        .build()
        .map_err(anyhow::Error::msg)?;
    let r = page.execute(params).await?;
    let json = r
        .result
        .result
        .value
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let (title, href): (String, String) = serde_json::from_str(json)?;
    let icon = fetch_icon(page, &href).await.ok().flatten();
    Ok(Event::PageInfo { title, icon })
}

async fn fetch_icon(page: &Page, href: &str) -> Result<Option<Arc<Image>>> {
    let tree = page.execute(GetFrameTreeParams::default()).await?;
    let params = LoadNetworkResourceParams::builder()
        .frame_id(tree.result.frame_tree.frame.id.clone())
        .url(href)
        .options(LoadNetworkResourceOptions::new(false, true))
        .build()
        .map_err(anyhow::Error::msg)?;
    let res = page.execute(params).await?.result.resource;
    let Some(stream) = res.stream else {
        return Ok(None);
    };
    let mime = res
        .headers
        .as_ref()
        .and_then(|h| h.inner().as_object())
        .and_then(|h| {
            h.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        })
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut bytes = vec![];
    loop {
        let chunk = page.execute(IoRead::new(stream.clone())).await?.result;
        if chunk.base64_encoded.unwrap_or(false) {
            bytes.extend(base64::engine::general_purpose::STANDARD.decode(&chunk.data)?);
        } else {
            bytes.extend(chunk.data.into_bytes());
        }
        if chunk.eof {
            break;
        }
    }
    let _ = page.execute(IoClose::new(stream)).await;
    if !res.success || bytes.is_empty() {
        return Ok(None);
    }
    // Without hints, a favicon is usually .ico.
    let format = image_format(&mime, href).unwrap_or(ImageFormat::Ico);
    Ok(Some(Arc::new(Image::from_bytes(format, bytes))))
}

fn decode_frame(b64: &str) -> Option<Arc<RenderImage>> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    // PNG from the local Chrome, JPEG from a device.
    let mut rgba = image::load_from_memory(&bytes).ok()?.into_rgba8();
    // GPUI expects BGRA.
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Network "Initiator" column: file:line or the initiator type.
fn initiator_label(i: &chromiumoxide::cdp::browser_protocol::network::Initiator) -> String {
    let frame = i.stack.as_ref().and_then(|s| s.call_frames.first());
    let (url, line) = match (frame, &i.url) {
        (Some(f), _) => (f.url.clone(), Some(f.line_number as f64)),
        (None, Some(u)) => (u.clone(), i.line_number),
        (None, None) => return format!("{:?}", i.r#type),
    };
    let file = url
        .split('?')
        .next()
        .unwrap_or(&url)
        .rsplit('/')
        .next()
        .unwrap_or(&url)
        .to_string();
    match line {
        Some(l) => format!("{file}:{}", l as i64 + 1),
        None => file,
    }
}

fn headers(h: &Headers) -> Vec<(String, String)> {
    let mut v: Vec<_> = h
        .inner()
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|a| a.0.to_lowercase());
    v
}

fn remote_to_string(o: &RemoteObject) -> String {
    if let Some(v) = &o.value {
        return match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
    }
    if let Some(p) = &o.preview {
        let props = p
            .properties
            .iter()
            .map(|p| format!("{}: {}", p.name, p.value.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join(", ");
        let open_close = if p.subtype.as_ref().map(|s| format!("{s:?}")) == Some("Array".into()) {
            ("[", "]")
        } else {
            ("{", "}")
        };
        return format!("{}{props}{}", open_close.0, open_close.1);
    }
    o.description
        .clone()
        .unwrap_or_else(|| format!("{:?}", o.r#type))
}

#[cfg(test)]
impl NetEntry {
    /// Minimal finished entry for unit tests.
    pub fn sample(method: &str, url: &str) -> Self {
        Self {
            id: "1".into(),
            method: method.into(),
            url: url.into(),
            resource_type: "Fetch".into(),
            initiator: String::new(),
            is_navigation: false,
            request_headers: vec![],
            post_data: None,
            status: Some(200),
            status_text: "OK".into(),
            mime: "application/json".into(),
            response_headers: vec![],
            remote_addr: None,
            protocol: None,
            wall_time: 1_700_000_000.0,
            start_ts: 10.0,
            end_ts: Some(10.25),
            size: 123.0,
            resource_size: 100.0,
            priority: "High".into(),
            cookies: vec![],
            set_cookies: vec![],
            error: None,
            body: None,
            image: None,
            pdf: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_pdf_by_mime_or_extension() {
        assert!(is_pdf("application/PDF", "https://x.dev/doc"));
        assert!(is_pdf("", "https://x.dev/a/report.PDF?v=1#page=2"));
        assert!(!is_pdf("application/json", "https://x.dev/pdf/list"));
        // The extension only counts in the path, not in the query.
        assert!(!is_pdf("", "https://x.dev/get?file=a.pdf"));
    }

    #[test]
    fn image_format_by_mime_or_extension() {
        assert_eq!(image_format("image/svg+xml", ""), Some(ImageFormat::Svg));
        assert_eq!(
            image_format("", "https://x.dev/a.JPG?w=2"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(image_format("image/x-icon", ""), Some(ImageFormat::Ico));
        assert_eq!(image_format("text/html", "https://x.dev/"), None);
    }
}
