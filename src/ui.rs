use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use chromiumoxide::cdp::browser_protocol::input::DispatchTouchEventType;
use gpui_kit::assets::IconNamed;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use tokio::sync::mpsc::UnboundedSender;

use crate::device::{self, DeviceProfile};
use crate::engine::{self, Command, Event, LogEntry, LogLevel, NetEntry, Throttle};
use crate::icons::Ico;
use crate::settings::Settings;
use crate::theme::{self, rgb};

mod android;
mod inspector;
mod net_tools;
mod selectable;
mod tools;
pub use inspector::InspectorView;
pub use inspector::init;
use inspector::{AppState, DevTab, Levels, NetColumn, NetDetailTab, NetFilter};

type Headers = Vec<(String, String)>;
/// Right-click menu: position and options (label, text).
type CopyMenu = (Point<Pixels>, Vec<(&'static str, String)>);

const UI_FONT: &str = "Adwaita Sans";
const MONO_FONT: &str = "Adwaita Mono";
const FRAME: f32 = 1.0;
/// Toolbar height (controls + URL).
const TOOLBAR_H: f32 = 72.0;
/// Gap between the bar and the web.
const WINDOW_GAP: f32 = 10.0;
/// Outer margin of the simulator window.
const SIM_PAD: f32 = 2.0;
const PANEL_RADIUS: f32 = 10.0;
/// Width of the floating panel on the right.
const SIDE_W: f32 = 290.0;
/// Resize handle thickness (responsive mode).
const HANDLE: f32 = 10.0;
/// Responsive mode limits; Chrome headless's virtual screen is 1600×2000.
const RESP_W: (i64, i64) = (200, 1600);
const RESP_H: (i64, i64) = (200, 2000);

// zeron's palette (see `theme.rs`).
const BG_PANEL: u32 = theme::SURFACE;
const BG_HEADER: u32 = theme::RAISED;
const BORDER: u32 = theme::HOVER;
const TEXT: u32 = theme::TEXT;
const TEXT_DIM: u32 = theme::TEXT_MUTED;
const ACCENT: u32 = theme::ACCENT_STRONG;
const GREEN: u32 = theme::SUCCESS;
const RED: u32 = theme::DANGER;
const YELLOW: u32 = theme::WARNING;

#[derive(Clone, Copy, PartialEq)]
enum PanelTab {
    Tools,
    Network,
    Console,
    Device,
    Android,
    Settings,
}

pub struct Modev {
    url: String,
    /// Current main frame URL (changes on navigation).
    page_url: String,
    devices: Vec<DeviceProfile>,
    /// Presets deleted by the user (`~/.config/modev/hidden-devices.json`).
    hidden: Vec<String>,
    /// Created by the user (`~/.config/modev/devices.json`).
    custom: Vec<DeviceProfile>,
    device: DeviceProfile,
    page_title: String,
    favicon: Option<Arc<Image>>,
    resize_drag: Option<ResizeDrag>,
    throttle: Throttle,
    /// Separate entity: frames arrive at 30-60 fps and must not re-render the inspector.
    screen: Entity<Screen>,
    connected: bool,
    net: Vec<NetEntry>,
    net_index: HashMap<String, usize>,
    logs: Vec<LogEntry>,
    selected: Option<String>,
    panel_tab: PanelTab,
    /// Inspector window (Console/Network), to focus it from the floating panel.
    pub main_window: Option<AnyWindowHandle>,
    // DevTools-style inspector state.
    dev_tab: DevTab,
    net_detail_tab: NetDetailTab,
    /// Button showing the confirmation ("Copied", "Sent"); cleared after 1.5 s.
    copied: Option<ElementId>,
    copied_reset: Option<Task<()>>,
    /// Wrap long lines to the panel width in the Network detail.
    detail_wrap: bool,
    /// Request loaded in the Replay editor (to avoid overwriting what is being edited).
    replay_for: Option<String>,
    /// The editor's original request: if the body is untouched, it is replayed as-is.
    replay_source: Option<NetEntry>,
    /// Replay in flight (method, URL) until it shows up in Network…
    replay_pending: Option<(String, String)>,
    /// …and its id, to show the response below the editor.
    replay_result: Option<String>,
    net_filter: NetFilter,
    /// Visible columns of the Network table (saved in `~/.config/modev/columns.json`).
    net_columns: Vec<NetColumn>,
    /// Open columns menu (click position).
    columns_menu: Option<Point<Pixels>>,
    /// Application tab of the inspector.
    app: AppState,
    /// Right-click "Copy" menu: position and options (label, text).
    copy_menu: Option<CopyMenu>,
    /// Table sort order: column and ascending.
    net_sort: Option<(NetColumn, bool)>,
    /// ExtraInfo of requests that don't have an entry yet (Chrome sometimes sends it first).
    pending_req_extra: HashMap<String, (Headers, Headers)>,
    pending_resp_extra: HashMap<String, Headers>,
    levels: Levels,
    recording: bool,
    preserve_log: bool,
    disable_cache: bool,
    console_scroll: ScrollHandle,
    /// Network table scroll: follows new requests while it is at the very bottom.
    net_scroll: ScrollHandle,
    /// Animation in progress towards the end of the table.
    net_follow: Option<Task<()>>,
    /// Prevents opening two inspector windows with consecutive clicks.
    inspector_opening: bool,
    display_height: f32,
    status: String,
    pressed: bool,
    screen_bounds: Rc<Cell<Bounds<Pixels>>>,
    zoom: f32,
    focus: FocusHandle,
    cmd: UnboundedSender<Command>,
    settings: Settings,
    capture: tools::Capture,
    /// Simulator window scale (screen pixels per logical pixel).
    window_scale: f32,
    /// Inspector text selection documents (console and Network detail).
    sel_console: gpui_kit::base::TextSelectionHandle,
    sel_detail: gpui_kit::base::TextSelectionHandle,
    /// Devices over adb and the Android page being inspected.
    android: android::Android,
    _events: Task<()>,
}

/// Dragging a web edge in responsive mode.
#[derive(Clone, Copy)]
struct ResizeDrag {
    x: bool,
    y: bool,
    start: Point<Pixels>,
    width: u32,
    height: u32,
    zoom: f32,
}

/// Size editor fields. They live in `SimulatorView` because they need its `Window`.
pub struct DeviceInputs {
    width: Entity<InputState>,
    height: Entity<InputState>,
    dpr: Entity<InputState>,
    safe_top: Entity<InputState>,
    safe_bottom: Entity<InputState>,
    name: Entity<InputState>,
    /// Seconds of the custom timer.
    timer: Entity<InputState>,
    capture_dir: Entity<InputState>,
}

impl Modev {
    pub fn new(url: String, display_height: f32, cx: &mut Context<Self>) -> Self {
        let hidden = device::load_hidden();
        let devices = visible_presets(&hidden);
        let settings = Settings::load();
        let custom = device::load_custom();
        let remembered = settings
            .last_device
            .as_ref()
            .filter(|_| settings.remember_device)
            .and_then(|name| {
                devices
                    .iter()
                    .chain(&custom)
                    .find(|d| d.name == *name)
                    .cloned()
            });
        let device = remembered
            .or_else(|| devices.first().cloned())
            .unwrap_or_else(|| device::presets()[0].clone());
        let zoom = fit_zoom(&device, display_height);
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        engine::spawn(device.clone(), url.clone(), ev_tx, cmd_rx);

        if settings.check_updates {
            // After `new` returns: the check needs `self`.
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| this.check_updates(false, cx));
            })
            .detach();
        }

        let events = cx.spawn(async move |this, cx| {
            while let Some(ev) = ev_rx.recv().await {
                if this.update(cx, |this, cx| this.handle(ev, cx)).is_err() {
                    break;
                }
            }
        });

        Self {
            page_url: url.clone(),
            url,
            devices,
            hidden,
            custom,
            device,
            page_title: String::new(),
            favicon: None,
            resize_drag: None,
            throttle: Throttle::None,
            screen: cx.new(|_| Screen::default()),
            connected: false,
            net: vec![],
            net_index: HashMap::new(),
            logs: vec![],
            selected: None,
            panel_tab: PanelTab::Network,
            main_window: None,
            dev_tab: DevTab::Network,
            net_detail_tab: NetDetailTab::Headers,
            copied: None,
            copied_reset: None,
            detail_wrap: true,
            replay_for: None,
            replay_source: None,
            replay_pending: None,
            replay_result: None,
            net_filter: NetFilter::All,
            net_columns: NetColumn::load(),
            columns_menu: None,
            copy_menu: None,
            app: AppState::default(),
            net_sort: None,
            pending_req_extra: HashMap::new(),
            pending_resp_extra: HashMap::new(),
            levels: Levels::default(),
            recording: true,
            preserve_log: false,
            disable_cache: false,
            console_scroll: ScrollHandle::new(),
            net_scroll: ScrollHandle::new(),
            net_follow: None,
            inspector_opening: false,
            display_height,
            status: "Starting Chromium…".into(),
            pressed: false,
            screen_bounds: Rc::new(Cell::new(Bounds::default())),
            zoom,
            focus: cx.focus_handle(),
            cmd: cmd_tx,
            settings,
            capture: tools::Capture::default(),
            window_scale: 1.0,
            sel_console: gpui_kit::base::TextSelectionHandle::new("", cx),
            sel_detail: gpui_kit::base::TextSelectionHandle::new("", cx),
            android: android::Android::new(cx),
            _events: events,
        }
    }

    fn handle(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::Frame(img) => {
                self.screen.update(cx, |s, cx| {
                    if let Some(old) = s.frame.replace(img) {
                        s.stale.push(old);
                    }
                    cx.notify();
                });
                if !self.connected {
                    self.connected = true;
                    self.status = "Connected".into();
                    cx.notify();
                }
                return;
            }
            Event::Navigated(url) => {
                self.page_url = url;
                self.page_title.clear();
                self.favicon = None;
                if self.dev_tab == DevTab::Application {
                    self.app_reload();
                }
            }
            Event::UrlChanged(url) => self.page_url = url,
            Event::Screenshot { path, png } => self.on_screenshot(path, png, cx),
            Event::Recorded(res) => self.on_recorded(res),
            Event::Storage(data) => self.app.apply(data),
            Event::StorageError(e) => self.app.set_error(e),
            Event::DomStorageChanged { local } => self.app_dom_changed(local),
            Event::PageInfo { title, icon } => {
                self.page_title = title;
                self.favicon = icon;
            }
            Event::NetStarted(e) => {
                // Like Chrome: cleared when navigation *starts*, not when it finishes
                // (frameNavigated arrives after the document request).
                if e.is_navigation && !self.preserve_log {
                    self.clear();
                }
                if !self.recording {
                    return;
                }
                if self
                    .replay_pending
                    .as_ref()
                    .is_some_and(|(m, u)| *m == e.method && *u == e.url)
                {
                    self.replay_pending = None;
                    self.replay_result = Some(e.id.clone());
                }
                let mut e = *e;
                if let Some((headers, cookies)) = self.pending_req_extra.remove(&e.id) {
                    apply_request_extra(&mut e, headers, cookies);
                }
                if let Some(headers) = self.pending_resp_extra.remove(&e.id) {
                    apply_response_extra(&mut e, headers);
                }
                // Redirects reuse the id: it is treated as a new entry.
                self.net_index.insert(e.id.clone(), self.net.len());
                // Like Chrome/Firefox: if it was at the very bottom, the table follows new
                // ones; if the user scrolled up, it stays put. When sorted, new ones don't go last.
                let follow = at_bottom(&self.net_scroll) && self.net_sort.is_none();
                self.net.push(e);
                if follow {
                    self.follow_net_tail(cx);
                }
            }
            Event::NetResponse {
                id,
                status,
                status_text,
                mime,
                headers,
                remote_addr,
                protocol,
            } => {
                if let Some(e) = self.entry_mut(&id) {
                    e.status = Some(status);
                    e.status_text = status_text;
                    e.mime = mime;
                    e.response_headers = headers;
                    e.remote_addr = remote_addr;
                    e.protocol = protocol;
                    // If the ExtraInfo arrived first, its Set-Cookies are not in these headers.
                    merge_set_cookies(e);
                }
            }
            Event::NetRequestExtra {
                id,
                headers,
                cookies,
            } => match self.entry_mut(&id) {
                Some(e) => apply_request_extra(e, headers, cookies),
                None => {
                    self.pending_req_extra.insert(id, (headers, cookies));
                }
            },
            Event::NetResponseExtra { id, headers } => match self.entry_mut(&id) {
                Some(e) => apply_response_extra(e, headers),
                None => {
                    self.pending_resp_extra.insert(id, headers);
                }
            },
            Event::NetData { id, len } => {
                if let Some(e) = self.entry_mut(&id) {
                    e.resource_size += len;
                }
                // Many of these arrive: not worth repainting for each one.
                return;
            }
            Event::NetFinished { id, ts, size } => {
                let mut fetch = false;
                if let Some(e) = self.entry_mut(&id) {
                    e.end_ts = Some(ts);
                    e.size = size;
                    // Chrome only keeps bodies for a while: API/document ones are fetched right away.
                    // Scripts, CSS and images (can weigh MB) are fetched when selected.
                    fetch = e.body.is_none()
                        && matches!(
                            e.resource_type.as_str(),
                            "Fetch" | "Xhr" | "Document" | "Other"
                        );
                }
                if fetch {
                    self.send(Command::FetchBody(id));
                }
            }
            Event::NetFailed { id, ts, error } => {
                if let Some(e) = self.entry_mut(&id) {
                    e.end_ts = Some(ts);
                    e.error = Some(error);
                }
            }
            Event::NetBody { id, body, bytes } => {
                let mut pdf = None;
                if let Some(e) = self.entry_mut(&id) {
                    if engine::is_pdf(&e.mime, &e.url) && e.pdf.is_none() {
                        pdf = bytes.clone();
                    } else if let Some(format) = engine::image_format(&e.mime, &e.url) {
                        // SVGs arrive as text; the rest, as binary.
                        let bytes = bytes.clone().unwrap_or_else(|| body.clone().into_bytes());
                        e.image = Some(Arc::new(Image::from_bytes(format, bytes)));
                    }
                    e.body = Some(body);
                }
                if let Some(bytes) = pdf {
                    let render = cx.background_spawn(async move { engine::render_pdf(bytes) });
                    cx.spawn(async move |this, cx| {
                        let pages = render.await;
                        let _ = this.update(cx, |this, cx| {
                            if let Some(e) = this.entry_mut(&id) {
                                e.pdf = Some(Arc::new(pages));
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
            Event::Log(l) => {
                self.logs.push(l);
                if self.logs.len() > 2000 {
                    self.logs.drain(..500);
                }
                // Same as Network: it only auto-scrolls if it was already at the end.
                if at_bottom(&self.console_scroll) {
                    self.console_scroll.scroll_to_bottom();
                }
            }
            Event::Status(s) => {
                self.android_status(&s);
                self.status = s;
            }
            Event::Attached {
                width,
                height,
                dpr,
                url,
            } => self.on_attached(width, height, dpr, url, cx),
            Event::Detached(reason) => self.on_detached(reason, cx),
            Event::Viewport(w, h) => self.on_viewport(w, h),
        }
        cx.notify();
    }

    fn entry_mut(&mut self, id: &str) -> Option<&mut NetEntry> {
        self.net_index
            .get(id)
            .copied()
            .and_then(|i| self.net.get_mut(i))
    }

    fn send(&self, c: Command) {
        let _ = self.cmd.send(c);
    }

    /// Window position -> page CSS coordinates.
    fn to_css(&self, p: Point<Pixels>) -> (f64, f64) {
        let b = self.screen_bounds.get();
        let x = (f32::from(p.x) - f32::from(b.origin.x)) / self.zoom;
        let y = (f32::from(p.y) - f32::from(b.origin.y)) / self.zoom;
        (
            x.clamp(0.0, self.device.width as f32) as f64,
            y.clamp(0.0, self.device.height as f32) as f64,
        )
    }

    fn select_request(&mut self, id: String) {
        if let Some(e) = self.entry_mut(&id)
            && e.body.is_none()
            && e.end_ts.is_some()
        {
            self.send(Command::FetchBody(id.clone()));
        }
        self.selected = Some(id);
    }

    /// Saves the whole Network log as HAR (opens in Chrome, Firefox, Insomnia…).
    fn export_har(&mut self, cx: &mut Context<Self>) {
        let har = net_tools::har(&self.net);
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let dir = Some(home.join("Downloads"))
            .filter(|d| d.is_dir())
            .unwrap_or(home);
        let name = format!("modev-{}.har", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let path = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let res = serde_json::to_vec_pretty(&har)
                .map_err(std::io::Error::other)
                .and_then(|bytes| std::fs::write(&path, bytes));
            let _ = this.update(cx, |m, cx| {
                m.status = match res {
                    Ok(()) => format!("HAR saved to {}", path.display()),
                    Err(e) => format!("Could not save the HAR: {e}"),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Slides the table to the end (~150 ms). If the user scrolls midway, it stops.
    fn follow_net_tail(&mut self, cx: &mut Context<Self>) {
        if self.net_follow.is_some() {
            return;
        }
        let handle = self.net_scroll.clone();
        self.net_follow = Some(cx.spawn(async move |this, cx| {
            let mut last_set: Option<Pixels> = None;
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let y = handle.offset().y;
                // A value other than the one we set = the user moved the wheel: stop following.
                if last_set.is_some_and(|l| (l - y).abs() > px(0.5)) {
                    break;
                }
                let target = -handle.max_offset().y;
                let remaining = y - target;
                if remaining <= px(0.5) {
                    break;
                }
                let next = if remaining <= px(1.) {
                    target
                } else {
                    y - (remaining * 0.3).max(px(1.))
                };
                handle.set_offset(point(handle.offset().x, next));
                last_set = Some(next);
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |this, _| this.net_follow = None);
        }));
    }

    fn clear(&mut self) {
        self.net.clear();
        self.net_index.clear();
        self.pending_req_extra.clear();
        self.pending_resp_extra.clear();
        self.logs.clear();
        self.selected = None;
    }

    fn is_responsive(&self) -> bool {
        self.device.name == device::RESPONSIVE
    }

    /// Switches device and reloads (the UA only applies to new requests).
    fn set_device(&mut self, d: DeviceProfile, cx: &mut Context<Self>) {
        if d == self.device {
            return;
        }
        if self.android.session.is_some() {
            // From the Android page straight to this device.
            self.detach(Some(d.clone()));
            self.send(Command::SetDevice(d));
            return;
        }
        self.zoom = fit_zoom(&d, self.display_height);
        self.device = d.clone();
        if self.settings.remember_device {
            self.settings.last_device = Some(d.name.clone());
            self.settings.save();
        }
        self.screen.update(cx, |s, _| {
            if let Some(old) = s.frame.take() {
                s.stale.push(old);
            }
        });
        self.send(Command::SetDevice(d));
        cx.notify();
    }

    /// Adjusts the responsive profile without reloading the page.
    fn resize_responsive(&mut self, mut d: DeviceProfile, cx: &mut Context<Self>) {
        d.name = device::RESPONSIVE.into();
        d.width = (d.width as i64).clamp(RESP_W.0, RESP_W.1) as u32;
        d.height = (d.height as i64).clamp(RESP_H.0, RESP_H.1) as u32;
        d.dpr = d.dpr.clamp(1.0, 4.0);
        d.safe_top = d.safe_top.min(200);
        d.safe_bottom = d.safe_bottom.min(200);
        if self.android.session.is_some() {
            self.detach(Some(d.clone()));
            self.send(Command::Resize(d));
            return;
        }
        if d != self.device {
            self.device = d.clone();
            self.send(Command::Resize(d));
            cx.notify();
        }
    }

    fn drag_to(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.resize_drag else { return };
        if ev.pressed_button != Some(MouseButton::Left) {
            self.resize_drag = None;
            return;
        }
        let delta = |a: Pixels, b: Pixels| (f32::from(a - b) / drag.zoom).round() as i64;
        let mut d = self.device.clone();
        if drag.x {
            d.width = (drag.width as i64 + delta(ev.position.x, drag.start.x)).max(0) as u32;
        }
        if drag.y {
            d.height = (drag.height as i64 + delta(ev.position.y, drag.start.y)).max(0) as u32;
        }
        self.resize_responsive(d, cx);
    }

    /// Saves the current responsive profile as a user device.
    fn save_custom(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() || name == device::RESPONSIVE {
            return;
        }
        let d = DeviceProfile {
            name: name.into(),
            ..self.device.clone()
        };
        match self.custom.iter_mut().find(|c| c.name == d.name) {
            Some(c) => *c = d.clone(),
            None => self.custom.push(d.clone()),
        }
        if let Err(e) = device::save_custom(&self.custom) {
            eprintln!("[modev] could not save devices.json: {e}");
        }
        self.device = d;
        cx.notify();
    }

    fn delete_preset(&mut self, i: usize, cx: &mut Context<Self>) {
        let d = self.devices.remove(i);
        self.hidden.push(d.name);
        if let Err(e) = device::save_hidden(&self.hidden) {
            eprintln!("[modev] could not save hidden-devices.json: {e}");
        }
        cx.notify();
    }

    fn restore_presets(&mut self, cx: &mut Context<Self>) {
        self.hidden.clear();
        self.devices = visible_presets(&self.hidden);
        if let Err(e) = device::save_hidden(&self.hidden) {
            eprintln!("[modev] could not save hidden-devices.json: {e}");
        }
        cx.notify();
    }

    fn delete_custom(&mut self, i: usize, cx: &mut Context<Self>) {
        self.custom.remove(i);
        if let Err(e) = device::save_custom(&self.custom) {
            eprintln!("[modev] could not save devices.json: {e}");
        }
        cx.notify();
    }
}

fn visible_presets(hidden: &[String]) -> Vec<DeviceProfile> {
    device::presets()
        .into_iter()
        .filter(|d| !hidden.contains(&d.name))
        .collect()
}

/// Is the scroll at the very bottom (or nothing to scroll yet)?
fn at_bottom(handle: &ScrollHandle) -> bool {
    -handle.offset().y >= handle.max_offset().y - px(4.)
}

/// Real request headers: they include Cookie and others that `requestWillBeSent` omits.
fn apply_request_extra(
    e: &mut NetEntry,
    headers: Vec<(String, String)>,
    cookies: Vec<(String, String)>,
) {
    if !headers.is_empty() {
        e.request_headers = headers;
    }
    e.cookies = cookies;
}

fn apply_response_extra(e: &mut NetEntry, headers: Vec<(String, String)>) {
    e.set_cookies = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .flat_map(|(_, v)| v.split('\n').map(str::to_owned))
        .collect();
    merge_set_cookies(e);
}

/// Adds the Set-Cookies to the response headers if not there yet (one per line, like DevTools).
fn merge_set_cookies(e: &mut NetEntry) {
    if e.status.is_none()
        || e.response_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
    {
        return;
    }
    for c in &e.set_cookies {
        e.response_headers.push(("set-cookie".into(), c.clone()));
    }
}

/// Width of the web column (min. 360 for the bar). The difference with the web is even
/// so that, centered, it lands on a whole pixel (a half pixel interpolates and blurs).
fn column_width(panel_w: f32) -> f32 {
    if panel_w >= 360.0 {
        panel_w
    } else {
        panel_w + ((360.0 - panel_w) / 2.0).ceil() * 2.0
    }
}

/// Scale so the device fits the host screen (max. 100%).
fn fit_zoom(d: &DeviceProfile, display_height: f32) -> f32 {
    let avail = display_height * 0.9
        - TOOLBAR_H
        - WINDOW_GAP
        - SIM_PAD * 2.0
        - FRAME * 2.0
        - handle_space(d);
    (avail / d.height as f32).clamp(0.3, 1.0)
}

/// Space below the web for the bottom handle (responsive mode only).
fn handle_space(d: &DeviceProfile) -> f32 {
    if d.name == device::RESPONSIVE {
        HANDLE
    } else {
        0.0
    }
}

#[derive(Default)]
pub struct Screen {
    frame: Option<Arc<RenderImage>>,
    stale: Vec<Arc<RenderImage>>,
}

/// Simulator window: bar (controls + URL) and web, as two fixed panels
/// inside a single transparent window (one dock icon).
pub struct SimulatorView {
    state: Entity<Modev>,
    /// Last size applied to the window.
    window_for: (String, u32, u32),
    /// Last responsive profile dumped into the fields (don't overwrite what is being typed).
    inputs_for: Option<DeviceProfile>,
    url_input: Entity<InputState>,
    /// Page URL last shown in the bar.
    url_for: String,
    inputs: DeviceInputs,
    _subscriptions: Vec<Subscription>,
}

impl SimulatorView {
    pub fn new(state: Entity<Modev>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe_in(&state, window, |this, state, window, cx| {
            let m = state.read(cx);
            let key = (m.device.name.clone(), m.device.width, m.device.height);
            if key != this.window_for {
                this.window_for = key;
                window.resize(m.simulator_window_size());
            }
            this.sync_inputs(window, cx);
            this.sync_url(window, cx);
            cx.notify();
        });
        let screen = state.read(cx).screen.clone();
        let observe_screen = cx.observe(&screen, |_, _, cx| cx.notify());

        let url = state.read(cx).url.clone();
        let url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("http://localhost:8100")
                .default_value(url)
        });
        let on_enter =
            cx.subscribe_in(&url_input, window, |this, input, ev: &InputEvent, _, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let mut url = input.read(cx).value().trim().to_string();
                    if url.is_empty() {
                        return;
                    }
                    if !url.contains("://") {
                        url = format!("http://{url}");
                    }
                    this.state.update(cx, |m, _| {
                        m.url = url.clone();
                        m.send(Command::Navigate(url));
                    });
                }
            });

        let input = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let inputs = DeviceInputs {
            width: input("width", window, cx),
            height: input("height", window, cx),
            dpr: input("dpr", window, cx),
            safe_top: input("top", window, cx),
            safe_bottom: input("bottom", window, cx),
            name: input("Device name", window, cx),
            timer: input("s", window, cx),
            capture_dir: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(state.read(cx).settings.capture_dir.clone())
            }),
        };
        let mut subscriptions = vec![
            observe,
            observe_screen,
            on_enter,
            // The user can change the buttons' order/side in the desktop settings.
            cx.observe_button_layout_changed(window, |_, _, cx| cx.notify()),
        ];
        // Numbers are applied on Enter or when leaving the field, not on every keystroke.
        for field in [
            &inputs.width,
            &inputs.height,
            &inputs.dpr,
            &inputs.safe_top,
            &inputs.safe_bottom,
        ] {
            subscriptions.push(cx.subscribe_in(
                field,
                window,
                |this, _, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.apply_inputs(cx);
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &inputs.capture_dir,
            window,
            |this, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let dir = input.read(cx).value().to_string();
                    this.state.update(cx, |m, _| m.set_capture_dir(dir));
                }
            },
        ));
        // "Auto" theme: follow the desktop when it changes.
        subscriptions.push(cx.observe_window_appearance(window, |this, _, cx| {
            let theme = this.state.read(cx).settings.theme;
            theme::set_light(theme.is_light(cx), cx);
        }));
        subscriptions.push(cx.subscribe_in(
            &inputs.name,
            window,
            |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.save_device(window, cx);
                }
            },
        ));

        let d = &state.read(cx).device;
        let window_for = (d.name.clone(), d.width, d.height);
        let mut this = Self {
            window_for,
            inputs_for: None,
            state,
            url_input,
            url_for: String::new(),
            inputs,
            _subscriptions: subscriptions,
        };
        this.sync_inputs(window, cx);
        this
    }

    /// The bar follows the page (navigations, SPA routes, a device's page), except while
    /// the user is typing in it.
    fn sync_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.state.read(cx).page_url.clone();
        if url == self.url_for || self.url_input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        self.url_input
            .update(cx, |f, cx| f.set_value(url.clone(), window, cx));
        self.url_for = url;
    }

    /// Dumps the responsive profile into the fields when it changes from outside (drag, selection).
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let d = self.state.read(cx).device.clone();
        if d.name != device::RESPONSIVE || self.inputs_for.as_ref() == Some(&d) {
            return;
        }
        let i = &self.inputs;
        for (field, value) in [
            (&i.width, d.width.to_string()),
            (&i.height, d.height.to_string()),
            (&i.dpr, d.dpr.to_string()),
            (&i.safe_top, d.safe_top.to_string()),
            (&i.safe_bottom, d.safe_bottom.to_string()),
        ] {
            field.update(cx, |f, cx| f.set_value(value, window, cx));
        }
        self.inputs_for = Some(d);
    }

    fn apply_inputs(&mut self, cx: &mut Context<Self>) {
        let mut d = self.state.read(cx).device.clone();
        if d.name != device::RESPONSIVE {
            return;
        }
        let i = &self.inputs;
        let num = |f: &Entity<InputState>, cx: &App| {
            f.read(cx)
                .value()
                .trim()
                .replace(',', ".")
                .parse::<f64>()
                .ok()
        };
        if let Some(v) = num(&i.width, cx) {
            d.width = v.max(0.0) as u32;
        }
        if let Some(v) = num(&i.height, cx) {
            d.height = v.max(0.0) as u32;
        }
        if let Some(v) = num(&i.dpr, cx) {
            d.dpr = v;
        }
        if let Some(v) = num(&i.safe_top, cx) {
            d.safe_top = v.max(0.0) as u32;
        }
        if let Some(v) = num(&i.safe_bottom, cx) {
            d.safe_bottom = v.max(0.0) as u32;
        }
        // Force re-sync: if something invalid or out of range was typed, the real value comes back.
        self.inputs_for = None;
        self.state.update(cx, |m, cx| {
            m.resize_responsive(d, cx);
            cx.notify();
        });
    }

    fn save_device(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inputs(cx);
        let name = self.inputs.name.read(cx).value().to_string();
        self.state.update(cx, |m, cx| m.save_custom(&name, cx));
        if !name.trim().is_empty() {
            self.inputs
                .name
                .update(cx, |f, cx| f.set_value("", window, cx));
        }
    }
}

impl Render for SimulatorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = self.state.read(cx).screen.clone();
        let stale = screen.update(cx, |s, _| std::mem::take(&mut s.stale));
        for old in stale {
            let _ = window.drop_image(old);
        }

        // Fit the scale to the size the compositor actually gave the window,
        // instead of assuming the one we asked for: this way the web is never cropped.
        let viewport = window.viewport_size();
        let window_scale = window.scale_factor();
        self.state.update(cx, |m, _| {
            m.fit_to(viewport, window_scale);
            m.sync_capture_scale(window_scale);
        });

        let d = self.state.read(cx).device.clone();
        let (reload_state, home_state) = (self.state.clone(), self.state.clone());
        let devtools_state = self.state.clone();

        // Row 1: title (drag zone) + GNOME-style controls on the right.
        let [left_caps, right_caps] = caption_layout(window, cx);
        // Both sides equally wide so the title stays centered.
        let caps_w = left_caps.len().max(right_caps.len()) as f32 * (CAPTION_SIZE + CAPTION_GAP);
        // Closing the simulator closes modev (and the headless Chrome).
        let quit = |_: &mut Window, cx: &mut App| cx.quit();
        let title_row = div()
            .h(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .child(caption_strip(&left_caps, quit).w(px(caps_w)).flex_none())
            .child(
                div()
                    .flex_1()
                    // Without it a long title (an Android page) pushes the buttons out.
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(rgb(TEXT_DIM))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                    .child(div().min_w_0().truncate().child(format!(
                        "{} – {} · {}×{}",
                        d.name, d.os_label, d.width, d.height
                    ))),
            )
            .child(
                caption_strip(&right_caps, quit)
                    .w(px(caps_w))
                    .flex_none()
                    .justify_end(),
            );

        // Row 2: reload, home and URL.
        let nav_row = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .pb(px(8.))
            .text_size(px(15.))
            .child(icon_button(
                "reload",
                Ico::Refresh,
                "Reload",
                move |_, _, cx| reload_state.update(cx, |m, _| m.send(Command::Reload)),
            ))
            .child(icon_button("home", Ico::Home, "Home", move |_, _, cx| {
                home_state.update(cx, |m, _| {
                    // Inspecting a device: the page it was on, not the local server.
                    let url = m
                        .android
                        .session
                        .as_ref()
                        .map_or(&m.url, |s| &s.home)
                        .clone();
                    m.send(Command::Navigate(url))
                })
            }))
            .child(div().flex_1().child(Input::new(&self.url_input).small()))
            .child(icon_button(
                "devtools",
                Ico::Search,
                "Inspector",
                move |_, _, cx| devtools_state.update(cx, |m, cx| m.open_monitor(cx)),
            ));

        let toolbar = div()
            .w_full()
            .h(px(TOOLBAR_H))
            .flex()
            .flex_col()
            .justify_between()
            .rounded(px(PANEL_RADIUS))
            .bg(rgb(BG_PANEL))
            .border_1()
            .border_color(rgb(BORDER))
            .child(title_row)
            .child(nav_row);

        let zoom = self.state.read(cx).zoom;
        let column_w = column_width(d.width as f32 * zoom + FRAME * 2.0);
        let left = div()
            .w(px(column_w))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(WINDOW_GAP))
            .child(toolbar)
            .child(
                self.state
                    .update(cx, |m, cx| m.render_simulator(cx).into_any_element()),
            );

        let save =
            cx.listener(|this: &mut Self, _: &ClickEvent, window, cx| this.save_device(window, cx));
        let panel = self.state.update(cx, |m, cx| {
            m.render_floating_panel(&self.inputs, save, cx)
                .into_any_element()
        });

        // Transparent background: the desktop shows between the panels.
        div()
            .size_full()
            .p(px(SIM_PAD))
            .flex()
            .items_start()
            .gap(px(WINDOW_GAP))
            .font_family(UI_FONT)
            .text_color(rgb(TEXT))
            // Responsive handle dragging (can leave the web area).
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                this.state.update(cx, |m, cx| m.drag_to(ev, cx))
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.state.update(cx, |m, _| m.resize_drag = None)),
            )
            .child(left)
            .child(panel)
    }
}

// ───────────────────────── Simulator ─────────────────────────

impl Modev {
    pub fn simulator_window_size(&self) -> Size<Pixels> {
        let z = self.zoom;
        let w = column_width(self.device.width as f32 * z + FRAME * 2.0);
        size(
            px(w + WINDOW_GAP + SIDE_W + SIM_PAD * 2.0),
            px(self.device.height as f32 * z
                + FRAME * 2.0
                + TOOLBAR_H
                + WINDOW_GAP
                + SIM_PAD * 2.0
                + handle_space(&self.device)),
        )
    }

    /// `scale`: window scale. The zoom is adjusted so the web occupies a whole number
    /// of pixels: with a fractional width GPUI interpolates the frame and everything looks blurry.
    fn fit_to(&mut self, viewport: Size<Pixels>, scale: f32) {
        let avail_h = f32::from(viewport.height)
            - SIM_PAD * 2.0
            - TOOLBAR_H
            - WINDOW_GAP
            - FRAME * 2.0
            - handle_space(&self.device);
        let avail_w = f32::from(viewport.width) - SIM_PAD * 2.0 - FRAME * 2.0 - WINDOW_GAP - SIDE_W;
        let d = &self.device;
        let zoom = (avail_h / d.height as f32)
            .min(avail_w / d.width as f32)
            .clamp(0.2, 1.0);
        // Whole width in logical px: also whole on screen with any whole scale.
        self.zoom = (d.width as f32 * zoom).floor() / d.width as f32;
        self.window_scale = scale;
    }

    fn render_simulator(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let z = self.zoom;
        let d = &self.device;
        let (w, h) = (d.width as f32 * z, d.height as f32 * z);
        let radius = PANEL_RADIUS;
        let bounds_cell = self.screen_bounds.clone();

        let mut screen = div()
            .id("screen")
            .relative()
            .w(px(w))
            .h(px(h))
            .rounded(px(radius))
            .overflow_hidden()
            .bg(rgb(0xffffff))
            .track_focus(&self.focus)
            .cursor_pointer()
            .child(
                canvas(move |b, _, _| bounds_cell.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.pressed = true;
                    let (x, y) = this.to_css(ev.position);
                    this.send(Command::Touch(DispatchTouchEventType::TouchStart, x, y));
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, _| {
                let (x, y) = this.to_css(ev.position);
                if this.pressed {
                    this.send(Command::Touch(DispatchTouchEventType::TouchMove, x, y));
                } else if this.capture.rec_since.is_some() {
                    // Only for the recording: on mobile there is no hover to send to Chrome.
                    this.send(Command::Pointer(Some((x, y))));
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                if !*hovered && this.capture.rec_since.is_some() {
                    this.send(Command::Pointer(None));
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, _| {
                    // Without a previous touchStart (e.g. releasing a resize handle) there is no touch.
                    if !std::mem::take(&mut this.pressed) {
                        return;
                    }
                    let (x, y) = this.to_css(ev.position);
                    this.send(Command::Touch(DispatchTouchEventType::TouchEnd, x, y));
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, _| {
                    if this.pressed {
                        this.pressed = false;
                        let (x, y) = this.to_css(ev.position);
                        this.send(Command::Touch(DispatchTouchEventType::TouchEnd, x, y));
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, _| {
                let (x, y) = this.to_css(ev.position);
                let (dx, dy) = match ev.delta {
                    ScrollDelta::Pixels(p) => {
                        (f32::from(p.x) / this.zoom, f32::from(p.y) / this.zoom)
                    }
                    ScrollDelta::Lines(l) => (l.x * 40.0, l.y * 40.0),
                };
                this.send(Command::Wheel(x, y, -dx as f64, -dy as f64));
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if let Some(c) = key_command(&ev.keystroke) {
                    this.send(c);
                    cx.stop_propagation();
                }
            }));

        if let Some(frame) = self.screen.read(cx).frame.clone() {
            // At its pixel size (1:1), not stretched to the box: Chrome already sends it at
            // the on-screen size and any rescaling in GPUI blurs it.
            let native = frame.size(0);
            let (fw, fh) = (
                native.width.0 as f32 / self.window_scale,
                native.height.0 as f32 / self.window_scale,
            );
            screen = screen.child(
                img(frame)
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(fw))
                    .h(px(fh))
                    .rounded(px(radius))
                    .object_fit(ObjectFit::Fill),
            );
        } else {
            screen = screen.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0x888888))
                    .text_size(px(13.))
                    .child(self.status.clone()),
            );
        }
        // Countdown of the timed screenshot and recording indicator.
        if let Some(n) = self.capture.countdown {
            screen = screen.child(
                div()
                    .absolute()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x00000055))
                    .child(
                        div()
                            .text_size(px(96.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(gpui_kit::rgb(0xffffff))
                            .child(n.to_string()),
                    ),
            );
        }
        if let Some(t) = self.capture.rec_since {
            let s = t.elapsed().as_secs();
            screen = screen.child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .px(px(8.))
                    .py(px(3.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .rounded_full()
                    .bg(rgba(0x000000aa))
                    .text_size(px(11.))
                    .text_color(gpui_kit::rgb(0xffffff))
                    .child(
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(gpui_kit::rgb(0xff3b30)),
                    )
                    .child(format!("REC {:02}:{:02}", s / 60, s % 60)),
            );
        }

        // Web panel: 1px border with the same corners as the bar.
        let panel = div()
            .relative()
            .p(px(FRAME))
            .rounded(px(radius + FRAME))
            .bg(rgb(BORDER))
            .child(screen);
        if !self.is_responsive() {
            return panel;
        }
        // Chrome DevTools handles: right edge, bottom edge and corner.
        let handle = |id: &'static str, x: bool, y: bool, cx: &mut Context<Self>| {
            let grip = || div().rounded_full().bg(rgb(TEXT_DIM));
            div()
                .id(id)
                .absolute()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .hover(|s| s.bg(rgb(BORDER)))
                .cursor(match (x, y) {
                    (true, false) => CursorStyle::ResizeLeftRight,
                    (false, true) => CursorStyle::ResizeUpDown,
                    _ => CursorStyle::ResizeUpLeftDownRight,
                })
                .when(x && !y, |s| {
                    s.top_0()
                        .bottom_0()
                        .right(px(-HANDLE))
                        .w(px(HANDLE))
                        .child(grip().w(px(3.)).h(px(28.)))
                })
                .when(y && !x, |s| {
                    s.left_0()
                        .right_0()
                        .bottom(px(-HANDLE))
                        .h(px(HANDLE))
                        .child(grip().h(px(3.)).w(px(28.)))
                })
                .when(x && y, |s| {
                    s.right(px(-HANDLE)).bottom(px(-HANDLE)).size(px(HANDLE))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.resize_drag = Some(ResizeDrag {
                            x,
                            y,
                            start: ev.position,
                            width: this.device.width,
                            height: this.device.height,
                            zoom: this.zoom,
                        });
                        cx.stop_propagation();
                    }),
                )
        };
        panel
            .mb(px(HANDLE))
            .child(handle("resize-x", true, false, cx))
            .child(handle("resize-y", false, true, cx))
            .child(handle("resize-xy", true, true, cx))
    }

    // ───────────────────────── Side panel ──────────────────────────────────

    fn render_floating_panel(
        &mut self,
        inputs: &DeviceInputs,
        on_save: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // "Pill" tab bar on top of the panel.
        let update_dot = self.capture.update.is_some();
        let tab = |id: &'static str, icon: Ico, t: PanelTab, cx: &mut Context<Self>| {
            let active = self.panel_tab == t;
            div()
                .id(id)
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .text_color(rgb(if active { TEXT } else { TEXT_DIM }))
                .when(active, |s| s.bg(rgb(BORDER)))
                .hover(|s| s.bg(rgb(BORDER)))
                .child(Icon::new(icon).small())
                // New version available: dot on the Settings tab.
                .when(update_dot && t == PanelTab::Settings, |d| {
                    d.relative().child(
                        div()
                            .absolute()
                            .top(px(5.))
                            .right(px(5.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(rgb(ACCENT)),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.panel_tab = t;
                    this.android_visible(t == PanelTab::Android);
                    cx.notify();
                }))
        };
        let tabs = div()
            .w_full()
            .h(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .rounded(px(12.))
            .bg(rgb(BG_PANEL))
            .border_1()
            .border_color(rgb(BORDER))
            .child(tab("pt-tools", Ico::Sliders, PanelTab::Tools, cx))
            .child(tab("pt-net", Ico::Network, PanelTab::Network, cx))
            .child(tab("pt-console", Ico::Terminal, PanelTab::Console, cx))
            .child(tab("pt-device", Ico::Smartphone, PanelTab::Device, cx))
            .child(
                div()
                    .relative()
                    .child(tab("pt-android", Ico::Android, PanelTab::Android, cx))
                    // A device was connected: the tab shows it until opened.
                    .when(self.android.new_device, |s| {
                        s.child(dot(GREEN).size(px(7.)).absolute().top(px(3.)).right(px(3.)))
                    }),
            )
            .child(tab("pt-settings", Ico::Settings, PanelTab::Settings, cx))
            .child(
                div()
                    .id("pt-open")
                    .size(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .text_color(rgb(TEXT_DIM))
                    .hover(|s| s.bg(rgb(BORDER)))
                    .child(Icon::new(Ico::Code).small())
                    .on_click(cx.listener(|this, _, _, cx| this.open_monitor(cx))),
            );

        let icon: AnyElement = match &self.favicon {
            Some(icon) => img(icon.clone())
                .size(px(34.))
                .rounded(px(8.))
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => div()
                .size(px(34.))
                .rounded(px(8.))
                .bg(linear_gradient(
                    135.,
                    linear_color_stop(rgb(theme::ACCENT), 0.),
                    linear_color_stop(rgb(theme::ACCENT_STRONG), 1.),
                ))
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(Ico::Globe).small())
                .into_any_element(),
        };
        let title = if self.page_title.is_empty() {
            "Ionic App".to_string()
        } else {
            self.page_title.clone()
        };
        let app = div().flex().items_center().gap(px(10.)).child(icon).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(TEXT_DIM))
                        .truncate()
                        .child(self.page_url.clone()),
                ),
        );

        let body: AnyElement = match self.panel_tab {
            PanelTab::Network => self.panel_network(cx).into_any_element(),
            PanelTab::Console => self.panel_console(cx).into_any_element(),
            PanelTab::Device => self.panel_device(inputs, on_save, cx).into_any_element(),
            PanelTab::Tools => self.panel_tools(inputs, cx).into_any_element(),
            PanelTab::Android => self.panel_android(cx).into_any_element(),
            PanelTab::Settings => self.panel_settings(inputs, cx).into_any_element(),
        };

        let card = div()
            .w_full()
            .flex()
            .flex_col()
            .p(px(14.))
            .rounded(px(14.))
            .bg(rgb(BG_PANEL))
            .border_1()
            .border_color(rgb(BORDER))
            .child(app)
            .child(div().my(px(12.)).h(px(1.)).bg(rgb(BORDER)))
            .child(body);

        div()
            .w(px(SIDE_W))
            .flex()
            .flex_col()
            .gap(px(WINDOW_GAP))
            .child(tabs)
            .child(card)
    }

    /// Brings the inspector window to the front, or opens it if it is closed.
    fn open_monitor(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.main_window {
            if h.update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
            self.main_window = None;
        }
        if self.inspector_opening {
            return;
        }
        self.inspector_opening = true;
        let state = cx.entity();
        // Deferred: the window cannot be opened while this entity is in use.
        cx.defer(move |cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1180.), px(760.)),
                    cx,
                ))),
                titlebar: None,
                window_decorations: Some(WindowDecorations::Client),
                window_min_size: Some(size(px(640.), px(320.))),
                // Transparent and without gpui-kit's Root (opaque background and straight frame),
                // like the simulator: the rounded corners are painted by the view itself.
                window_background: WindowBackgroundAppearance::Transparent,
                // Same app_id as the simulator: the dock groups both under one icon.
                app_id: Some("modev".into()),
                ..Default::default()
            };
            let opened = cx.open_window(options, {
                let state = state.clone();
                move |window, cx| cx.new(|cx| InspectorView::new(state, window, cx))
            });
            state.update(cx, |m, _| {
                m.inspector_opening = false;
                m.main_window = opened.ok().map(Into::into);
            });
        });
    }

    fn panel_network(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.throttle;
        let speed = div()
            .id("speed")
            .flex()
            .items_center()
            .gap(px(10.))
            .p(px(10.))
            .rounded(px(10.))
            .bg(rgb(BG_HEADER))
            .hover(|s| s.bg(rgb(BORDER)))
            .child(
                Icon::new(if t == Throttle::Offline {
                    Ico::WifiOff
                } else {
                    Ico::Wifi
                })
                .small(),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(12.)).child(t.label()))
                    .child(div().text_size(px(10.)).text_color(rgb(TEXT_DIM)).child(
                        if t == Throttle::None {
                            "No restriction"
                        } else {
                            "Active"
                        },
                    )),
            )
            .child(
                div()
                    .text_color(rgb(TEXT_DIM))
                    .child(Icon::new(Ico::ChevronsUpDown).small()),
            )
            // Each click moves to the next network profile.
            .on_click(cx.listener(|this, _, _, cx| {
                let i = Throttle::ALL
                    .iter()
                    .position(|x| *x == this.throttle)
                    .unwrap_or(0);
                let next = Throttle::ALL[(i + 1) % Throttle::ALL.len()];
                this.throttle = next;
                this.send(Command::Throttle(next));
                cx.notify();
            }));

        let mut recent = div().flex().flex_col().gap(px(10.));
        if self.net.is_empty() {
            recent = recent.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(TEXT_DIM))
                    .child("No requests yet"),
            );
        }
        for e in self.net.iter().rev().take(5) {
            let id = e.id.clone();
            recent = recent.child(
                div()
                    .id(SharedString::from(format!("recent-{}", e.id)))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .text_size(px(11.))
                    .rounded(px(6.))
                    .hover(|s| s.bg(rgb(BG_HEADER)))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(status_badge(e))
                            .child(div().text_color(rgb(TEXT_DIM)).child(e.method.clone()))
                            .child(
                                div()
                                    .ml_auto()
                                    .text_color(rgb(TEXT_DIM))
                                    .child(clock(e.wall_time, false)),
                            ),
                    )
                    .child(div().truncate().child(short_url(&e.url)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dev_tab = DevTab::Network;
                        this.select_request(id.clone());
                        this.open_monitor(cx);
                        cx.notify();
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .child(panel_section("NETWORK THROTTLING"))
            .child(speed)
            .child(panel_section("NETWORK MONITOR"))
            .child(recent)
            .child(panel_button(
                "open-net",
                "Open Network Monitor",
                cx.listener(|this, _, _, cx| {
                    this.dev_tab = DevTab::Network;
                    this.open_monitor(cx);
                    cx.notify();
                }),
            ))
    }

    fn panel_console(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut logs = div().flex().flex_col().gap(px(8.));
        if self.logs.is_empty() {
            logs = logs.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(TEXT_DIM))
                    .child("No logs yet"),
            );
        }
        for l in self.logs.iter().rev().take(8) {
            let color = level_color(l.level);
            logs = logs.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .text_size(px(10.))
                            .child(
                                div()
                                    .text_color(rgb(color))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(level_label(l.level)),
                            )
                            .child(
                                div()
                                    .ml_auto()
                                    .text_color(rgb(TEXT_DIM))
                                    .child(clock(l.wall_time, false)),
                            ),
                    )
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(px(11.))
                            .text_color(rgb(if l.level == LogLevel::Log {
                                TEXT
                            } else {
                                color
                            }))
                            .line_clamp(2)
                            .child(l.text.clone()),
                    ),
            );
        }
        let (errors, warns) = self.logs.iter().fold((0, 0), |(e, w), l| match l.level {
            LogLevel::Error => (e + 1, w),
            LogLevel::Warn => (e, w + 1),
            _ => (e, w),
        });

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .text_size(px(11.))
                    .child(
                        div()
                            .text_color(rgb(TEXT_DIM))
                            .child(format!("{} logs", self.logs.len())),
                    )
                    .child(div().text_color(rgb(RED)).child(format!("{errors} errors")))
                    .child(
                        div()
                            .text_color(rgb(YELLOW))
                            .child(format!("{warns} warnings")),
                    ),
            )
            .child(panel_section("RECENT LOGS"))
            .child(logs)
            .child(panel_button(
                "open-logs",
                "Open Console",
                cx.listener(|this, _, _, cx| {
                    this.dev_tab = DevTab::Console;
                    this.open_monitor(cx);
                    cx.notify();
                }),
            ))
    }

    fn panel_device(
        &mut self,
        inputs: &DeviceInputs,
        on_save: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let row = |id: ElementId, name: String, info: String, active: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(6.))
                .text_size(px(12.))
                .when(active, |s| s.bg(rgb(ACCENT)))
                .when(!active, |s| s.hover(|s| s.bg(rgb(BG_HEADER))))
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(
                    div()
                        .text_color(rgb(if active { TEXT } else { TEXT_DIM }))
                        .child(info),
                )
        };
        let info = |d: &DeviceProfile| format!("{}×{} @{}x", d.width, d.height, d.dpr);
        let delete = |id: ElementId| {
            div()
                .id(id)
                .px(px(4.))
                .rounded(px(4.))
                .text_color(rgb(TEXT_DIM))
                .hover(|s| s.bg(rgb(BORDER)).text_color(rgb(RED)))
                .child(Icon::new(Ico::Close).xsmall())
        };

        let responsive = self.is_responsive();
        let mut devices = div()
            .id("device-list")
            .max_h(px(320.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                row(
                    "responsive".into(),
                    device::RESPONSIVE.into(),
                    "free".into(),
                    responsive,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.is_responsive() {
                        let d = DeviceProfile::responsive_from(&this.device);
                        this.resize_responsive(d, cx);
                    }
                })),
            );
        for (i, d) in self.devices.iter().enumerate() {
            devices = devices.child(
                row(
                    ("device", i).into(),
                    d.name.clone(),
                    info(d),
                    *d == self.device,
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_device(this.devices[i].clone(), cx)),
                )
                .child(delete(("device-del", i).into()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.delete_preset(i, cx);
                    },
                ))),
            );
        }
        for (i, d) in self.custom.iter().enumerate() {
            devices = devices.child(
                row(
                    ("custom", i).into(),
                    d.name.clone(),
                    info(d),
                    *d == self.device,
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_device(this.custom[i].clone(), cx)),
                )
                .child(delete(("custom-del", i).into()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.delete_custom(i, cx);
                    },
                ))),
            );
        }

        let d = &self.device;
        let mut panel = div()
            .flex()
            .flex_col()
            .child(panel_section("DEVICE"))
            .child(devices)
            .when(!self.hidden.is_empty(), |p| {
                p.child(panel_button(
                    "restore-devices",
                    "Restore devices",
                    cx.listener(|this, _, _, cx| {
                        this.restore_presets(cx);
                    }),
                ))
            });

        if responsive {
            let field = |label: &'static str, input: &Entity<InputState>| {
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(TEXT_DIM))
                            .child(label),
                    )
                    .child(Input::new(input).small())
            };
            let os = |id: &'static str, label: &'static str, ios: bool, cx: &mut Context<Self>| {
                let active = self.device.is_ios() == ios;
                div()
                    .id(id)
                    .flex_1()
                    .flex()
                    .justify_center()
                    .py(px(4.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .when(active, |s| s.bg(rgb(ACCENT)))
                    .when(!active, |s| s.hover(|s| s.bg(rgb(BORDER))))
                    .child(label)
                    // Changes the UA: reload so Ionic picks the ios/md mode.
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut d = this.device.clone();
                        d.set_ios(ios);
                        this.set_device(d, cx);
                    }))
            };
            panel = panel
                .child(panel_section("SIZE"))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(field("Width", &inputs.width))
                        .child(field("Height", &inputs.height))
                        .child(field("DPR", &inputs.dpr)),
                )
                .child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .gap(px(6.))
                        .child(field("Safe area top", &inputs.safe_top))
                        .child(field("Safe area bottom", &inputs.safe_bottom)),
                )
                .child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .gap(px(4.))
                        .p(px(2.))
                        .rounded(px(8.))
                        .bg(rgb(BG_HEADER))
                        .child(os("os-ios", "iOS", true, cx))
                        .child(os("os-android", "Android", false, cx)),
                )
                .child(panel_section("SAVE AS DEVICE"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().flex_1().child(Input::new(&inputs.name).small()))
                        .child(
                            div()
                                .id("save-device")
                                .px(px(10.))
                                .py(px(4.))
                                .rounded(px(6.))
                                .bg(rgb(ACCENT))
                                .text_size(px(12.))
                                .hover(|s| s.opacity(0.85))
                                .child("Save")
                                .on_click(on_save),
                        ),
                );
        }

        let ppi = if d.ppi > 0.0 {
            format!("{:.0}", d.ppi)
        } else {
            "—".into()
        };
        panel.child(panel_section("SCREEN")).child(mono(format!(
            "viewport  {}×{} css\nphysical {}×{} px\ndpr       {}\nppi       {}\nsafe area {} / {}\nscale     {:.0}%",
            d.width,
            d.height,
            (d.width as f64 * d.dpr).round(),
            (d.height as f64 * d.dpr).round(),
            d.dpr,
            ppi,
            d.safe_top,
            d.safe_bottom,
            self.zoom * 100.
        )))
    }
}

// ───────────────────────── Helpers ─────────────────────────

fn dot(color: u32) -> Div {
    div().size(px(12.)).rounded_full().bg(rgb(color))
}

fn icon_button(
    id: &'static str,
    icon: Ico,
    _tooltip: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_color(rgb(TEXT_DIM))
        .hover(|s| s.bg(rgb(BORDER)).text_color(rgb(TEXT)))
        .child(Icon::new(icon).small())
        .on_click(on_click)
}

fn panel_section(label: &'static str) -> Div {
    div()
        .mt(px(14.))
        .mb(px(8.))
        .text_size(px(10.))
        .text_color(rgb(TEXT_DIM))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn panel_button(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .mt(px(14.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(rgb(BG_HEADER))
        .border_1()
        .border_color(rgb(BORDER))
        .text_size(px(12.))
        .flex()
        .justify_center()
        .hover(|s| s.bg(rgb(BORDER)))
        .child(label)
        .on_click(on_click)
}

const CAPTION_SIZE: f32 = 24.0;
const CAPTION_GAP: f32 = 2.0;

/// Window buttons on the left and on the right, like zeron: on Linux, with client
/// decorations, the side and order come from the desktop's `button-layout` and the ones
/// the compositor doesn't support are removed. Never maximize (project decision). If the
/// compositor draws the bar (server decorations), none.
#[cfg(target_os = "linux")]
pub(crate) fn caption_layout(window: &Window, cx: &App) -> [Vec<WindowButton>; 2] {
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return [vec![], vec![]];
    }
    let layout = cx
        .button_layout()
        .unwrap_or_else(WindowButtonLayout::linux_default);
    let supported = window.window_controls();
    let side = |buttons: [Option<WindowButton>; MAX_BUTTONS_PER_SIDE]| {
        buttons
            .into_iter()
            .flatten()
            .filter(|b| match b {
                WindowButton::Minimize => supported.minimize,
                WindowButton::Maximize => false,
                WindowButton::Close => true,
            })
            .collect()
    };
    [side(layout.left), side(layout.right)]
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn caption_layout(_window: &Window, _cx: &App) -> [Vec<WindowButton>; 2] {
    [vec![], vec![WindowButton::Minimize, WindowButton::Close]]
}

/// zeron-style buttons: 24 px, 6 corners, dimmed 16 px icon; close turns red.
pub(crate) fn caption_strip(
    buttons: &[WindowButton],
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(CAPTION_GAP))
        .children(buttons.iter().map(|&button| {
            let close = button == WindowButton::Close;
            let group = SharedString::from(format!("caption-{}", button.id()));
            let icon = if close { Ico::Close } else { Ico::Minimize };
            let on_close = on_close.clone();
            div()
                .id(button.id())
                .group(group.clone())
                .size(px(CAPTION_SIZE))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .hover(move |s| {
                    s.bg(rgb(if close {
                        theme::CLOSE_HOVER
                    } else {
                        theme::HOVER
                    }))
                })
                // So the click doesn't start dragging the window.
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    match button {
                        WindowButton::Close => on_close(window, cx),
                        WindowButton::Minimize => window.minimize_window(),
                        WindowButton::Maximize => window.zoom_window(),
                    }
                })
                // GPUI's SVGs don't inherit the color on hover: recolored per group.
                .child(
                    svg()
                        .path(icon.path())
                        .size(px(16.))
                        .flex_none()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .group_hover(group, move |s| {
                            s.text_color(rgb(if close { 0xffffff } else { theme::TEXT }))
                        }),
                )
        }))
}

fn status_badge(e: &NetEntry) -> Div {
    let (text, color) = match (&e.error, e.status) {
        (Some(err), _) => (format!("✕ {err}"), RED),
        (None, Some(s)) if s < 400 => (format!("✓ {s}"), GREEN),
        (None, Some(s)) => (format!("✕ {s}"), RED),
        (None, None) => ("… pending".into(), TEXT_DIM),
    };
    div()
        .text_color(rgb(color))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

fn mono(t: String) -> Div {
    div().font_family(MONO_FONT).text_size(px(12.)).child(t)
}

fn body_text(e: &NetEntry, max_chars: usize) -> String {
    match (&e.body, &e.error, e.end_ts) {
        (Some(b), _, _) => {
            let pretty = pretty_json(b);
            if pretty.chars().count() > max_chars {
                format!("{}…", pretty.chars().take(max_chars).collect::<String>())
            } else {
                pretty
            }
        }
        (None, Some(_), _) => "— (request failed)".into(),
        (None, None, None) => "Waiting for response…".into(),
        (None, None, Some(_)) => "Loading body…".into(),
    }
}

fn split_url(url: &str) -> (String, String, String, Vec<(String, String)>) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let (hostpath, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = hostpath
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((hostpath, "/".into()));
    let q = query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.to_string(), v.to_string())
        })
        .collect();
    (
        scheme.into(),
        host.into(),
        path.split('#').next().unwrap_or("").into(),
        q,
    )
}

fn short_url(url: &str) -> String {
    let (_, host, path, _) = split_url(url);
    format!("{host}{path}")
}

fn pretty_json(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| s.chars().take(200_000).collect())
}

fn fmt_size(b: f64) -> String {
    if b <= 0.0 {
        "0 KB".into()
    } else if b < 1024.0 {
        format!("{b:.0} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    }
}

fn fmt_duration(e: &NetEntry) -> String {
    e.end_ts
        .map(|end| format!("{:.0} ms", (end - e.start_ts) * 1000.0))
        .unwrap_or_else(|| "—".into())
}

fn clock(wall: f64, millis: bool) -> String {
    use chrono::{Local, TimeZone};
    let ms = (wall * 1000.0) as i64;
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) if millis => t.format("%H:%M:%S%.3f").to_string(),
        Some(t) => t.format("%H:%M").to_string(),
        None => String::new(),
    }
}

fn level_color(l: LogLevel) -> u32 {
    match l {
        LogLevel::Error => RED,
        LogLevel::Warn => YELLOW,
        LogLevel::Info => theme::INFO,
        LogLevel::Debug => theme::DEBUG,
        LogLevel::Log | LogLevel::Command => TEXT_DIM,
        LogLevel::Result => TEXT,
    }
}

fn level_label(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
        LogLevel::Log => "LOG",
        LogLevel::Command => ">",
        LogLevel::Result => "<",
    }
}

/// Translates a GPUI key into a CDP command.
fn key_command(k: &Keystroke) -> Option<Command> {
    let special = |key: &str, code: &str, vk: i64, text: Option<&str>| Command::Key {
        key: key.into(),
        code: code.into(),
        vk,
        text: text.map(Into::into),
    };
    Some(match k.key.as_str() {
        "backspace" => special("Backspace", "Backspace", 8, None),
        "delete" => special("Delete", "Delete", 46, None),
        "enter" => special("Enter", "Enter", 13, Some("\r")),
        "tab" => special("Tab", "Tab", 9, None),
        "escape" => special("Escape", "Escape", 27, None),
        "left" => special("ArrowLeft", "ArrowLeft", 37, None),
        "up" => special("ArrowUp", "ArrowUp", 38, None),
        "right" => special("ArrowRight", "ArrowRight", 39, None),
        "down" => special("ArrowDown", "ArrowDown", 40, None),
        _ => {
            if k.modifiers.control || k.modifiers.platform || k.modifiers.alt {
                return None;
            }
            let text = k
                .key_char
                .clone()
                .or_else(|| (k.key == "space").then(|| " ".into()))?;
            Command::InsertText(text)
        }
    })
}
