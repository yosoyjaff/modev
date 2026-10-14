# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The inspector button next to the URL bar is gone (the inspector opens from the floating panel).

### Added

- Updates install from Settings and modev offers to restart: macOS replaces the app from the dmg, Windows runs the
  msi on restart, Linux updates the deb/rpm (asks for the password) or the tarball's binary.

## [0.1.1] - 2026-10-14

### Changed

- The search button next to the URL bar loads the typed URL, like Enter; the inspector button now has a code icon.

### Fixed

- macOS: the web no longer shows at a quarter of its size in the top-left corner on Retina screens.

## [0.1.0] - 2026-10-14

### Added

- Simulator window rendering a URL in headless Chrome with device emulation: CSS viewport, DPR, safe-area insets,
  user agent, platform and touch input.
- Phone and tablet presets (from Firefox's device list), user-defined devices, hiding/restoring presets, and a
  free-size Responsive mode with drag handles.
- URL bar, reload, home, page title and favicon.
- Inspector window with Chrome DevTools-style tabs:
  - Console: logs, exceptions, filtering, preserve log, JavaScript evaluation.
  - Network: request table with filters and columns; headers, payload, preview (JSON, images, PDF), response and
    timing; search; JWT decoding; Copy as cURL; HAR export; Replay of editable requests; throttling; disable cache.
  - Application: Local/Session storage, IndexedDB, cookies and Cache storage.
- Tools panel: page `prefers-color-scheme`, PNG screenshots (file + clipboard), countdown timer, MP4 recording with
  ffmpeg with optional touch indicators.
- Settings: light/dark/auto theme, capture folder, remember last device.
- App icons for macOS, Windows and Linux; native traffic lights on macOS.
- Update check against GitHub releases (on launch and from Settings).
- Release workflow: macOS dmg, Windows msi and zip, Linux deb, rpm and tar.gz, with SHA256SUMS and latest.json.

### Fixed

- Cookie values with a `%` followed by non-ASCII text no longer crash the Application tab.

[Unreleased]: https://github.com/yosoyjaff/modev/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/yosoyjaff/modev/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/yosoyjaff/modev/releases/tag/v0.1.0
