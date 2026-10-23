# Security Policy

## Supported versions

FlowLocal is pre-1.0. Security fixes are made against the latest release and `main`.

## Reporting a vulnerability

Do not open a public issue for security problems.

Report privately through GitHub Security Advisories:
<https://github.com/Lets-Decode-AI/FlowLocal/security/advisories/new>
(repository **Security** tab, then **Report a vulnerability**).

Include what you found, steps to reproduce, the affected version and OS, and the impact you expect.
We aim to acknowledge reports within a few days. We will work with you on a fix and on disclosure.

## Scope notes

FlowLocal processes audio on your device and stores history in a local SQLite database. We are most interested in reports about:
the model downloader (integrity checks and redirects), the Hugging Face token (kept in the system keychain), clipboard handling,
and the Tauri commands exposed to the webview.
