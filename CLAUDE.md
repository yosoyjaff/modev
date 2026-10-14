# modev

Desktop tool (Rust + GPUI) for developing Ionic/Angular apps: it shows the web in an emulated "device" as faithfully as possible (viewport, DPR, safe areas, UA, touch), with a native Network and Console inspector.

## Commands

- Verify: `cargo check` — do **not** run `cargo build`/`cargo run`; the user compiles and tests himself.
- Run (user): `cargo run -- http://localhost:8100` (default URL).
- App icons: `assets/app-icon/` (macOS `.icns` for `cargo bundle --release`, Windows `.ico` embedded by `build.rs` with `winresource`, Linux hicolor + `.desktop`); source image `assets/logo/logo-iOS-Dark-1024x1024@1x.png`.

## Architecture

- `src/engine.rs` — headless Chrome driven by CDP (`chromiumoxide`) in its own tokio runtime (separate thread). Communication with the UI through channels: `Command` (UI → engine) and `Event` (engine → UI). It emulates the device (`setDeviceMetricsOverride`, `setSafeAreaInsetsOverride`, `setUserAgentOverride`, touch), streams the screen with `Page.startScreencast` (PNG → BGRA → `RenderImage`; JPEG left color halos on text at 1×) and listens to Network/Runtime.
- `src/device.rs` — device profiles (CSS viewport, DPR, ppi, safe areas, UA). Presets = phones and tablets from Firefox (remote-settings `devtools-devices` collection; ppi/safe areas approximated); deleted ones go to `~/.config/modev/hidden-devices.json` ("Restore" brings them back). User devices in `~/.config/modev/devices.json`; the `Responsive` profile is free-size (fields and drag handles) and is applied with `Command::Resize` without reloading.
- `src/ui.rs` — `Modev`: **shared state** (entity, not view) + render of the simulator and floating panel. `SimulatorView`: a single transparent window with a bar (draggable title, minimize/close, reload, home, URL), the web and the right floating panel (Tools / Network / Console / Device / Android / Settings). `Screen`: separate entity for the frames (so the inspector doesn't re-render at 30-60 fps).
- `src/ui/net_tools.rs` — UI-free logic of the Network detail: JWT, HAR, search.
- `src/ui/inspector.rs` — `InspectorView`: on-demand window with native Chrome DevTools-style tabs (Console, Network). The `InputState`s live here (they need their `Window`); the state still lives in `Modev`.
- `src/storage.rs` + `src/ui/inspector/application.rs` — Application tab (Storage, Local/Session storage, IndexedDB, Cookies, Cache storage) with raw CDP (`Raw`); `Command::Storage` runs in another tokio task and answers `Event::Storage`.
- `src/ui/tools.rs` — floating panel tabs Tools (`prefers-color-scheme` of the web, PNG screenshot → file + clipboard, timer, MP4 recording with `ffmpeg`) and Settings (modev theme, capture folder, remember device, update check, about).
- `src/settings.rs` — `~/.config/modev/settings.json`.
- `src/adb.rs` + `src/ui/android.rs` — Android tab (like chrome://inspect/#devices): own thread speaking the adb server protocol (127.0.0.1:5037; `adb start-server` if it is down, so Android Studio is not needed), `host:track-devices-l`, and, only while the tab is open, the `*devtools_remote*` sockets of `/proc/net/unix` → `adb forward tcp:0` → `/json/version` + `/json/list`. "Inspect" = `Command::Attach`: the engine connects chromiumoxide to the device's browser and swaps `page` (no emulation, JPEG screencast); the local page keeps running with its events muted. `Command::Detach` / `Event::Detached` return to the emulated device.
- `src/updater.rs` — update check: reads `latest.json` from the latest GitHub release (published by `.github/workflows/release.yml`) with `ureq`; on launch if enabled, and from Settings. It only links to the package.
- `src/theme.rs` — zeron's dark palette (`~/proyectos/zeron`) and its installation into gpui-kit's theme (inputs, popups). All windows use these tokens. Light mode: `theme::rgb` (which replaces `gpui::rgb` in the UI) maps each dark token to its `LIGHT` pair; for new colors, add the pair.
- `src/icons.rs` — Hugeicons (MIT) icons embedded in `assets/icons/` (`Ico`, used with `Icon::new`); `Assets` serves them alongside gpui-kit's.
- `src/main.rs` — creates the state and opens only the simulator; the inspector is opened with `Modev::open_monitor`.

## Decisions (do not revert without asking)

- UI with GPUI + gpui-kit (`gpui_kit::*`). No phone frame: the web is a box with a border.
- Window controls like zeron: on Linux the side/order come from the desktop's `button-layout` (`cx.button_layout()`) filtered by `window.window_controls()`; **no maximize**; no macOS traffic lights.
- Simulator = one window (one dock icon); bar, web and floating panel fixed to each other. Opened with `cx.open_window` **without** gpui-kit's `Root` (it paints an opaque background and a straight frame).
- All windows equal: transparent, no `Root`, content in a `rounded(PANEL_RADIUS)` panel + `BORDER` border + `BG_PANEL` background. In the inspector the content goes 3 px in so its rectangular backgrounds don't stick out of the corners.
- Inspector: native tabs in GPUI imitating Chrome DevTools, not the real embedded frontend. Next: Elements, Sources.
- Closing the simulator closes the app (`cx.quit()`), including Chrome.

## Gotchas

- chromiumoxide prepends `--` to `.arg()`: pass args without the prefix; with a value use a tuple `("key", "value")`.
- The headless screencast captures at the virtual screen's scale (1x), not the emulated DPR: without `force-device-scale-factor` the frames arrive at CSS size and look blurry.
- Chrome headless brings `--ozone-override-screen-size=800,600` and crops the emulated viewport: pass a large `screen-info` and `ozone-override-screen-size`.
- chromiumoxide's `DispatchTouchEventParams` omits an empty `touchPoints` and Chrome rejects `touchEnd` (-32602). Use the custom `DispatchTouch` command. On another -32602, check `skip_serializing_if` in the params.
- `Page.frameNavigated` arrives after the document request: clear Network/Console when navigation starts (`is_navigation`), not there.
- Redirects reuse `requestId`: close the previous entry with its `redirectResponse`.
- Network viewers: images with `Image::from_bytes`; PDF rasterized with `hayro` (pure Rust) in `cx.background_spawn` → `NetEntry::pdf` (PNG pages).
- Don't request `getResponseBody` for everything (Vite serves multi-MB scripts): only Fetch/Xhr/Document/Other when they finish; the rest when selected.
- Wayland: the app cannot position windows; undecorated windows need a drag zone (`start_window_move`) and resize borders (`start_window_resize`).
- Scroll lag: input events were queueing up (each `dispatch*Event` waits for Chrome). The engine merges consecutive `Wheel` and `TouchMove`; frames go through a `watch` (only the latest is decoded) and the screencast is requested at the exact on-screen size, `zoom × window scale` (`Command::CaptureScale`, DPR cap, no 1× minimum): let Chrome downsample; if GPUI rescales the frame (bilinear, no mipmaps) the text looks blurry. For the same reason: the zoom is adjusted to a whole width (`fit_to`), the column is centered without a half pixel (`column_width`) and the frame is painted at its native size (1:1), not stretched to the box.
- GPUI expects BGRA; release old frames with `window.drop_image` (otherwise the atlas grows unbounded).
- Favicon with `Network.loadNetworkResource` + `IO.read` (requested by the browser): a `fetch` from the page would show up in Network.
- "Replay" (`Command::Replay`) does a `fetch()` from the page: same origin and cookies, but the browser drops forbidden headers (Cookie, Host, Origin…) and the HTTP/2 pseudo-headers `:authority` throw a TypeError (they are filtered out).
- `IndexedDB.requestData` fails (-32000) with `indexName: ""`: omit it when there is no index. Values arrive as RemoteObject: the full JSON comes from `Runtime.callFunctionOn` + `JSON.stringify` (and `releaseObject`).
- Selectable text in the inspector: `ui/selectable.rs` (`SelText`, adapted from `gpui_base::SelectableText` to support highlighting; `SelDoc` groups several texts into one document so selection spans lines). Without `Root`, the window paints its `TextSelectionLayer` and handles Ctrl+C with the `CopySelection` action (context "Inspector"); clicking selectable text removes focus from the inputs so Ctrl+C isn't captured by the input.
- To debug CDP without compiling: Node 22 script (native WebSocket) against `google-chrome --headless=new --remote-debugging-port=N`.

- chromiumoxide's `Handler` stream does not end when the websocket drops: a remote session is watched with a periodic `evaluate("1")` (`Command::RemoteClosed`).

## Style

- Comments and UI text in English; comments only where the why is not obvious.
- Minimal, surgical changes; follow the existing style (`div()` builders, `cx.listener`).
