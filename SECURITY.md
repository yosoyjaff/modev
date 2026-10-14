# Security Policy

## Reporting a vulnerability

Please do not open public issues for security problems. Report them privately through GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability):
on the repository, go to **Security → Report a vulnerability**.

Include the modev version or commit, your OS, the Chrome/Chromium version, and steps to reproduce. You should get a
first response within a week. Fixes are released on `main`; there are no maintained release branches yet.

## Scope and things to know

- **modev launches Chrome.** It starts a headless Chrome/Chromium process controlled over the DevTools Protocol and
  loads whatever URL you give it. Treat it like a browser: only open sites you trust. The inspector can run
  JavaScript in the page (Console) and replay requests with the page's cookies (Network → Replay).
- modev stores its configuration as plain JSON in `~/.config/modev/`. It does not store credentials.
