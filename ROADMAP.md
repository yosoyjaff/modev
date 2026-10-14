# Roadmap

Direction, not promises. No dates. Open an issue to discuss any item before starting on it.

## Short term

- **More tests**: `storage.rs` parsing and engine helpers; UI-free logic in `ui/net_tools.rs`, `device.rs`,
  `settings.rs` and `updater.rs` is already covered.
- **Configurable `Accept-Language`**: it is hard-coded to `es-ES` in `engine::apply_device`.
- **Clearer startup errors**: if Chrome is missing or fails to launch, the engine stops with a status message only;
  show an actionable error (install Chrome / set its path) and allow a retry.
- **Chrome path setting**: let users pick the Chrome/Chromium executable.

## Medium term

- **Inspector: Elements tab** (DOM tree, computed styles, box model).
- **Inspector: Sources tab**.
- **In-app update install**: the update check only links to the package; download, verify against `SHA256SUMS`
  and install from the app.
- **Packaging**: code signing on Windows, AppImage/Flatpak on Linux, Homebrew/winget manifests. The release
  workflow already builds dmg (signing/notarization when the secrets are set), msi, zip, deb, rpm and tar.gz.
- **Split large modules**: `ui.rs` (~1,900 lines) and `ui/inspector.rs` (~2,000) into per-panel/per-tab files to
  reduce merge conflicts and make reviews easier.
- **Manual testing on macOS and Windows**, and fixes for what turns up (window controls, transparency, Chrome
  discovery).
- **Device presets**: refine ppi and safe-area values (currently approximated).

## Long term

- Multiple simulators side by side (several devices on the same URL).
- Device orientation (portrait/landscape) and more emulation (geolocation, network conditions per request).
- Integration tests against a real headless Chrome (CDP contract tests for the gotchas in ARCHITECTURE.md).
