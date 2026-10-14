# Contributing to modev

Thanks for your interest. Bug reports, fixes and features are welcome. For larger changes, open an issue first so
we can agree on the approach. Some design decisions are intentional (see [ARCHITECTURE.md](ARCHITECTURE.md#design-decisions)).

## Setup

1. Install Rust (stable), Google Chrome or Chromium, and optionally `ffmpeg`.
2. On Linux, install GPUI's system libraries (see [README.md](README.md#build-from-source)).
3. Start any dev server (e.g. `ionic serve`, `npm run dev`) and run:

   ```sh
   cargo run -- http://localhost:8100
   ```

Read [ARCHITECTURE.md](ARCHITECTURE.md) before touching `engine.rs` or `ui.rs`. `CLAUDE.md` holds the same notes in
condensed form for coding agents.

## Before opening a PR

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check   # optional: licenses and advisories (cargo install cargo-deny)
```

CI runs the same checks on Linux, macOS and Windows, plus an MSRV build and `cargo deny`.

## Code style

- Minimal, focused changes; follow the surrounding code.
- UI is built with GPUI `div()` builders and `cx.listener` callbacks.
- Colors go through `theme::rgb` (not `gpui::rgb`) with a palette token. When you add a color, add its light-mode
  pair in `theme.rs`.
- Icons are Hugeicons SVGs in `assets/icons/`, registered in `icons.rs`.
- Comments and UI text in English. Comment only when the *why* is not obvious.
- Engine ↔ UI communication goes through `engine::Command` / `engine::Event`; keep CDP code in the engine (or
  `storage.rs`) and pure logic in UI-free modules such as `ui/net_tools.rs`, where it can be unit-tested.

## Debugging CDP

To try CDP calls without rebuilding modev, start Chrome with a debugging port and talk to it from a Node 22 script
(Node 22 has a native `WebSocket`):

```sh
google-chrome --headless=new --remote-debugging-port=9222 about:blank
```

```js
// cdp.mjs — node cdp.mjs
const targets = await (await fetch('http://localhost:9222/json')).json();
const ws = new WebSocket(targets.find(t => t.type === 'page').webSocketDebuggerUrl);
let id = 0;
const send = (method, params = {}) => ws.send(JSON.stringify({ id: ++id, method, params }));
ws.onmessage = e => console.log(e.data);
ws.onopen = () => {
  send('Network.enable');
  send('Page.navigate', { url: 'http://localhost:8100' });
};
```

The [CDP reference](https://chromedevtools.github.io/devtools-protocol/) lists every method. Chrome answers
`-32602` when params are malformed (often a field chromiumoxide omits or serializes differently).

## Branches, commits and PRs

- Branch from `main` (`feat/…`, `fix/…`, `docs/…`).
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/): `feat: …`, `fix: …`,
  `docs: …`, `refactor: …`, `chore: …`.
- Keep PRs focused. Describe what changed and how you tested it, and add a screenshot for UI changes.
- Add an entry under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) for user-visible changes.

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE) and that you
follow the [Code of Conduct](CODE_OF_CONDUCT.md).
