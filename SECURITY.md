# Security Policy

## Supported versions

FlowLocal is pre-1.0. Security fixes are made against the latest release and `main`.

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

Report privately through GitHub Security Advisories:
<https://github.com/Lets-Decode-AI/FlowLocal/security/advisories/new>
(repository **Security** tab > **Report a vulnerability**).

Include what you found, steps to reproduce, the affected version and OS, and the impact you expect.
We aim to acknowledge reports within a few days and will coordinate a fix and disclosure with you.

## Scope notes

FlowLocal processes audio locally and stores history in a local SQLite database. Areas of particular interest:
the model downloader (integrity checks, redirects), the Hugging Face token (kept in the OS keychain), clipboard handling,
and the Tauri command surface exposed to the webview.
