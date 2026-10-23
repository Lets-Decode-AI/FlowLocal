# Releasing FlowLocal

Pushing a tag that matches `v*` runs `.github/workflows/release.yml`. The workflow builds all
four targets with `tauri-apps/tauri-action` and creates a **draft** GitHub Release with the installers and
auto-generated release notes attached. A maintainer reviews the draft and publishes it.

## Steps

1. Check that CI passes on `main`.
2. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (keep them equal), and run
   `cargo check` in `src-tauri` so `Cargo.lock` updates. Commit as `chore: release vX.Y.Z`.
3. Tag and push: `git tag vX.Y.Z && git push origin main vX.Y.Z`. A tag containing `-` (for example `v0.2.0-rc.1`) is marked as a pre-release.
4. Wait for the four matrix jobs (macOS arm64, macOS x64, Windows x64, Linux x64) to finish.
5. Open the draft release and check the assets (two `.dmg` files, the NSIS `.exe`, the `.AppImage` and the `.deb`) and the notes. Then publish.

## Artifacts

| Target | Runner | Bundles |
|---|---|---|
| `aarch64-apple-darwin` | `macos-14` | `.dmg` |
| `x86_64-apple-darwin` | `macos-15-intel` | `.dmg` |
| `x86_64-pc-windows-msvc` | `windows-latest` | NSIS `-setup.exe` |
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` | `.AppImage`, `.deb` |

GitHub retired `macos-13`, so the Intel build uses `macos-15-intel`.

## Code signing secrets (all optional)

With none of these set, the workflow still runs and produces **unsigned** installers. macOS shows a Gatekeeper
warning, and Windows SmartScreen may warn. Add the secrets under Settings > Secrets and variables > Actions.

| Secret | Purpose |
|---|---|
| `APPLE_CERTIFICATE` | Base64 of the "Developer ID Application" `.p12` certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Password of that `.p12` |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple ID used for notarization |
| `APPLE_PASSWORD` | App-specific password for that Apple ID |
| `APPLE_TEAM_ID` | Apple developer team ID |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater signing key (only needed once auto-update is added) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for that key |

Create the certificate value with `base64 -i certificate.p12 | pbcopy`. The workflow exports each variable only when
the matching secret is not empty. For notarization, set `APPLE_SIGNING_IDENTITY` together with `APPLE_ID`,
`APPLE_PASSWORD` and `APPLE_TEAM_ID`. See the [Tauri macOS signing guide](https://tauri.app/distribute/sign/macos/).
Windows code signing is not set up yet.

## Before the first release

- Create the `Lets-Decode-AI/maintainers` team (referenced by `.github/CODEOWNERS`) and enable Discussions and private vulnerability reporting.
- Optionally add a conduct email address to `CODE_OF_CONDUCT.md`. For now, reports go through GitHub Security Advisories or the maintainers in `CODEOWNERS`.
- Run through `docs/E2E-CHECKLIST.md` on a release build.
