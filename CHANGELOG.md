# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- App icons for macOS, Windows and Linux.
- Update check against GitHub releases (on launch and from Settings).
- Release workflow: macOS dmg, Windows msi and zip, Linux deb, rpm and tar.gz, with SHA256SUMS and latest.json.

### Fixed

- Cookie values with a `%` followed by non-ASCII text no longer crash the Application tab.
