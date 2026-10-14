//! Android tab of the floating panel, like chrome://inspect/#devices: devices connected
//! over adb and their pages (Chrome tabs, debuggable WebViews). "Inspect" moves the screen
//! and the inspector to one of them; "Back" returns to the emulated device.

use std::collections::HashSet;

use super::*;
use crate::adb;

/// Android state (lives in `Modev`).
pub struct Android {
    pub snapshot: adb::Snapshot,
    /// The adb thread lists the pages only while the tab is open.
    active: tokio::sync::watch::Sender<bool>,
    /// A device came online while the tab was closed (dot on the tab).
    pub new_device: bool,
    /// Page being connected: target id and label.
    connecting: Option<(String, String)>,
    /// Why the last "Inspect" failed or the session ended.
    error: Option<String>,
    pub session: Option<Session>,
    _task: Task<()>,
}

/// Android page being inspected and what to restore when leaving it.
pub struct Session {
    target: String,
    /// The page's URL when attaching: "Home" goes back to it.
    pub home: String,
    local_device: DeviceProfile,
    local_page: (String, String, Option<Arc<Image>>),
}

impl Android {
    pub fn new(cx: &mut Context<Modev>) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (active, active_rx) = tokio::sync::watch::channel(false);
        adb::spawn(tx, active_rx);
        let task = cx.spawn(async move |this, cx| {
            while let Some(snap) = rx.recv().await {
                if this.update(cx, |m, cx| m.on_adb(snap, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            snapshot: adb::Snapshot::default(),
            active,
            new_device: false,
            connecting: None,
            error: None,
            session: None,
            _task: task,
        }
    }
}

impl Modev {
    pub(super) fn android_visible(&mut self, visible: bool) {
        self.android.active.send_replace(visible);
        if visible {
            self.android.new_device = false;
        }
    }

    fn on_adb(&mut self, snap: adb::Snapshot, cx: &mut Context<Self>) {
        let online = |s: &adb::Snapshot| -> HashSet<String> {
            s.devices
                .iter()
                .filter(|d| d.state == "device")
                .map(|d| d.serial.clone())
                .collect()
        };
        let before = online(&self.android.snapshot);
        if self.panel_tab != PanelTab::Android && online(&snap).iter().any(|s| !before.contains(s))
        {
            self.android.new_device = true;
        }
        self.android.snapshot = snap;
        cx.notify();
    }

    fn inspect(&mut self, ws: String, target: String, label: String) {
        self.android.error = None;
        self.android.connecting = Some((target.clone(), label));
        self.send(Command::Attach { ws, target });
    }

    /// An "Inspect" failed (the engine reports it as a status).
    pub(super) fn android_status(&mut self, status: &str) {
        if self.android.connecting.take().is_some() {
            self.android.error = Some(status.to_string());
        }
    }

    /// Leaves the Android page; `next`: emulated device to use afterwards.
    pub(super) fn detach(&mut self, next: Option<DeviceProfile>) {
        if let Some(s) = &mut self.android.session {
            if let Some(d) = next {
                s.local_device = d;
            }
            self.send(Command::Detach);
        }
    }

    pub(super) fn on_attached(
        &mut self,
        width: u32,
        height: u32,
        dpr: f64,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let (target, label) = self
            .android
            .connecting
            .take()
            .unwrap_or_else(|| (String::new(), "Android".into()));
        let (local_device, local_page) = match self.android.session.take() {
            Some(s) => (s.local_device, s.local_page),
            None => (
                self.device.clone(),
                (
                    self.page_url.clone(),
                    self.page_title.clone(),
                    self.favicon.clone(),
                ),
            ),
        };
        self.android.session = Some(Session {
            target,
            home: url.clone(),
            local_device,
            local_page,
        });
        self.device = DeviceProfile {
            name: label,
            os_label: "Android (USB)".into(),
            width,
            height,
            dpr,
            ppi: 0.0,
            safe_top: 0,
            safe_bottom: 0,
            ..self.device.clone()
        };
        self.switch_screen(cx);
        self.page_url = url;
        self.page_title.clear();
        self.favicon = None;
    }

    pub(super) fn on_detached(&mut self, reason: Option<String>, cx: &mut Context<Self>) {
        if let Some(s) = self.android.session.take() {
            self.device = s.local_device;
            self.switch_screen(cx);
            (self.page_url, self.page_title, self.favicon) = s.local_page;
        }
        self.android.error = reason;
    }

    /// Rotation or keyboard on the device.
    pub(super) fn on_viewport(&mut self, width: u32, height: u32) {
        if self.android.session.is_some()
            && width > 0
            && height > 0
            && (width, height) != (self.device.width, self.device.height)
        {
            self.device.width = width;
            self.device.height = height;
            self.zoom = fit_zoom(&self.device, self.display_height);
        }
    }

    /// Another screen: new zoom, no frame of the previous one and empty Network/Console.
    fn switch_screen(&mut self, cx: &mut Context<Self>) {
        self.zoom = fit_zoom(&self.device, self.display_height);
        self.screen.update(cx, |s, _| {
            if let Some(old) = s.frame.take() {
                s.stale.push(old);
            }
        });
        self.clear();
    }

    pub(super) fn panel_android(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = |text: String| {
            div()
                .text_size(px(11.))
                .text_color(rgb(TEXT_DIM))
                .child(text)
        };
        let mut panel = div().flex().flex_col();

        if let Some(s) = &self.android.session {
            panel = panel
                .child(panel_section("INSPECTING"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .truncate()
                                .child(self.device.name.clone()),
                        )
                        .child(hint(s.home.clone()).truncate()),
                )
                .child(panel_button(
                    "android-back",
                    "Back to the simulator",
                    cx.listener(|this, _, _, cx| {
                        this.detach(None);
                        cx.notify();
                    }),
                ));
        }
        if let Some(e) = &self.android.error {
            panel = panel.child(
                div()
                    .mt(px(10.))
                    .text_size(px(11.))
                    .text_color(rgb(RED))
                    .child(e.clone()),
            );
        }

        panel = panel.child(panel_section("DEVICES"));
        let snap = &self.android.snapshot;
        if !snap.error.is_empty() {
            return panel.child(hint(snap.error.clone()));
        }
        if snap.devices.is_empty() {
            return panel.child(hint(
                "No devices. Connect a phone with USB debugging enabled.".into(),
            ));
        }

        let attached = self.android.session.as_ref().map(|s| s.target.clone());
        let connecting = self.android.connecting.as_ref().map(|c| c.0.clone());
        let mut list = div()
            .id("android-list")
            .max_h(px(460.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(12.));
        let mut n = 0usize;
        for dev in &snap.devices {
            let ready = dev.state == "device";
            let mut item = div().flex().flex_col().gap(px(6.)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(dot(if ready { GREEN } else { YELLOW }).size(px(8.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(dev.model.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(TEXT_DIM))
                            .child(dev.serial.clone()),
                    ),
            );
            if !ready {
                let why = if dev.state == "unauthorized" {
                    "Accept the \"Allow USB debugging\" prompt on the phone.".to_string()
                } else if dev.state.starts_with("no permissions") {
                    "No USB permissions: install the udev rules (android-udev-rules) and reconnect."
                        .to_string()
                } else {
                    format!("Device {}: reconnect the cable.", dev.state)
                };
                list = list.child(item.child(hint(why)));
                continue;
            }
            if dev.browsers.is_empty() {
                item = item.child(hint(
                    "No inspectable pages. Open Chrome or a debug build of the app (WebView debugging enabled)."
                        .into(),
                ));
            }
            for b in &dev.browsers {
                item = item.child(
                    div()
                        .mt(px(4.))
                        .flex()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .child(div().flex_1().min_w_0().truncate().child(b.name.clone()))
                        .child(div().text_color(rgb(TEXT_DIM)).child(b.version.clone())),
                );
                if b.targets.is_empty() {
                    item = item.child(hint("No pages".into()));
                }
                for t in &b.targets {
                    n += 1;
                    let is_attached = attached.as_deref() == Some(t.id.as_str());
                    let is_connecting = connecting.as_deref() == Some(t.id.as_str());
                    let (ws, id) = (b.ws.clone(), t.id.clone());
                    let label = format!("{} · {}", dev.model, b.name);
                    let button = div()
                        .id(("android-inspect", n))
                        .px(px(10.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .text_size(px(11.))
                        .bg(rgb(if is_attached { BG_PANEL } else { ACCENT }))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .hover(|s| s.opacity(0.85))
                        .child(if is_attached {
                            "Back"
                        } else if is_connecting {
                            "Connecting…"
                        } else {
                            "Inspect"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if is_attached {
                                this.detach(None);
                            } else {
                                this.inspect(ws.clone(), id.clone(), label.clone());
                            }
                            cx.notify();
                        }));
                    let title = if t.title.is_empty() {
                        t.url.clone()
                    } else {
                        t.title.clone()
                    };
                    item = item.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .p(px(8.))
                            .rounded(px(8.))
                            .bg(rgb(BG_HEADER))
                            .when(is_attached, |s| s.border_1().border_color(rgb(ACCENT)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(div().text_size(px(12.)).truncate().child(title))
                                    .child(hint(t.url.clone()).truncate())
                                    .when(!t.visible, |s| s.child(hint("hidden".into()))),
                            )
                            .child(button),
                    );
                }
            }
            list = list.child(item);
        }
        panel.child(list)
    }
}
