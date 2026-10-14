//! Floating panel tabs: Tools (web appearance, screenshots, recording)
//! and Settings (modev theme, capture folder, updates, about).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;
use crate::settings::AppTheme;

impl AppTheme {
    pub fn is_light(self, cx: &App) -> bool {
        match self {
            AppTheme::Light => true,
            AppTheme::Dark => false,
            AppTheme::Auto => matches!(
                cx.window_appearance(),
                WindowAppearance::Light | WindowAppearance::VibrantLight
            ),
        }
    }
}

/// Capture tools state (lives in `Modev`).
#[derive(Default)]
pub struct Capture {
    /// Emulated `prefers-color-scheme` of the web ("" = the system's).
    pub web_scheme: &'static str,
    /// Seconds left until the timed screenshot.
    pub countdown: Option<u32>,
    countdown_task: Option<Task<()>>,
    /// Start of the recording in progress.
    pub rec_since: Option<Instant>,
    rec_task: Option<Task<()>>,
    /// Last saved file (or error) to show in the panel.
    pub last: Option<std::result::Result<PathBuf, String>>,
    pub update_status: Option<String>,
    /// Newer release found by the update check.
    pub update: Option<crate::updater::Update>,
    /// Update installed (or, on Windows, downloaded): waiting for the restart.
    pub installed: Option<crate::updater::Installed>,
    update_task: Option<Task<()>>,
    /// Last capture scale sent to the engine.
    pub scale_sent: f64,
}

impl Modev {
    pub(super) fn set_web_scheme(&mut self, scheme: &'static str, cx: &mut Context<Self>) {
        self.capture.web_scheme = scheme;
        self.send(Command::ColorScheme(scheme));
        cx.notify();
    }

    /// Checks GitHub releases in the background. `manual`: also report "up to date"
    /// and errors (the launch check stays silent unless there is an update).
    pub(super) fn check_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.capture.update_task.is_some() {
            return;
        }
        if manual {
            self.capture.update_status = Some("Checking for updates…".into());
            cx.notify();
        }
        let check = cx.background_spawn(async { crate::updater::check() });
        self.capture.update_task = Some(cx.spawn(async move |this, cx| {
            let res = check.await;
            let _ = this.update(cx, |this, cx| {
                this.capture.update_task = None;
                let current = env!("CARGO_PKG_VERSION");
                match res {
                    Ok(Some(u)) => {
                        this.capture.update_status = Some(format!(
                            "modev {} is available (you have {current}).",
                            u.version
                        ));
                        this.capture.update = Some(u);
                    }
                    Ok(None) if manual => {
                        this.capture.update_status = Some(format!("modev {current} is up to date."))
                    }
                    Err(e) if manual => {
                        this.capture.update_status =
                            Some(format!("Could not check for updates: {e:#}"))
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("[modev] update check failed: {e:#}"),
                }
                cx.notify();
            });
        }));
    }

    /// Downloads and installs the update found by `check_updates`, then offers a restart.
    fn install_update(&mut self, cx: &mut Context<Self>) {
        let Some(update) = self.capture.update.clone() else {
            return;
        };
        if self.capture.update_task.is_some() {
            return;
        }
        self.capture.update_status = Some(format!("Downloading modev {}…", update.version));
        cx.notify();
        let install = cx.background_spawn(async move { crate::updater::install(&update) });
        self.capture.update_task = Some(cx.spawn(async move |this, cx| {
            let res = install.await;
            let _ = this.update(cx, |this, cx| {
                this.capture.update_task = None;
                match res {
                    Ok(installed) => {
                        this.capture.update_status = Some(if cfg!(windows) {
                            format!(
                                "modev {} is ready. Restart to install it.",
                                installed.version
                            )
                        } else {
                            format!(
                                "modev {} is installed. Restart to use it.",
                                installed.version
                            )
                        });
                        this.capture.update = None;
                        this.capture.installed = Some(installed);
                    }
                    Err(e) => {
                        this.capture.update_status =
                            Some(format!("Could not install the update: {e:#}"))
                    }
                }
                cx.notify();
            });
        }));
    }

    fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        let Some(installed) = &self.capture.installed else {
            return;
        };
        match crate::updater::restart(installed) {
            Ok(()) => cx.quit(),
            Err(e) => {
                self.capture.update_status = Some(format!("Could not restart: {e:#}"));
                cx.notify();
            }
        }
    }

    pub(super) fn take_screenshot(&mut self) {
        self.send(Command::Screenshot(self.settings.capture_path("png")));
    }

    /// Screenshot after `secs` seconds, with a countdown over the web.
    pub(super) fn start_countdown(&mut self, secs: u32, cx: &mut Context<Self>) {
        if secs == 0 {
            self.take_screenshot();
            return;
        }
        self.capture.countdown = Some(secs);
        self.capture.countdown_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let done = this
                    .update(cx, |this, cx| {
                        let left = this.capture.countdown.unwrap_or(1).saturating_sub(1);
                        this.capture.countdown = (left > 0).then_some(left);
                        if left == 0 {
                            this.take_screenshot();
                        }
                        cx.notify();
                        left == 0
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn cancel_countdown(&mut self, cx: &mut Context<Self>) {
        self.capture.countdown = None;
        self.capture.countdown_task = None;
        cx.notify();
    }

    pub(super) fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        if self.capture.rec_since.is_some() {
            self.send(Command::StopRecording);
            self.capture.rec_since = None;
            self.capture.rec_task = None;
        } else {
            self.send(Command::StartRecording {
                path: self.settings.capture_path("mp4"),
                touches: self.settings.record_touches,
            });
            self.capture.rec_since = Some(Instant::now());
            // Repaints every second for the counter.
            self.capture.rec_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    pub(super) fn on_screenshot(&mut self, path: PathBuf, png: Vec<u8>, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
            ImageFormat::Png,
            png,
        )));
        self.capture.last = Some(Ok(path));
    }

    pub(super) fn on_recorded(&mut self, res: std::result::Result<PathBuf, String>) {
        self.capture.rec_since = None;
        self.capture.rec_task = None;
        self.capture.last = Some(res);
    }

    /// After each render: the engine only captures the pixels being displayed.
    pub(super) fn sync_capture_scale(&mut self, window_scale: f32) {
        let scale = (window_scale * self.zoom) as f64;
        // Small threshold: any difference forces GPUI to rescale the frame.
        if (scale - self.capture.scale_sent).abs() > 0.005 {
            self.capture.scale_sent = scale;
            self.send(Command::CaptureScale(scale));
        }
    }

    pub(super) fn set_app_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        self.settings.theme = theme;
        self.settings.save();
        theme::set_light(theme.is_light(cx), cx);
        cx.notify();
    }

    pub(super) fn set_capture_dir(&mut self, dir: String) {
        let dir = dir.trim().to_string();
        if !dir.is_empty() && dir != self.settings.capture_dir {
            self.settings.capture_dir = dir;
            self.settings.save();
        }
    }

    // ───────────────────────── Tools ─────────────────────────

    pub(super) fn panel_tools(
        &mut self,
        inputs: &DeviceInputs,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let scheme = self.capture.web_scheme;
        let web_mode = |id: &'static str,
                        icon: Ico,
                        label: &'static str,
                        value: &'static str,
                        cx: &mut Context<Self>| {
            segment(id, Some(icon), label, scheme == value)
                .on_click(cx.listener(move |this, _, _, cx| this.set_web_scheme(value, cx)))
        };

        let countdown = self.capture.countdown;
        let timer = |id: &'static str, label: &'static str, secs: u32, cx: &mut Context<Self>| {
            segment(id, None, label, false)
                .on_click(cx.listener(move |this, _, _, cx| this.start_countdown(secs, cx)))
        };
        let custom = inputs.timer.clone();
        let touches = self.settings.record_touches;

        let recording = self.capture.rec_since.map(|t| t.elapsed().as_secs());
        let rec_label = match recording {
            Some(s) => format!("Stop  {:02}:{:02}", s / 60, s % 60),
            None => "Record video".into(),
        };

        let mut panel = div()
            .flex()
            .flex_col()
            .child(panel_section("WEB APPEARANCE"))
            .child(
                segmented()
                    .child(web_mode("ws-auto", Ico::Smartphone, "System", "", cx))
                    .child(web_mode("ws-light", Ico::Sun, "Light", "light", cx))
                    .child(web_mode("ws-dark", Ico::Moon, "Dark", "dark", cx)),
            )
            .child(hint("Emulates prefers-color-scheme, like Chrome DevTools."))
            .child(panel_section("SCREENSHOT"))
            .child(
                action_row("shot", Ico::Camera, "Screenshot", false).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.take_screenshot();
                        cx.notify();
                    },
                )),
            )
            .child(panel_section("TIMED SCREENSHOT"))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(timer("t-3", "3 s", 3, cx))
                    .child(timer("t-5", "5 s", 5, cx))
                    .child(div().w(px(56.)).child(Input::new(&inputs.timer).small()))
                    .child(
                        segment("t-custom", Some(Ico::Timer), "Start", false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                let secs = custom
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .parse::<u32>()
                                    .unwrap_or(0)
                                    .min(600);
                                this.start_countdown(secs, cx);
                            }),
                        ),
                    ),
            )
            .when_some(countdown, |p, n| {
                p.child(
                    div()
                        .mt(px(6.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT_DIM))
                        .child(format!("Screenshot in {n} s…"))
                        .child(
                            div()
                                .id("t-cancel")
                                .px(px(8.))
                                .rounded(px(6.))
                                .hover(|s| s.bg(rgb(BORDER)).text_color(rgb(TEXT)))
                                .child("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_countdown(cx))),
                        ),
                )
            })
            .child(panel_section("RECORDING"))
            .child(
                action_row(
                    "rec",
                    if recording.is_some() {
                        Ico::Stop
                    } else {
                        Ico::Record
                    },
                    rec_label,
                    recording.is_some(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_recording(cx))),
            )
            .child(
                toggle_row("rec-touches", "Show touches and clicks", touches)
                    .mt(px(6.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.record_touches = !this.settings.record_touches;
                        this.settings.save();
                        cx.notify();
                    })),
            )
            .child(hint(
                "MP4 at 30 fps with ffmpeg; every screenshot is copied to the clipboard.",
            ));

        if let Some(last) = &self.capture.last {
            let (text, color, dir) = match last {
                Ok(p) => (
                    format!(
                        "Saved: {}",
                        p.file_name()
                            .map(|f| f.to_string_lossy())
                            .unwrap_or_default()
                    ),
                    GREEN,
                    p.parent().map(|d| d.to_path_buf()),
                ),
                Err(e) => (e.clone(), RED, None),
            };
            panel = panel.child(
                div()
                    .mt(px(12.))
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(rgb(BG_HEADER))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .child(div().text_color(rgb(color)).child(text))
                    .when_some(dir, |d, dir| {
                        d.child(
                            div()
                                .id("open-captures")
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .text_color(rgb(TEXT_DIM))
                                .hover(|s| s.text_color(rgb(TEXT)))
                                .child(Icon::new(Ico::Folder).xsmall())
                                .child("Open folder")
                                .on_click(move |_, _, cx| cx.open_with_system(&dir)),
                        )
                    }),
            );
        }
        panel
    }

    // ───────────────────────── Settings ─────────────────────────

    pub(super) fn panel_settings(
        &mut self,
        inputs: &DeviceInputs,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = self.settings.theme;
        let theme = |id: &'static str,
                     icon: Ico,
                     label: &'static str,
                     t: AppTheme,
                     cx: &mut Context<Self>| {
            segment(id, Some(icon), label, current == t)
                .on_click(cx.listener(move |this, _, _, cx| this.set_app_theme(t, cx)))
        };
        let dir = PathBuf::from(&self.settings.capture_dir);
        let remember = self.settings.remember_device;
        let auto_update = self.settings.check_updates;

        div()
            .flex()
            .flex_col()
            .child(panel_section("APPEARANCE"))
            .child(
                segmented()
                    .child(theme("th-auto", Ico::Sliders, "Auto", AppTheme::Auto, cx))
                    .child(theme("th-dark", Ico::Moon, "Dark", AppTheme::Dark, cx))
                    .child(theme("th-light", Ico::Sun, "Light", AppTheme::Light, cx)),
            )
            .child(panel_section("SCREENSHOTS & RECORDINGS"))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&inputs.capture_dir).small()),
                    )
                    .child(
                        segment("dir-open", Some(Ico::Folder), "Open", false).on_click(
                            move |_, _, cx| {
                                let _ = std::fs::create_dir_all(&dir);
                                cx.open_with_system(&dir);
                            },
                        ),
                    ),
            )
            .child(panel_section("GENERAL"))
            .child(
                toggle_row("remember", "Remember last device", remember).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.settings.remember_device = !this.settings.remember_device;
                        this.settings.last_device = this
                            .settings
                            .remember_device
                            .then(|| this.device.name.clone());
                        this.settings.save();
                        cx.notify();
                    },
                )),
            )
            .child(panel_section("UPDATES"))
            .child(
                toggle_row("auto-update", "Check for updates on launch", auto_update).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.settings.check_updates = !this.settings.check_updates;
                        this.settings.save();
                        cx.notify();
                    }),
                ),
            )
            .child(
                action_row("check-update", Ico::Refresh, "Check for updates", false)
                    .mt(px(6.))
                    .on_click(cx.listener(|this, _, _, cx| this.check_updates(true, cx))),
            )
            .when_some(self.capture.update_status.clone(), |d, s| {
                d.child(hint_owned(s))
            })
            .when(self.capture.installed.is_some(), |d| {
                d.child(
                    action_row("update-restart", Ico::Refresh, "Restart now", false)
                        .mt(px(6.))
                        .on_click(cx.listener(|this, _, _, cx| this.restart_to_update(cx))),
                )
            })
            .when_some(self.capture.update.clone(), |d, u| {
                let notes = u.notes.clone();
                // Without a package for this installation, the release page has the downloads.
                let busy = self.capture.update_task.is_some();
                d.when(u.package.is_some() && !busy, |d| {
                    d.child(
                        action_row(
                            "update-install",
                            Ico::Download,
                            format!("Install {}", u.version),
                            false,
                        )
                        .mt(px(6.))
                        .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
                    )
                })
                .child(
                    action_row("update-notes", Ico::Globe, "Release notes", false)
                        .mt(px(6.))
                        .on_click(move |_, _, cx| cx.open_url(&notes)),
                )
            })
            .child(panel_section("ABOUT"))
            .child(
                div()
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(BG_HEADER))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(Icon::new(Ico::Smartphone).small())
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("modev"))
                            .child(
                                div()
                                    .text_color(rgb(TEXT_DIM))
                                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(TEXT_DIM))
                            .child("Device simulator for mobile web apps with a native inspector."),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(TEXT_DIM))
                            .child("Chrome headless (CDP) · GPUI"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(TEXT_DIM))
                            .child("Icons: Hugeicons (MIT)"),
                    ),
            )
    }
}

/// Group of exclusive buttons (like the responsive mode's OS selector).
fn segmented() -> Div {
    div()
        .flex()
        .gap(px(4.))
        .p(px(2.))
        .rounded(px(8.))
        .bg(rgb(BG_HEADER))
}

fn segment(
    id: &'static str,
    icon: Option<Ico>,
    label: &'static str,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .px(px(6.))
        .py(px(4.))
        .rounded(px(6.))
        .text_size(px(12.))
        .when(active, |s| s.bg(rgb(ACCENT)))
        .when(!active, |s| {
            s.bg(rgb(BG_HEADER)).hover(|s| s.bg(rgb(BORDER)))
        })
        .when_some(icon, |s, i| s.child(Icon::new(i).xsmall()))
        .child(label)
}

fn action_row(
    id: &'static str,
    icon: Ico,
    label: impl Into<SharedString>,
    danger: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .py(px(7.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(if danger { RED } else { BORDER }))
        .bg(rgb(BG_HEADER))
        .text_size(px(12.))
        .when(danger, |s| s.text_color(rgb(RED)))
        .hover(|s| s.bg(rgb(BORDER)))
        .child(Icon::new(icon).small())
        .child(label.into())
}

fn toggle_row(id: &'static str, label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .py(px(4.))
        .text_size(px(12.))
        .child(label)
        .child(
            div()
                .w(px(30.))
                .h(px(18.))
                .p(px(2.))
                .flex()
                .when(on, |s| s.justify_end())
                .rounded_full()
                .bg(rgb(if on { ACCENT } else { BORDER }))
                .child(div().size(px(14.)).rounded_full().bg(rgb(0xffffff))),
        )
}

fn hint(text: &'static str) -> Div {
    hint_owned(text.into())
}

fn hint_owned(text: String) -> Div {
    div()
        .mt(px(6.))
        .text_size(px(11.))
        .text_color(rgb(TEXT_DIM))
        .child(text)
}
