//! Native inspector with the look of Chrome DevTools: Console and Network tabs.

use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;

use super::net_tools::{self, Jwt};
use super::selectable::{SelDoc, SelText};
use super::*;
use gpui_kit::base::{TextSelection, TextSelectionLayer};

actions!(inspector, [CopySelection]);

/// Ctrl+C / Cmd+C copies the mouse-selected text (without gpui-kit's Root).
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-c", CopySelection, Some("Inspector")),
        KeyBinding::new("cmd-c", CopySelection, Some("Inspector")),
    ]);
}

mod application;
pub use application::AppState;

// Chrome DevTools layout with zeron's palette (see `theme.rs`).
const DT_BG: u32 = theme::BG;
const DT_TOOLBAR: u32 = theme::SURFACE;
const DT_BORDER: u32 = theme::BORDER;
const DT_TEXT: u32 = theme::TEXT;
const DT_DIM: u32 = theme::TEXT_MUTED;
const DT_BLUE: u32 = theme::ACCENT;
const DT_ROW_ALT: u32 = theme::ROW_ALT;
const DT_HOVER: u32 = theme::HOVER;
const DT_SELECTED: u32 = theme::ACCENT_WASH;
const DT_RED: u32 = theme::DANGER;
const DT_YELLOW: u32 = theme::WARNING;
const DT_GREEN: u32 = theme::SUCCESS;
const ERR_BG: u32 = theme::DANGER_BG;
const ERR_BORDER: u32 = theme::DANGER_BORDER;
const WARN_BG: u32 = theme::WARNING_BG;
const WARN_BORDER: u32 = theme::WARNING_BORDER;

const WATERFALL_W: f32 = 180.0;
/// Minimum width of "Name" (takes what is left over).
const NAME_MIN_W: f32 = 180.0;

#[derive(Clone, Copy, PartialEq)]
pub enum DevTab {
    Console,
    Network,
    Application,
}

#[derive(Clone, Copy, PartialEq)]
pub enum NetFilter {
    All,
    FetchXhr,
    Doc,
    Css,
    Js,
    Font,
    Img,
    Media,
    Ws,
    Other,
}

impl NetFilter {
    const ALL: [NetFilter; 10] = [
        NetFilter::All,
        NetFilter::FetchXhr,
        NetFilter::Doc,
        NetFilter::Css,
        NetFilter::Js,
        NetFilter::Font,
        NetFilter::Img,
        NetFilter::Media,
        NetFilter::Ws,
        NetFilter::Other,
    ];

    fn label(self) -> &'static str {
        match self {
            NetFilter::All => "All",
            NetFilter::FetchXhr => "Fetch/XHR",
            NetFilter::Doc => "Doc",
            NetFilter::Css => "CSS",
            NetFilter::Js => "JS",
            NetFilter::Font => "Font",
            NetFilter::Img => "Img",
            NetFilter::Media => "Media",
            NetFilter::Ws => "WS",
            NetFilter::Other => "Other",
        }
    }

    fn matches(self, resource_type: &str) -> bool {
        match self {
            NetFilter::All => true,
            NetFilter::FetchXhr => matches!(resource_type, "Fetch" | "Xhr"),
            NetFilter::Doc => resource_type == "Document",
            NetFilter::Css => resource_type == "Stylesheet",
            NetFilter::Js => resource_type == "Script",
            NetFilter::Font => resource_type == "Font",
            NetFilter::Img => resource_type == "Image",
            NetFilter::Media => resource_type == "Media",
            NetFilter::Ws => resource_type == "WebSocket",
            NetFilter::Other => !matches!(
                resource_type,
                "Fetch"
                    | "Xhr"
                    | "Document"
                    | "Stylesheet"
                    | "Script"
                    | "Font"
                    | "Image"
                    | "Media"
                    | "WebSocket"
            ),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum NetDetailTab {
    Headers,
    Payload,
    Preview,
    Response,
    Timing,
    /// Decoded JWT tokens (only appears if the request carries one).
    Jwt,
    /// Editor to replay the (possibly modified) request.
    Replay,
}

/// Network table columns: Chrome's and Firefox's. "Name" is always visible.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum NetColumn {
    Name,
    Status,
    Method,
    Domain,
    Path,
    Url,
    Protocol,
    Scheme,
    RemoteAddr,
    Initiator,
    Type,
    MimeType,
    Cookies,
    SetCookies,
    Transferred,
    Size,
    Priority,
    StartTime,
    Time,
    Waterfall,
}

impl NetColumn {
    pub const ALL: [NetColumn; 20] = [
        NetColumn::Name,
        NetColumn::Status,
        NetColumn::Method,
        NetColumn::Domain,
        NetColumn::Path,
        NetColumn::Url,
        NetColumn::Protocol,
        NetColumn::Scheme,
        NetColumn::RemoteAddr,
        NetColumn::Initiator,
        NetColumn::Type,
        NetColumn::MimeType,
        NetColumn::Cookies,
        NetColumn::SetCookies,
        NetColumn::Transferred,
        NetColumn::Size,
        NetColumn::Priority,
        NetColumn::StartTime,
        NetColumn::Time,
        NetColumn::Waterfall,
    ];

    const DEFAULT: [NetColumn; 9] = [
        NetColumn::Name,
        NetColumn::Status,
        NetColumn::Method,
        NetColumn::Domain,
        NetColumn::Type,
        NetColumn::Initiator,
        NetColumn::Transferred,
        NetColumn::Time,
        NetColumn::Waterfall,
    ];

    fn label(self) -> &'static str {
        match self {
            NetColumn::Name => "Name",
            NetColumn::Status => "Status",
            NetColumn::Method => "Method",
            NetColumn::Domain => "Domain",
            NetColumn::Path => "Path",
            NetColumn::Url => "URL",
            NetColumn::Protocol => "Protocol",
            NetColumn::Scheme => "Scheme",
            NetColumn::RemoteAddr => "Remote Address",
            NetColumn::Initiator => "Initiator",
            NetColumn::Type => "Type",
            NetColumn::MimeType => "Content-Type",
            NetColumn::Cookies => "Cookies",
            NetColumn::SetCookies => "Set-Cookies",
            NetColumn::Transferred => "Transferred",
            NetColumn::Size => "Size",
            NetColumn::Priority => "Priority",
            NetColumn::StartTime => "Start Time",
            NetColumn::Time => "Time",
            NetColumn::Waterfall => "Waterfall",
        }
    }

    /// Fixed width; `Name` takes the rest (with a minimum).
    fn width(self) -> f32 {
        match self {
            NetColumn::Name => NAME_MIN_W,
            NetColumn::Status => 60.,
            NetColumn::Method => 70.,
            NetColumn::Domain => 160.,
            NetColumn::Path => 200.,
            NetColumn::Url => 280.,
            NetColumn::Protocol => 80.,
            NetColumn::Scheme => 70.,
            NetColumn::RemoteAddr => 150.,
            NetColumn::Initiator => 130.,
            NetColumn::Type => 80.,
            NetColumn::MimeType => 150.,
            NetColumn::Cookies => 70.,
            NetColumn::SetCookies => 90.,
            NetColumn::Transferred => 90.,
            NetColumn::Size => 80.,
            NetColumn::Priority => 80.,
            NetColumn::StartTime => 100.,
            NetColumn::Time => 80.,
            NetColumn::Waterfall => WATERFALL_W + 12.,
        }
    }

    fn numeric(self) -> bool {
        matches!(
            self,
            NetColumn::Status
                | NetColumn::Cookies
                | NetColumn::SetCookies
                | NetColumn::Transferred
                | NetColumn::Size
                | NetColumn::StartTime
                | NetColumn::Time
        )
    }

    fn text(self, e: &NetEntry) -> String {
        let (scheme, host, path, _) = split_url(&e.url);
        match self {
            NetColumn::Name => request_name(&e.url),
            NetColumn::Status => status_text(e),
            NetColumn::Method => e.method.clone(),
            NetColumn::Domain => host,
            NetColumn::Path => path,
            NetColumn::Url => e.url.clone(),
            NetColumn::Protocol => e.protocol.clone().unwrap_or_default(),
            NetColumn::Scheme => scheme,
            NetColumn::RemoteAddr => e.remote_addr.clone().unwrap_or_default(),
            NetColumn::Initiator => e.initiator.clone(),
            NetColumn::Type => type_label(&e.resource_type),
            NetColumn::MimeType => e.mime.clone(),
            NetColumn::Cookies => count(e.cookies.len()),
            NetColumn::SetCookies => count(e.set_cookies.len()),
            NetColumn::Transferred => fmt_size(e.size),
            NetColumn::Size => fmt_size(e.resource_size),
            NetColumn::Priority => e.priority.clone(),
            NetColumn::StartTime => clock(e.wall_time, true),
            NetColumn::Time => fmt_duration(e),
            NetColumn::Waterfall => String::new(),
        }
    }

    /// Sort key: numbers as numbers, the rest as text.
    fn sort_key(self, e: &NetEntry) -> (f64, String) {
        let n = match self {
            NetColumn::Status => e.status.map_or(-1.0, |s| s as f64),
            NetColumn::Cookies => e.cookies.len() as f64,
            NetColumn::SetCookies => e.set_cookies.len() as f64,
            NetColumn::Transferred => e.size,
            NetColumn::Size => e.resource_size,
            NetColumn::StartTime | NetColumn::Waterfall => e.start_ts,
            NetColumn::Time => e.end_ts.map_or(f64::MAX, |end| end - e.start_ts),
            _ => 0.0,
        };
        (
            n,
            if self.numeric() || self == NetColumn::Waterfall {
                String::new()
            } else {
                self.text(e).to_lowercase()
            },
        )
    }

    pub fn load() -> Vec<NetColumn> {
        let saved: Option<Vec<NetColumn>> = device::config_file("columns.json")
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok());
        let mut cols = saved.unwrap_or_else(|| Self::DEFAULT.to_vec());
        if !cols.contains(&NetColumn::Name) {
            cols.insert(0, NetColumn::Name);
        }
        cols
    }

    fn save(cols: &[NetColumn]) {
        let Some(path) = device::config_file("columns.json") else {
            return;
        };
        let res = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&path, serde_json::to_vec(cols).unwrap_or_default()));
        if let Err(e) = res {
            eprintln!("[modev] could not save columns.json: {e}");
        }
    }
}

fn count(n: usize) -> String {
    if n == 0 { String::new() } else { n.to_string() }
}

/// Levels visible in the console (like the "Default levels" dropdown).
#[derive(Clone, Copy)]
pub struct Levels {
    pub verbose: bool,
    pub info: bool,
    pub warnings: bool,
    pub errors: bool,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            verbose: false,
            info: true,
            warnings: true,
            errors: true,
        }
    }
}

impl Levels {
    fn shows(&self, l: LogLevel) -> bool {
        match l {
            LogLevel::Debug => self.verbose,
            LogLevel::Log | LogLevel::Info | LogLevel::Command | LogLevel::Result => self.info,
            LogLevel::Warn => self.warnings,
            LogLevel::Error => self.errors,
        }
    }
}

/// Inspector text fields. They live in the inspector window (they need its `Window`).
#[derive(Clone)]
pub struct Inputs {
    net_query: Entity<InputState>,
    console_query: Entity<InputState>,
    console_input: Entity<InputState>,
    /// Search in a request's detail.
    detail_query: Entity<InputState>,
    replay_method: Entity<InputState>,
    replay_url: Entity<InputState>,
    replay_headers: Entity<EditorState>,
    replay_body: Entity<EditorState>,
    /// Application: filter, inline editing and IndexedDB's "Start from key".
    app_filter: Entity<InputState>,
    app_edit: Entity<InputState>,
    idb_start: Entity<InputState>,
}

/// Inspector (Console/Network) window. Opened on demand; the state lives in `Modev`.
pub struct InspectorView {
    state: Entity<Modev>,
    inputs: Inputs,
    _subscriptions: Vec<Subscription>,
}

impl InspectorView {
    pub fn new(state: Entity<Modev>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let inputs = Inputs {
            net_query: cx.new(|cx| InputState::new(window, cx).placeholder("Filter")),
            console_query: cx.new(|cx| InputState::new(window, cx).placeholder("Filter")),
            console_input: cx.new(|cx| InputState::new(window, cx)),
            detail_query: cx.new(|cx| InputState::new(window, cx).placeholder("Search")),
            replay_method: cx.new(|cx| InputState::new(window, cx).placeholder("GET")),
            replay_url: cx.new(|cx| InputState::new(window, cx).placeholder("https://…")),
            replay_headers: cx.new(|cx| {
                EditorState::new(window, cx)
                    .line_number(false)
                    .placeholder("Header: value")
            }),
            replay_body: cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("json")
                    .placeholder("Body")
            }),
            app_filter: cx.new(|cx| InputState::new(window, cx).placeholder("Filter")),
            app_edit: cx.new(|cx| InputState::new(window, cx)),
            idb_start: cx.new(|cx| InputState::new(window, cx).placeholder("Start from key")),
        };
        let mut subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.observe_button_layout_changed(window, |_, _, cx| cx.notify()),
        ];
        for q in [
            &inputs.net_query,
            &inputs.console_query,
            &inputs.detail_query,
            &inputs.app_filter,
        ] {
            subscriptions.push(cx.subscribe(q, |_, _, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    cx.notify();
                }
            }));
        }
        // Enter in the console prompt: evaluate in the app and clear.
        subscriptions.push(cx.subscribe_in(
            &inputs.console_input,
            window,
            |this, input, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let expr = input.read(cx).value().trim().to_string();
                    if !expr.is_empty() {
                        this.state
                            .update(cx, |m, _| m.send(Command::Evaluate(expr)));
                        input.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                }
            },
        ));
        // Application inline editing: Enter or losing focus saves (like Chrome).
        subscriptions.push(
            cx.subscribe(&inputs.app_edit, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let value = input.read(cx).value().to_string();
                    this.state.update(cx, |m, cx| {
                        m.app_commit_edit(value);
                        cx.notify();
                    });
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&inputs.idb_start, |this, input, ev: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let start = input.read(cx).value().to_string();
                    this.state.update(cx, |m, cx| {
                        m.app_start_from_key(start);
                        cx.notify();
                    });
                }
            }),
        );
        Self {
            state,
            inputs,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for InspectorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let inputs = self.inputs.clone();
        // When "Replay" is opened on another request, its data is loaded into the editor.
        if let Some(e) = self.state.update(cx, |m, _| m.replay_to_load()) {
            let headers = e
                .request_headers
                .iter()
                .filter(|(k, _)| !k.starts_with(':'))
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n");
            let body = e.post_data.as_deref().map(pretty_json).unwrap_or_default();
            let i = &self.inputs;
            i.replay_method
                .update(cx, |f, cx| f.set_value(e.method.clone(), window, cx));
            i.replay_url
                .update(cx, |f, cx| f.set_value(e.url.clone(), window, cx));
            i.replay_headers
                .update(cx, |f, cx| f.set_value(headers, window, cx));
            i.replay_body
                .update(cx, |f, cx| f.set_value(body, window, cx));
        }
        if let Some(v) = self.state.update(cx, |m, _| m.app_edit_to_load()) {
            self.inputs.app_edit.update(cx, |f, cx| {
                f.set_value(v, window, cx);
                f.focus(window, cx);
            });
        }
        let captions = caption_layout(window, cx);
        let content = self.state.update(cx, |m, cx| {
            m.render_devtools(&inputs, captions, cx).into_any_element()
        });
        // Same panel as the simulator: radius, border and background. The content goes 3 px in
        // so its rectangular backgrounds don't stick out of the rounded corners
        // (radius × (1 − 1/√2) ≈ 2.9 px).
        let panel = div()
            .size_full()
            .key_context("Inspector")
            .on_action(|_: &CopySelection, window, cx| {
                let text = TextSelection::selected_text(window, cx).trim().to_string();
                if text.is_empty() {
                    cx.propagate();
                } else {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            })
            // When starting to select text, release the fields' focus: otherwise Ctrl+C would
            // be handled by the focused field instead of the selection.
            .on_mouse_down(MouseButton::Left, |ev, window, cx| {
                if TextSelection::is_selectable_at(ev.position, window, cx) {
                    window.blur(cx);
                }
            })
            .child(TextSelectionLayer)
            .p(px(3.))
            .rounded(px(PANEL_RADIUS))
            .bg(rgb(BG_PANEL))
            .border_1()
            .border_color(rgb(BORDER))
            .overflow_hidden()
            .child(content);
        resize_frame(
            div()
                .size_full()
                .p(px(SIM_PAD))
                .child(panel)
                .into_any_element(),
        )
    }
}

/// Window without system decorations: invisible borders to resize it.
fn resize_frame(content: AnyElement) -> Div {
    const E: f32 = 5.0;
    const C: f32 = 10.0;
    let edge = |edge: ResizeEdge, cursor: CursorStyle| {
        div()
            .absolute()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                window.start_window_resize(edge)
            })
    };
    div()
        .relative()
        .size_full()
        .child(content)
        .child(
            edge(ResizeEdge::Top, CursorStyle::ResizeUpDown)
                .top_0()
                .left_0()
                .w_full()
                .h(px(E)),
        )
        .child(
            edge(ResizeEdge::Bottom, CursorStyle::ResizeUpDown)
                .bottom_0()
                .left_0()
                .w_full()
                .h(px(E)),
        )
        .child(
            edge(ResizeEdge::Left, CursorStyle::ResizeLeftRight)
                .top_0()
                .left_0()
                .h_full()
                .w(px(E)),
        )
        .child(
            edge(ResizeEdge::Right, CursorStyle::ResizeLeftRight)
                .top_0()
                .right_0()
                .h_full()
                .w(px(E)),
        )
        .child(
            edge(ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight)
                .top_0()
                .left_0()
                .size(px(C)),
        )
        .child(
            edge(ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight)
                .bottom_0()
                .right_0()
                .size(px(C)),
        )
        .child(
            edge(ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft)
                .top_0()
                .right_0()
                .size(px(C)),
        )
        .child(
            edge(ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft)
                .bottom_0()
                .left_0()
                .size(px(C)),
        )
}

impl Modev {
    pub(super) fn render_devtools(
        &mut self,
        inputs: &Inputs,
        [left_caps, right_caps]: [Vec<WindowButton>; 2],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let close = |window: &mut Window, _: &mut App| window.remove_window();
        let tab = |id: &'static str, label: &'static str, t: DevTab, cx: &mut Context<Self>| {
            let active = self.dev_tab == t;
            div()
                .id(id)
                .h_full()
                .px(px(12.))
                .flex()
                .items_center()
                .border_b_2()
                .border_color(if active {
                    rgb(DT_BLUE)
                } else {
                    rgba(0x00000000)
                })
                .text_color(rgb(if active { DT_TEXT } else { DT_DIM }))
                .hover(|s| s.bg(rgb(DT_HOVER)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dev_tab = t;
                    if t == DevTab::Application {
                        this.app_reload();
                    }
                    cx.notify();
                }))
        };

        let strip = div()
            .h(px(30.))
            .flex()
            .items_center()
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .when(!left_caps.is_empty(), |s| {
                s.child(caption_strip(&left_caps, close).px(px(6.)))
            })
            .child(tab("dt-console", "Console", DevTab::Console, cx))
            .child(tab("dt-network", "Network", DevTab::Network, cx))
            .child(tab(
                "dt-application",
                "Application",
                DevTab::Application,
                cx,
            ))
            // Window drag zone (it has no system bar).
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(6.))
                    .px(px(10.))
                    .text_color(rgb(DT_DIM))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                    .child(dot(if self.connected { DT_GREEN } else { DT_YELLOW }))
                    .child(format!("{} · {}", self.device.name, self.status)),
            )
            .child(caption_strip(&right_caps, close).pr(px(6.)));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(DT_BG))
            .text_color(rgb(DT_TEXT))
            .text_size(px(12.))
            .font_family(UI_FONT)
            .child(strip)
            .child(match self.dev_tab {
                DevTab::Console => self.render_console(inputs, cx).into_any_element(),
                DevTab::Network => self.render_network(inputs, cx).into_any_element(),
                DevTab::Application => self.render_application(inputs, cx).into_any_element(),
            })
            .when_some(self.copy_menu.clone(), |d, (pos, items)| {
                d.child(self.copy_menu(pos, items, cx))
            })
    }

    // ───────────────────────── Console ─────────────────────────

    fn render_console(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> impl IntoElement {
        let query = inputs.console_query.read(cx).value().to_lowercase();
        let levels = self.levels;

        let level_chip = |id: &'static str,
                          label: &'static str,
                          on: bool,
                          cx: &mut Context<Self>,
                          f: fn(&mut Levels)| {
            chip(id, label, on).on_click(cx.listener(move |this, _, _, cx| {
                f(&mut this.levels);
                cx.notify();
            }))
        };

        let toolbar = toolbar()
            .child(tool_icon(
                "c-clear",
                Ico::Clear,
                cx.listener(|this, _, _, cx| {
                    this.logs.clear();
                    cx.notify();
                }),
            ))
            .child(separator())
            .child(div().text_color(rgb(DT_DIM)).child("top ▾"))
            .child(separator())
            .child(
                div()
                    .w(px(220.))
                    .child(Input::new(&inputs.console_query).xsmall()),
            )
            .child(div().flex_1())
            .child(level_chip(
                "lv-verbose",
                "Verbose",
                levels.verbose,
                cx,
                |l| l.verbose = !l.verbose,
            ))
            .child(level_chip("lv-info", "Info", levels.info, cx, |l| {
                l.info = !l.info
            }))
            .child(level_chip(
                "lv-warn",
                "Warnings",
                levels.warnings,
                cx,
                |l| l.warnings = !l.warnings,
            ))
            .child(level_chip("lv-error", "Errors", levels.errors, cx, |l| {
                l.errors = !l.errors
            }))
            .child(separator())
            .child(checkbox(
                "c-preserve",
                "Preserve log",
                self.preserve_log,
                cx.listener(|this, _, _, cx| {
                    this.preserve_log = !this.preserve_log;
                    cx.notify();
                }),
            ));

        let console_doc = SelDoc::new("clog", &self.sel_console, 0);
        let rows = self
            .logs
            .iter()
            .enumerate()
            .filter(|(_, l)| levels.shows(l.level))
            .filter(|(_, l)| query.is_empty() || l.text.to_lowercase().contains(&query))
            .map(|(i, l)| {
                console_row(i, l, &console_doc).on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        let line = |l: &LogEntry| match &l.source {
                            Some(src) => format!("{}  {src}", l.text),
                            None => l.text.to_string(),
                        };
                        let Some(msg) = this.logs.get(i).map(|l| l.text.to_string()) else {
                            return;
                        };
                        let levels = this.levels;
                        let all = this
                            .logs
                            .iter()
                            .filter(|l| levels.shows(l.level))
                            .map(line)
                            .collect::<Vec<_>>()
                            .join("\n");
                        let items = vec![("Copy message", msg), ("Copy console", all)];
                        this.copy_menu = Some((ev.position, items));
                        cx.notify();
                    }),
                )
            })
            .collect::<Vec<_>>();

        let prompt = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(2.))
            .border_t_1()
            .border_color(rgb(DT_BORDER))
            .child(
                div()
                    .text_color(rgb(DT_BLUE))
                    .child(Icon::new(Ico::ChevronRight).xsmall()),
            )
            .child(
                div()
                    .flex_1()
                    .font_family(MONO_FONT)
                    .child(Input::new(&inputs.console_input).appearance(false).xsmall()),
            );

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .id("console-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.console_scroll)
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .child(prompt)
    }

    // ───────────────────────── Network ─────────────────────────

    fn render_network(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> impl IntoElement {
        let query = inputs.net_query.read(cx).value().to_lowercase();
        let filter = self.net_filter;

        let visible: Vec<usize> = self
            .net
            .iter()
            .enumerate()
            .filter(|(_, e)| filter.matches(&e.resource_type))
            .filter(|(_, e)| query.is_empty() || e.url.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();

        let recording = self.recording;
        let toolbar1 = toolbar()
            .child(
                div()
                    .id("n-record")
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .hover(|s| s.bg(rgb(DT_HOVER)))
                    .child(
                        div()
                            .size(px(10.))
                            .rounded_full()
                            .when(recording, |s| s.bg(rgb(DT_RED)))
                            .when(!recording, |s| s.border_1().border_color(rgb(DT_DIM))),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.recording = !this.recording;
                        cx.notify();
                    })),
            )
            .child(tool_icon(
                "n-clear",
                Ico::Clear,
                cx.listener(|this, _, _, cx| {
                    this.net.clear();
                    this.net_index.clear();
                    this.selected = None;
                    cx.notify();
                }),
            ))
            .child(tool_icon(
                "n-har",
                Ico::Download,
                cx.listener(|this, _, _, cx| this.export_har(cx)),
            ))
            .child(
                tool_icon("n-columns", Ico::Columns, |_, _, _| {}).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.columns_menu = Some(ev.position);
                        cx.notify();
                    }),
                ),
            )
            .child(separator())
            .child(
                div()
                    .w(px(220.))
                    .child(Input::new(&inputs.net_query).xsmall()),
            )
            .child(separator())
            .child(checkbox(
                "n-preserve",
                "Preserve log",
                self.preserve_log,
                cx.listener(|this, _, _, cx| {
                    this.preserve_log = !this.preserve_log;
                    cx.notify();
                }),
            ))
            .child(checkbox(
                "n-cache",
                "Disable cache",
                self.disable_cache,
                cx.listener(|this, _, _, cx| {
                    this.disable_cache = !this.disable_cache;
                    this.send(Command::SetCacheDisabled(this.disable_cache));
                    cx.notify();
                }),
            ))
            .child(separator())
            .child(
                div()
                    .id("n-throttle")
                    .px(px(6.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(rgb(DT_HOVER)))
                    .child(format!("{} ▾", self.throttle.label()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let i = Throttle::ALL
                            .iter()
                            .position(|x| *x == this.throttle)
                            .unwrap_or(0);
                        let next = Throttle::ALL[(i + 1) % Throttle::ALL.len()];
                        this.throttle = next;
                        this.send(Command::Throttle(next));
                        cx.notify();
                    })),
            );

        let mut toolbar2 = toolbar();
        for (i, f) in NetFilter::ALL.into_iter().enumerate() {
            toolbar2 = toolbar2.child(chip(("nf", i), f.label(), f == filter).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.net_filter = f;
                    cx.notify();
                }),
            ));
        }

        // Waterfall scale.
        let t0 = visible
            .iter()
            .map(|&i| self.net[i].start_ts)
            .fold(f64::MAX, f64::min);
        let t1 = visible
            .iter()
            .map(|&i| self.net[i].end_ts.unwrap_or(self.net[i].start_ts))
            .fold(f64::MIN, f64::max);
        let span = (t1 - t0).max(0.001);

        let compact = self.selected.is_some();
        // With the detail open only the name is visible (like DevTools).
        let columns: Vec<NetColumn> = if compact {
            vec![NetColumn::Name]
        } else {
            NetColumn::ALL
                .into_iter()
                .filter(|c| self.net_columns.contains(c))
                .collect()
        };
        let total_w: f32 = columns.iter().map(|c| c.width()).sum();
        let sort = self.net_sort;

        let mut header = div()
            .id("net-header")
            .h(px(22.))
            .flex()
            .flex_none()
            .items_center()
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .text_color(rgb(DT_DIM))
            // Right-click on the header: choose columns (like Chrome and Firefox).
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.columns_menu = Some(ev.position);
                    cx.notify();
                }),
            );
        for (n, col) in columns.iter().copied().enumerate() {
            let arrow = sort
                .filter(|(c, _)| *c == col)
                .map(|(_, asc)| if asc { Ico::ArrowUp } else { Ico::ArrowDown });
            let base = if col == NetColumn::Name {
                cell_flex("")
            } else {
                cell("", col.width())
            };
            header = header.child(
                base.id(("nh", n))
                    .gap(px(4.))
                    .hover(|s| s.bg(rgb(DT_HOVER)))
                    .child(div().truncate().child(col.label()))
                    .when_some(arrow, |d, a| d.child(Icon::new(a).xsmall()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // Asc → desc → no order.
                        this.net_sort = match this.net_sort {
                            Some((c, true)) if c == col => Some((col, false)),
                            Some((c, false)) if c == col => None,
                            _ => Some((col, true)),
                        };
                        cx.notify();
                    })),
            );
        }

        let mut visible = visible;
        if let Some((col, asc)) = sort {
            visible.sort_by(|&a, &b| {
                let (ka, kb) = (col.sort_key(&self.net[a]), col.sort_key(&self.net[b]));
                let o = ka.0.total_cmp(&kb.0).then_with(|| ka.1.cmp(&kb.1));
                if asc { o } else { o.reverse() }
            });
        }

        let mut rows = div().flex().flex_col();
        for (n, &i) in visible.iter().enumerate() {
            let e = &self.net[i];
            let selected = self.selected.as_deref() == Some(e.id.as_str());
            let failed = e.error.is_some() || e.status.is_some_and(|s| s >= 400);
            let id = e.id.clone();
            let mut row = div()
                .id(SharedString::from(format!("nrow-{}", e.id)))
                .h(px(22.))
                .flex()
                .items_center()
                .when(n % 2 == 1, |d| d.bg(rgb(DT_ROW_ALT)))
                .when(selected, |d| d.bg(rgb(DT_SELECTED)))
                .when(!selected, |d| d.hover(|s| s.bg(rgb(DT_HOVER))))
                .text_color(rgb(if failed && !selected { DT_RED } else { DT_TEXT }));
            for col in columns.iter().copied() {
                row = row.child(match col {
                    NetColumn::Name => cell_flex(col.text(e)),
                    NetColumn::Waterfall => {
                        let left = ((e.start_ts - t0) / span) as f32 * WATERFALL_W;
                        let width = (((e.end_ts.unwrap_or(t1) - e.start_ts) / span) as f32
                            * WATERFALL_W)
                            .max(2.0);
                        div()
                            .w(px(col.width()))
                            .flex_none()
                            .h_full()
                            .px(px(6.))
                            .relative()
                            .border_l_1()
                            .border_color(rgb(DT_BORDER))
                            .child(
                                div()
                                    .absolute()
                                    .top(px(8.))
                                    .left(px(6. + left))
                                    .w(px(width))
                                    .h(px(6.))
                                    .rounded(px(1.))
                                    .bg(rgb(if e.end_ts.is_some() { DT_BLUE } else { DT_DIM })),
                            )
                    }
                    _ if col.numeric() => cell(col.text(e), col.width()).justify_end(),
                    _ => cell(col.text(e), col.width()),
                });
            }
            let menu_id = id.clone();
            rows = rows.child(
                row.on_click(cx.listener(move |this, _, _, cx| {
                    this.select_request(id.clone());
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        let Some(&i) = this.net_index.get(&menu_id) else {
                            return;
                        };
                        let e = &this.net[i];
                        let lines = |h: &[(String, String)]| {
                            h.iter()
                                .map(|(k, v)| format!("{k}: {v}"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        };
                        let mut items = vec![
                            ("Copy URL", e.url.clone()),
                            ("Copy as cURL", curl_command(e)),
                        ];
                        if let Some(body) = e.post_data.clone() {
                            items.push(("Copy payload", body));
                        }
                        if let Some(body) = e.body.clone().filter(|_| !has_viewer(e)) {
                            items.push(("Copy response", body));
                        }
                        items.push(("Copy request headers", lines(&e.request_headers)));
                        items.push(("Copy response headers", lines(&e.response_headers)));
                        items.push((
                            "Copy all URLs",
                            this.net
                                .iter()
                                .map(|e| e.url.as_str())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ));
                        this.copy_menu = Some((ev.position, items));
                        cx.notify();
                    }),
                ),
            );
        }

        // With many columns the table scrolls horizontally (header included).
        let table = div()
            .min_w(px(total_w))
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("net-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.net_scroll)
                    .child(rows),
            );
        let list = div()
            .id("net-list")
            .when(compact, |d| {
                d.w(px(280.)).border_r_1().border_color(rgb(DT_BORDER))
            })
            .when(!compact, |d| d.flex_1())
            .min_w_0()
            .h_full()
            .overflow_x_scroll()
            // GPUI turns the vertical wheel into horizontal in an only-X container: without this,
            // every notch moved the table in both axes at once.
            .restrict_scroll_to_axis()
            .child(table);

        let transferred: f64 = visible.iter().map(|&i| self.net[i].size).sum();
        let finish = if visible.is_empty() {
            "—".into()
        } else {
            format!("{:.0} ms", span * 1000.0)
        };
        let status = div()
            .h(px(22.))
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(8.))
            .bg(rgb(DT_TOOLBAR))
            .border_t_1()
            .border_color(rgb(DT_BORDER))
            .text_color(rgb(DT_DIM))
            .child(format!("{} / {} requests", visible.len(), self.net.len()))
            .child(separator())
            .child(format!("{} transferred", fmt_size(transferred)))
            .child(separator())
            .child(format!("Finish: {finish}"));

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar1)
            .child(toolbar2)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(list)
                    .when(compact, |d| d.child(self.render_net_detail(inputs, cx))),
            )
            .child(status)
            .when_some(self.columns_menu, |d, pos| {
                d.child(self.columns_menu(pos, cx))
            })
    }

    fn render_net_detail(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> impl IntoElement {
        let doc = SelDoc::new("nd", &self.sel_detail, 0);
        let entry = self
            .selected
            .as_ref()
            .and_then(|id| self.net_index.get(id))
            .and_then(|&i| self.net.get(i))
            .cloned();
        let Some(e) = entry else {
            return div().into_any_element();
        };
        let jwts = net_tools::find_jwts(&e);
        if self.net_detail_tab == NetDetailTab::Jwt && jwts.is_empty() {
            self.net_detail_tab = NetDetailTab::Headers;
        }
        let detail_tab = self.net_detail_tab;
        let query = inputs.detail_query.read(cx).value().to_string();
        let wrap = self.detail_wrap;

        let tab = |id: &'static str, label: String, t: NetDetailTab, cx: &mut Context<Self>| {
            let active = detail_tab == t;
            div()
                .id(id)
                .h_full()
                .px(px(8.))
                .flex()
                .flex_none()
                .items_center()
                .border_b_2()
                .border_color(if active {
                    rgb(DT_BLUE)
                } else {
                    rgba(0x00000000)
                })
                .text_color(rgb(if active { DT_TEXT } else { DT_DIM }))
                .hover(|s| s.bg(rgb(DT_HOVER)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.net_detail_tab = t;
                    cx.notify();
                }))
        };

        let has_text = matches!(
            detail_tab,
            NetDetailTab::Payload | NetDetailTab::Preview | NetDetailTab::Response
        ) && !has_viewer(&e);
        let tabs = div()
            .h(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .child(tool_icon(
                "nd-close",
                Ico::Close,
                cx.listener(|this, _, _, cx| {
                    this.selected = None;
                    cx.notify();
                }),
            ))
            .child(tab(
                "nd-headers",
                "Headers".into(),
                NetDetailTab::Headers,
                cx,
            ))
            .child(tab(
                "nd-payload",
                "Payload".into(),
                NetDetailTab::Payload,
                cx,
            ))
            .child(tab(
                "nd-preview",
                "Preview".into(),
                NetDetailTab::Preview,
                cx,
            ))
            .child(tab(
                "nd-response",
                "Response".into(),
                NetDetailTab::Response,
                cx,
            ))
            .child(tab("nd-timing", "Timing".into(), NetDetailTab::Timing, cx))
            .when(!jwts.is_empty(), |d| {
                d.child(tab(
                    "nd-jwt",
                    format!("JWT ({})", jwts.len()),
                    NetDetailTab::Jwt,
                    cx,
                ))
            })
            .child(tab("nd-replay", "Replay".into(), NetDetailTab::Replay, cx))
            .child(div().flex_1())
            .when(has_text, |d| {
                d.child(toggle_button(
                    "nd-wrap",
                    "Wrap",
                    wrap,
                    cx.listener(|this, _, _, cx| {
                        this.detail_wrap = !this.detail_wrap;
                        cx.notify();
                    }),
                ))
            })
            .when_some(self.detail_text(&e), |d, text| {
                d.child(self.copy_button("nd-copy", "Copy", text, cx))
            })
            .child(self.copy_button("nd-curl", "cURL", curl_command(&e), cx))
            // Opens the editor with the original data (if untouched, it is replayed unchanged).
            .when(detail_tab != NetDetailTab::Replay, |d| {
                d.child(self.action_button(
                    "nd-repeat",
                    Ico::Refresh,
                    "Replay",
                    "Replayed",
                    cx.listener(|this, _, _, cx| {
                        this.net_detail_tab = NetDetailTab::Replay;
                        this.replay_for = None;
                        cx.notify();
                    }),
                ))
            })
            .child(div().w(px(6.)));

        // Search bar (not in Timing or in the editor).
        let searchable = detail_tab != NetDetailTab::Timing && !has_viewer(&e);
        let matches = match detail_tab {
            NetDetailTab::Headers => e
                .request_headers
                .iter()
                .chain(&e.response_headers)
                .filter(|(k, v)| row_matches(k, v, &query))
                .count(),
            NetDetailTab::Jwt => jwts
                .iter()
                .map(|j| net_tools::search_ranges(&j.payload, &query).len())
                .sum(),
            NetDetailTab::Replay => self
                .replay_result
                .as_ref()
                .and_then(|id| self.net_index.get(id))
                .and_then(|&i| self.net.get(i))
                .map_or(0, |r| {
                    net_tools::search_ranges(&body_text(r, 200_000), &query).len()
                }),
            _ => self
                .detail_text(&e)
                .map_or(0, |t| net_tools::search_ranges(&t, &query).len()),
        };
        let search = div()
            .h(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .child(
                div()
                    .w(px(240.))
                    .child(Input::new(&inputs.detail_query).xsmall().cleanable(true)),
            )
            .when(!query.is_empty(), |d| {
                d.child(
                    div()
                        .text_color(rgb(if matches > 0 { DT_DIM } else { DT_RED }))
                        .child(match matches {
                            0 => "No matches".to_string(),
                            1 => "1 match".to_string(),
                            n => format!("{n} matches"),
                        }),
                )
            });

        let headers = |h: &[(String, String)], cx: &mut Context<Self>| {
            let rows: Vec<_> = h
                .iter()
                .filter(|(k, v)| row_matches(k, v, &query))
                .cloned()
                .collect();
            kv_rows(&rows, &doc, cx)
        };
        let body: AnyElement = match detail_tab {
            NetDetailTab::Headers => {
                let (status, color) = match (&e.error, e.status) {
                    (Some(err), _) => (format!("(failed) {err}"), DT_RED),
                    (None, Some(s)) => (
                        format!("{s} {}", e.status_text).trim().to_string(),
                        if s < 400 { DT_GREEN } else { DT_RED },
                    ),
                    (None, None) => ("(pending)".into(), DT_DIM),
                };
                let mut general = vec![
                    ("Request URL".to_string(), e.url.clone()),
                    ("Request Method".to_string(), e.method.clone()),
                ];
                if let Some(a) = &e.remote_addr {
                    general.push(("Remote Address".into(), a.clone()));
                }
                div()
                    .flex()
                    .flex_col()
                    .child(section_title("General"))
                    .child(kv_rows(&general[..1], &doc, cx))
                    .child(kv_rows(&general[1..2], &doc, cx))
                    .child(
                        div()
                            .flex()
                            .py(px(2.))
                            .child(
                                div()
                                    .w(px(180.))
                                    .flex_none()
                                    .text_color(rgb(DT_DIM))
                                    .child("Status Code"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(dot(color))
                                    .child(status),
                            ),
                    )
                    .child(kv_rows(&general[2..], &doc, cx))
                    .child(section_title(&format!(
                        "Response Headers ({})",
                        e.response_headers.len()
                    )))
                    .child(headers(&e.response_headers, cx))
                    .child(section_title(&format!(
                        "Request Headers ({})",
                        e.request_headers.len()
                    )))
                    .child(headers(&e.request_headers, cx))
                    .into_any_element()
            }
            NetDetailTab::Payload => {
                let (_, _, _, params) = split_url(&e.url);
                div()
                    .flex()
                    .flex_col()
                    .when(!params.is_empty(), |d| {
                        d.child(section_title("Query String Parameters"))
                            .child(kv_rows(&params, &doc, cx))
                    })
                    .child(section_title("Request Payload"))
                    .child(code_view(
                        e.post_data
                            .as_deref()
                            .map(pretty_json)
                            .unwrap_or_else(|| "No payload".into()),
                        &query,
                        wrap,
                    ))
                    .into_any_element()
            }
            NetDetailTab::Preview | NetDetailTab::Response if e.image.is_some() => {
                image_preview(e.image.clone().unwrap(), &e).into_any_element()
            }
            NetDetailTab::Preview | NetDetailTab::Response if engine::is_pdf(&e.mime, &e.url) => {
                pdf_preview(&e).into_any_element()
            }
            NetDetailTab::Preview => {
                code_view(body_text(&e, 200_000), &query, wrap).into_any_element()
            }
            NetDetailTab::Response => code_view(
                match &e.body {
                    Some(b) => b.chars().take(200_000).collect(),
                    None => body_text(&e, 0),
                },
                &query,
                wrap,
            )
            .into_any_element(),
            NetDetailTab::Timing => {
                let rel = |t: f64| format!("{:.1} ms", t * 1000.0);
                let mut rows = vec![
                    ("Started at".to_string(), clock(e.wall_time, true)),
                    ("Queued → start".to_string(), "0 ms".to_string()),
                ];
                if let Some(end) = e.end_ts {
                    rows.push(("Total".into(), rel(end - e.start_ts)));
                }
                div()
                    .flex()
                    .flex_col()
                    .child(section_title("Timing"))
                    .child(kv_rows(&rows, &doc, cx))
                    .into_any_element()
            }
            NetDetailTab::Jwt => {
                let mut list = div().flex().flex_col().gap(px(14.));
                for (i, jwt) in jwts.iter().enumerate() {
                    list = list.child(self.jwt_card(i, jwt, &query, cx));
                }
                list.into_any_element()
            }
            NetDetailTab::Replay => self
                .replay_editor(inputs, &query, wrap, cx)
                .into_any_element(),
        };

        let scroll = div().id("nd-body").flex_1().min_h_0().p(px(10.));
        let scroll = if wrap || !has_text {
            scroll.overflow_y_scroll()
        } else {
            scroll.overflow_scroll()
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(tabs)
            .when(searchable, |d| d.child(search))
            .child(scroll.child(body))
            .into_any_element()
    }

    fn jwt_card(&self, i: usize, jwt: &Jwt, query: &str, cx: &mut Context<Self>) -> Div {
        let doc = SelDoc::new("jwt", &self.sel_detail, 1_000_000 * (i as u64 + 1));
        let (badge, color) = if jwt.expired {
            ("Expired", DT_RED)
        } else {
            ("Valid", DT_GREEN)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(DT_BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(jwt.source.clone()),
                    )
                    .child(
                        div()
                            .px(px(6.))
                            .rounded(px(4.))
                            .text_size(px(11.))
                            .text_color(rgb(color))
                            .border_1()
                            .border_color(rgb(color))
                            .when(!jwt.times.iter().any(|(k, _)| k.contains("exp")), |d| {
                                d.invisible()
                            })
                            .child(badge),
                    )
                    .child(div().flex_1())
                    .child(self.copy_button(("jwt-copy", i), "Copy token", jwt.token.clone(), cx))
                    .child(self.copy_button(
                        ("jwt-payload", i),
                        "Copy payload",
                        jwt.payload.clone(),
                        cx,
                    )),
            )
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(11.))
                    .text_color(rgb(DT_DIM))
                    .child(jwt.token.clone()),
            )
            .when(!jwt.times.is_empty(), |d| {
                d.child(kv_rows(&jwt.times, &doc, cx))
            })
            .child(section_title("Header"))
            .child(code_view(jwt.header.clone(), query, true))
            .child(section_title("Payload"))
            .child(code_view(jwt.payload.clone(), query, true))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(DT_DIM))
                    .child("The signature is not verified: modev does not know the key."),
            )
    }

    /// "Replay" editor: method, URL, headers and body are editable.
    fn replay_editor(
        &self,
        inputs: &Inputs,
        query: &str,
        wrap: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let send = {
            let inputs = inputs.clone();
            cx.listener(move |this, _, _, cx| {
                let value =
                    |f: &Entity<InputState>, cx: &App| f.read(cx).value().trim().to_string();
                let method = value(&inputs.replay_method, cx).to_uppercase();
                let url = value(&inputs.replay_url, cx);
                if url.is_empty() {
                    return;
                }
                let headers = inputs
                    .replay_headers
                    .read(cx)
                    .value()
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .filter(|(k, _)| !k.is_empty())
                    .collect();
                let edited = inputs.replay_body.read(cx).value().to_string();
                // The editor shows the formatted JSON: unchanged, the original body is sent byte for byte.
                let original = this
                    .replay_source
                    .as_ref()
                    .and_then(|s| s.post_data.clone());
                let body = match &original {
                    Some(o) if pretty_json(o) == edited => Some(o.clone()),
                    _ => (!edited.trim().is_empty()).then_some(edited),
                };
                let method = if method.is_empty() {
                    "GET".to_string()
                } else {
                    method
                };
                this.replay_pending = Some((method.clone(), url.clone()));
                this.replay_result = None;
                this.send(Command::Replay {
                    method,
                    url,
                    headers,
                    body,
                });
                this.flash("nr-send", cx);
            })
        };
        let label = |t: &'static str| {
            div()
                .mt(px(10.))
                .mb(px(4.))
                .text_color(rgb(DT_DIM))
                .child(t)
        };
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(div().w(px(90.)).child(Input::new(&inputs.replay_method).xsmall()))
                    .child(div().flex_1().child(Input::new(&inputs.replay_url).xsmall())),
            )
            .child(label("Headers (one per line: «Name: value»; Authorization included)"))
            .child(div().h(px(150.)).child(Editor::new(&inputs.replay_headers).h_full()))
            .child(label("Body"))
            .child(div().h(px(220.)).child(Editor::new(&inputs.replay_body).h_full()))
            .child(
                div()
                    .mt(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.action_button("nr-send", Ico::Refresh, "Send", "Sent", send))
                    .child(
                        div()
                            .id("nr-reset")
                            .h(px(22.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .text_color(rgb(DT_DIM))
                            .hover(|s| s.bg(rgb(DT_HOVER)).text_color(rgb(DT_TEXT)))
                            .child("Restore original")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.replay_for = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(DT_DIM))
                            .child("Sent from the page (same origin and cookies); it also appears in the list. The browser ignores Cookie, Host and other protected headers."),
                    ),
            )
            .child(self.replay_response(query, wrap))
    }

    /// Response of the last replay, below the editor (Postman style).
    fn replay_response(&self, query: &str, wrap: bool) -> Div {
        let pending = self.replay_pending.is_some();
        let result = self
            .replay_result
            .as_ref()
            .and_then(|id| self.net_index.get(id))
            .and_then(|&i| self.net.get(i));
        let Some(r) = result else {
            return div().when(pending, |d| {
                d.child(section_title("Response"))
                    .child(div().text_color(rgb(DT_DIM)).child("Sending…"))
            });
        };
        let (status, color) = match (&r.error, r.status) {
            (Some(err), _) => (format!("(failed) {err}"), DT_RED),
            (None, Some(s)) => (
                format!("{s} {}", r.status_text).trim().to_string(),
                if s < 400 { DT_GREEN } else { DT_RED },
            ),
            (None, None) => ("(pending)".into(), DT_DIM),
        };
        let body: AnyElement = match &r.image {
            Some(image) => image_preview(image.clone(), r).into_any_element(),
            None => code_view(body_text(r, 200_000), query, wrap).into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .child(section_title("Response"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mb(px(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(dot(color))
                            .child(status),
                    )
                    .child(div().text_color(rgb(DT_DIM)).child(fmt_duration(r)))
                    .child(div().text_color(rgb(DT_DIM)).child(fmt_size(r.size))),
            )
            .child(body)
    }

    /// Data to load into the Replay editor if the selected request changed.
    pub(super) fn replay_to_load(&mut self) -> Option<NetEntry> {
        if self.net_detail_tab != NetDetailTab::Replay || self.selected == self.replay_for {
            return None;
        }
        self.replay_for = self.selected.clone();
        self.replay_pending = None;
        self.replay_result = None;
        let id = self.selected.as_ref()?;
        self.replay_source = self
            .net_index
            .get(id)
            .and_then(|&i| self.net.get(i))
            .cloned();
        self.replay_source.clone()
    }
}

// ───────────────────────── Columns ─────────────────────────

impl Modev {
    /// Menu to show or hide columns; it closes when clicking outside.
    fn columns_menu(&self, pos: Point<Pixels>, cx: &mut Context<Self>) -> impl IntoElement {
        let mut menu = div()
            .id("columns-menu")
            .occlude()
            .w(px(200.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(rgb(theme::RAISED))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.columns_menu = None;
                cx.notify();
            }));
        for (i, col) in NetColumn::ALL.into_iter().enumerate().skip(1) {
            let on = self.net_columns.contains(&col);
            menu = menu.child(
                div()
                    .id(("col", i))
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .hover(|s| s.bg(rgb(DT_HOVER)))
                    .child(
                        div()
                            .size(px(14.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(DT_BLUE))
                            .when(on, |d| d.child(Icon::new(Ico::Check).xsmall())),
                    )
                    .child(col.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if on {
                            this.net_columns.retain(|c| *c != col);
                        } else {
                            this.net_columns.push(col);
                        }
                        NetColumn::save(&this.net_columns);
                        cx.notify();
                    })),
            );
        }
        menu = menu
            .child(div().my(px(4.)).h(px(1.)).bg(rgb(DT_BORDER)))
            .child(
                div()
                    .id("col-reset")
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .text_color(rgb(DT_DIM))
                    .hover(|s| s.bg(rgb(DT_HOVER)).text_color(rgb(DT_TEXT)))
                    .child("Reset columns")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.net_columns = NetColumn::DEFAULT.to_vec();
                        NetColumn::save(&this.net_columns);
                        cx.notify();
                    })),
            );
        deferred(anchored().position(pos).snap_to_window().child(menu)).with_priority(1)
    }
}

// ───────────────────────── Copy ─────────────────────────

impl Modev {
    fn copy_menu(
        &self,
        pos: Point<Pixels>,
        items: Vec<(&'static str, String)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut menu = div()
            .id("copy-menu")
            .occlude()
            .min_w(px(200.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(rgb(theme::RAISED))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.copy_menu = None;
                cx.notify();
            }));
        for (i, (label, text)) in items.into_iter().enumerate() {
            menu = menu.child(
                div()
                    .id(("copy-item", i))
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .hover(|s| s.bg(rgb(DT_HOVER)))
                    .child(
                        div()
                            .text_color(rgb(DT_DIM))
                            .child(Icon::new(Ico::Copy).xsmall()),
                    )
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        this.copy_menu = None;
                        cx.notify();
                    })),
            );
        }
        deferred(anchored().position(pos).snap_to_window().child(menu)).with_priority(1)
    }

    /// What "Copy" copies in the current detail tab (`None` if there is nothing).
    fn detail_text(&self, e: &NetEntry) -> Option<String> {
        let lines = |h: &[(String, String)]| {
            h.iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        match self.net_detail_tab {
            NetDetailTab::Headers => Some(format!(
                "{} {}\n\n{}\n\n{}",
                e.method,
                e.url,
                lines(&e.response_headers),
                lines(&e.request_headers)
            )),
            NetDetailTab::Payload => e.post_data.clone(),
            NetDetailTab::Preview if !has_viewer(e) => e.body.as_deref().map(pretty_json),
            NetDetailTab::Response if !has_viewer(e) => e.body.clone(),
            _ => None,
        }
    }

    /// Shows a button's confirmation for 1.5 s.
    fn flash(&mut self, id: impl Into<ElementId>, cx: &mut Context<Self>) {
        self.copied = Some(id.into());
        self.copied_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn action_button(
        &self,
        id: impl Into<ElementId>,
        icon: Ico,
        label: &'static str,
        done_label: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        let id = id.into();
        let done = self.copied.as_ref() == Some(&id);
        div()
            .id(id)
            .h(px(22.))
            .px(px(8.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .rounded(px(6.))
            .text_color(rgb(if done { DT_GREEN } else { DT_DIM }))
            .hover(|s| s.bg(rgb(DT_HOVER)).text_color(rgb(DT_TEXT)))
            .child(Icon::new(if done { Ico::Check } else { icon }).xsmall())
            .child(if done { done_label } else { label })
            .on_click(on_click)
    }

    fn copy_button(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        text: String,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = id.into();
        let flash_id = id.clone();
        self.action_button(
            id,
            Ico::Copy,
            label,
            "Copied",
            cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                this.flash(flash_id.clone(), cx);
            }),
        )
    }
}

/// The request as a `curl` command (like DevTools' "Copy as cURL").
fn curl_command(e: &NetEntry) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut cmd = format!("curl {}", q(&e.url));
    if e.method != "GET" {
        cmd += &format!(" \\\n  -X {}", e.method);
    }
    // HTTP/2 pseudo-headers (`:authority`…) are not headers for curl.
    for (k, v) in e
        .request_headers
        .iter()
        .filter(|(k, _)| !k.starts_with(':'))
    {
        cmd += &format!(" \\\n  -H {}", q(&format!("{k}: {v}")));
    }
    if let Some(body) = &e.post_data {
        cmd += &format!(" \\\n  --data-raw {}", q(body));
    }
    cmd
}

fn row_matches(k: &str, v: &str, query: &str) -> bool {
    let q = query.to_lowercase();
    q.is_empty() || k.to_lowercase().contains(&q) || v.to_lowercase().contains(&q)
}

fn toggle_button(
    id: &'static str,
    label: &'static str,
    on: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(22.))
        .px(px(8.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .rounded(px(6.))
        .text_color(rgb(if on { DT_TEXT } else { DT_DIM }))
        .when(on, |s| s.bg(rgb(DT_HOVER)))
        .hover(|s| s.bg(rgb(DT_HOVER)).text_color(rgb(DT_TEXT)))
        .child(Icon::new(Ico::WrapText).xsmall())
        .child(label)
        .on_click(on_click)
}

/// Image preview: at its size (at most the panel's) with its data.
fn image_preview(image: Arc<Image>, e: &NetEntry) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .p(px(12.))
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(DT_BORDER))
                .bg(rgb(DT_ROW_ALT))
                .child(
                    img(image)
                        .max_w(px(560.))
                        .max_h(px(560.))
                        .object_fit(ObjectFit::Contain),
                ),
        )
        .child(div().text_color(rgb(DT_DIM)).child(format!(
            "{} · {}",
            if e.mime.is_empty() { "image" } else { &e.mime },
            fmt_size(e.size)
        )))
}

/// Binary responses with their own viewer (image or PDF) instead of text.
fn has_viewer(e: &NetEntry) -> bool {
    e.image.is_some() || engine::is_pdf(&e.mime, &e.url)
}

/// PDF viewer: the pages one below the other, at the panel's width.
fn pdf_preview(e: &NetEntry) -> Div {
    const PAGE_W: f32 = 560.0;
    let info = |text: String| div().text_color(rgb(DT_DIM)).child(text);
    let mut d = div().flex().flex_col().items_center().gap(px(10.));
    let Some(pages) = &e.pdf else {
        let msg = if e.body.is_some() {
            "Rendering PDF…"
        } else {
            "Loading PDF…"
        };
        return d.child(info(msg.into()));
    };
    if pages.is_empty() {
        return d.child(info("Could not render the PDF".into()));
    }
    d = d.child(info(format!(
        "{} · {} · {} page{}",
        if e.mime.is_empty() { "PDF" } else { &e.mime },
        fmt_size(e.size),
        pages.len(),
        if pages.len() == 1 { "" } else { "s" }
    )));
    for (i, p) in pages.iter().enumerate() {
        let h = PAGE_W * p.height as f32 / p.width.max(1) as f32;
        d = d.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .border_1()
                        .border_color(rgb(DT_BORDER))
                        .shadow_md()
                        .child(img(p.image.clone()).w(px(PAGE_W)).h(px(h))),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(DT_DIM))
                        .child(format!("{} / {}", i + 1, pages.len())),
                ),
        );
    }
    d
}

// ───────────────────────── JSON ─────────────────────────

/// Monospaced text; if it is valid JSON, colored like in DevTools. Highlights `query`.
fn code_view(text: String, query: &str, wrap: bool) -> Div {
    let el = div()
        .font_family(MONO_FONT)
        .text_size(px(12.))
        .when(!wrap, |d| d.whitespace_nowrap());
    let syntax = if serde_json::from_str::<serde::de::IgnoredAny>(&text).is_ok() {
        json_highlights(&text)
    } else {
        vec![]
    };
    let matches = net_tools::search_ranges(&text, query);
    // Stable id per content: two sibling views (JWT header/payload) don't collide.
    let id = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        SharedString::from(format!("code-{:x}", h.finish()))
    };
    if syntax.is_empty() && matches.is_empty() {
        return el.child(SelText::new(id, text));
    }
    let highlights = net_tools::merge_highlights(syntax, &matches, theme::SEARCH_MATCH);
    el.child(SelText::new(id, text).highlights(highlights))
}

fn json_highlights(s: &str) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let color = |c: u32| HighlightStyle {
        color: Some(rgb(c).into()),
        ..Default::default()
    };
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let start = i;
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                // A string followed by `:` is a key.
                let next = b[i..].iter().find(|c| !c.is_ascii_whitespace());
                let c = if next == Some(&b':') {
                    theme::SYN_KEY
                } else {
                    theme::SYN_STRING
                };
                out.push((start..i, color(c)));
            }
            b'-' | b'0'..=b'9' => {
                while i < b.len() && matches!(b[i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    i += 1;
                }
                out.push((start..i, color(theme::SYN_NUMBER)));
            }
            b't' | b'f' | b'n' => {
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                out.push((start..i, color(theme::SYN_LITERAL)));
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                i += 1;
                out.push((start..i, color(DT_DIM)));
            }
            _ => i += 1,
        }
    }
    out
}

// ───────────────────────── Pieces ─────────────────────────

fn toolbar() -> Div {
    div()
        .h(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(6.))
        .bg(rgb(DT_TOOLBAR))
        .border_b_1()
        .border_color(rgb(DT_BORDER))
        .text_color(rgb(DT_TEXT))
}

fn separator() -> Div {
    div().w(px(1.)).h(px(16.)).bg(rgb(DT_BORDER))
}

fn tool_icon(
    id: &'static str,
    icon: Ico,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(22.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_color(rgb(DT_DIM))
        .hover(|s| s.bg(rgb(DT_HOVER)).text_color(rgb(DT_TEXT)))
        .child(Icon::new(icon).xsmall())
        .on_click(on_click)
}

fn chip(id: impl Into<ElementId>, label: &'static str, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(8.))
        .py(px(1.))
        .rounded_full()
        .border_1()
        .border_color(rgb(if active { DT_BLUE } else { DT_BORDER }))
        .when(active, |s| s.bg(rgba(0x7c86ff33)))
        .text_color(rgb(if active { DT_TEXT } else { DT_DIM }))
        .hover(|s| s.bg(rgb(DT_HOVER)))
        .child(label)
}

fn checkbox(
    id: &'static str,
    label: &'static str,
    checked: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            div()
                .size(px(13.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(2.))
                .border_1()
                .border_color(rgb(if checked { DT_BLUE } else { DT_DIM }))
                .when(checked, |s| s.bg(rgb(DT_BLUE)).text_color(rgb(DT_BG)))
                .when(checked, |s| s.child(Icon::new(Ico::Check).xsmall())),
        )
        .child(label)
        .on_click(on_click)
}

fn cell(text: impl Into<SharedString>, width: f32) -> Div {
    div()
        .w(px(width))
        .flex_none()
        .h_full()
        .px(px(6.))
        .flex()
        .items_center()
        .border_l_1()
        .border_color(rgb(DT_BORDER))
        .overflow_hidden()
        .child(div().truncate().child(text.into()))
}

fn cell_flex(text: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .px(px(6.))
        .flex()
        .items_center()
        .overflow_hidden()
        .child(div().truncate().child(text.into()))
}

fn section_title(t: &str) -> Div {
    div()
        .mt(px(10.))
        .mb(px(4.))
        .pb(px(2.))
        .border_b_1()
        .border_color(rgb(DT_BORDER))
        .font_weight(FontWeight::BOLD)
        .child(format!("▼ {t}"))
}

fn kv_rows(kv: &[(String, String)], doc: &SelDoc, cx: &mut Context<Modev>) -> Div {
    let mut d = div().flex().flex_col();
    for (k, v) in kv {
        let (key, value) = (k.clone(), v.clone());
        d = d.child(
            div()
                .flex()
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        let items = vec![
                            ("Copy value", value.clone()),
                            ("Copy line", format!("{key}: {value}")),
                        ];
                        this.copy_menu = Some((ev.position, items));
                        cx.notify();
                    }),
                )
                .py(px(2.))
                .child(
                    div()
                        .w(px(180.))
                        .flex_none()
                        .text_color(rgb(DT_DIM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(doc.text(k.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(MONO_FONT)
                        .child(doc.text(v.clone())),
                ),
        );
    }
    d
}

fn console_row(i: usize, l: &LogEntry, doc: &SelDoc) -> Stateful<Div> {
    let (bg, border, color, icon) = match l.level {
        LogLevel::Error => (Some(ERR_BG), ERR_BORDER, DT_RED, Some(Ico::Error)),
        LogLevel::Warn => (Some(WARN_BG), WARN_BORDER, DT_YELLOW, Some(Ico::Warning)),
        LogLevel::Info => (None, DT_BORDER, DT_TEXT, Some(Ico::Info)),
        LogLevel::Command => (None, DT_BORDER, DT_DIM, Some(Ico::ChevronRight)),
        LogLevel::Result => (None, DT_BORDER, DT_TEXT, Some(Ico::ChevronLeft)),
        LogLevel::Debug => (None, DT_BORDER, DT_BLUE, None),
        LogLevel::Log => (None, DT_BORDER, DT_TEXT, None),
    };
    div()
        .id(("crow", i))
        .flex()
        .items_start()
        .gap(px(6.))
        .px(px(8.))
        .py(px(3.))
        .border_b_1()
        .border_color(rgb(border))
        .when_some(bg, |d, bg| d.bg(rgb(bg)))
        .child(
            div()
                .w(px(14.))
                .flex_none()
                .pt(px(1.))
                .text_color(rgb(color))
                .when_some(icon, |d, icon| d.child(Icon::new(icon).xsmall())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(MONO_FONT)
                .text_color(rgb(color))
                .child(doc.text(l.text.clone())),
        )
        .when_some(l.source.clone(), |d, src| {
            d.child(div().flex_none().text_color(rgb(DT_DIM)).child(src))
        })
}

fn request_name(url: &str) -> String {
    let (_, host, path, _) = split_url(url);
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if last.is_empty() {
        host
    } else {
        last.to_string()
    }
}

fn status_text(e: &NetEntry) -> String {
    match (&e.error, e.status) {
        (Some(_), _) => "(failed)".into(),
        (None, Some(s)) => s.to_string(),
        (None, None) => "(pending)".into(),
    }
}

fn type_label(t: &str) -> String {
    match t {
        "Xhr" => "xhr".into(),
        "Stylesheet" => "stylesheet".into(),
        other => other.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::{NetEntry, curl_command, row_matches, status_text};

    #[test]
    fn curl_get_is_just_the_url() {
        let e = NetEntry::sample("GET", "https://api.dev/a?b=1");
        assert_eq!(curl_command(&e), "curl 'https://api.dev/a?b=1'");
    }

    #[test]
    fn curl_with_method_headers_and_body() {
        let mut e = NetEntry::sample("POST", "https://api.dev/it's");
        e.request_headers = vec![
            (":authority".into(), "api.dev".into()),
            (":method".into(), "POST".into()),
            ("Content-Type".into(), "application/json".into()),
            ("X-Note".into(), "it's".into()),
        ];
        e.post_data = Some(r#"{"name":"O'Brien"}"#.into());
        assert_eq!(
            curl_command(&e),
            "curl 'https://api.dev/it'\\''s' \\\n  -X POST \\\n  -H 'Content-Type: application/json' \\\n  -H 'X-Note: it'\\''s' \\\n  --data-raw '{\"name\":\"O'\\''Brien\"}'"
        );
    }

    #[test]
    fn status_text_states() {
        let mut e = NetEntry::sample("GET", "https://a.dev/");
        assert_eq!(status_text(&e), "200");
        e.status = None;
        assert_eq!(status_text(&e), "(pending)");
        e.error = Some("net::ERR_ABORTED".into());
        e.status = Some(200);
        assert_eq!(status_text(&e), "(failed)");
    }

    #[test]
    fn row_matches_is_case_insensitive() {
        assert!(row_matches("Content-Type", "text/html", ""));
        assert!(row_matches("Content-Type", "text/html", "content"));
        assert!(row_matches("Content-Type", "text/html", "HTML"));
        assert!(!row_matches("Content-Type", "text/html", "json"));
    }
}
