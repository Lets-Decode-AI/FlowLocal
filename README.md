# FlowLocal

[![CI](https://github.com/Lets-Decode-AI/FlowLocal/actions/workflows/ci.yml/badge.svg)](https://github.com/Lets-Decode-AI/FlowLocal/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/github/license/Lets-Decode-AI/FlowLocal)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/Lets-Decode-AI/FlowLocal?include_prereleases)](https://github.com/Lets-Decode-AI/FlowLocal/releases/latest)

**Private push-to-talk dictation that runs on your computer.**

Hold a shortcut in any app, speak, and let go. Punctuated text appears at your cursor. FlowLocal is an
open-source alternative to tools like Wispr Flow. The difference is that speech recognition runs
**on your computer** with [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) models such as NVIDIA
Parakeet. Your audio never leaves your machine, and there is no account or subscription.

It is a small [Tauri 2](https://tauri.app) app (Rust core, React UI) that lives in the system tray.
The installer includes everything the app needs, including the ONNX Runtime libraries. You download
models from Hugging Face inside the app.

> Status: Part 1 (core dictation and the local model manager) is done. See the [Roadmap](#roadmap).

## Features

- Push-to-talk and hands-free dictation into any focused text field. Double-press the shortcut for hands-free, and press `Esc` to cancel.
- Speech-to-text with sherpa-onnx. Transcription does not use the network.
- In-app model manager with a curated library, Hugging Face search, resumable downloads, SHA-256 verification, pause, resume, cancel and disk usage per model.
- A model that suits your hardware is recommended on first run.
- Streaming chunked transcription, so long dictations finish soon after you let go of the key.
- Flow Bar: a small pill on top of other windows that shows when FlowLocal is listening, transcribing or done, and shows errors. It never takes focus.
- Text is inserted by pasting, and your clipboard (text or image) is restored afterwards. Only plain text and images are
  restored. Rich formatting (HTML or RTF) comes back as plain text. If the clipboard held something FlowLocal cannot read,
  such as files, FlowLocal leaves it alone instead of clearing it.
- Personal dictionary replacements, plus basic capitalization and spacing fixes.
- Searchable history of your dictations, stored in SQLite on your machine.
- Low resource use. FlowLocal stays in the tray at rest, and the model unloads after an idle timeout (10 minutes by default, or never).
- Optional Hugging Face token for gated or private repos, stored in the system keychain.
- Open at login, microphone selection and a configurable shortcut.

## Privacy

- Audio is processed on your device and is never uploaded; there is no account and no telemetry.
- The microphone stays open for up to 15 s after a dictation so the next one does not clip the first word. While it is open, only the latest 0.3 s of audio is held in memory.
  After 15 s it closes.
- The only network traffic is model downloads and searches on Hugging Face, which you start yourself.
- Custom Hugging Face repos: files tracked with Git LFS are checked against their SHA-256. Other files
  (such as `tokens.txt`) are checked for size only, because Hugging Face publishes no hash for them.

## Screenshots

_Screenshots will be added here. Contributions are welcome._

<!-- Add images under docs/images/ and reference them here. -->

## Install

Download the installer for your system from the
[Releases page](https://github.com/Lets-Decode-AI/FlowLocal/releases/latest).

| OS | File | Notes |
|---|---|---|
| macOS (Apple Silicon) | `FlowLocal_<version>_aarch64.dmg` | |
| macOS (Intel) | `FlowLocal_<version>_x64.dmg` | |
| Windows (x64) | `FlowLocal_<version>_x64-setup.exe` | NSIS installer |
| Linux (x64) | `.AppImage` or `.deb` | |

Models are not bundled. On first launch, setup downloads a recommended model
(Parakeet TDT 0.6B v2, about 660 MB, by default).

### macOS

Release builds may be **unsigned**. They are signed and notarized only if the maintainers have
configured signing certificates. If macOS says the app "is damaged" or "cannot be opened because
the developer cannot be verified":

1. Drag FlowLocal to `/Applications`.
2. Try to open it once. Then go to System Settings > Privacy & Security, scroll to Security and click **Open Anyway** (macOS 15 and later).
3. Or run `xattr -dr com.apple.quarantine /Applications/FlowLocal.app` in Terminal.

FlowLocal needs two permissions. Setup walks you through them (System Settings > Privacy & Security):

- **Microphone**: records your voice.
- **Accessibility**: lets FlowLocal read the global shortcut (the `Fn` key is read through an event tap) and send the paste keystroke.
  Until you grant it, FlowLocal shows "Grant Accessibility to enable the hotkey". The shortcut starts working as soon as
  you grant it. You do not need to relaunch.

If you use the `Fn` or Globe key as the shortcut, set System Settings > Keyboard > "Press 🌐 key to" to **Do Nothing**.
Otherwise macOS may open the emoji picker or its own dictation at the same time.

### Windows

Run the installer. For unsigned builds, Windows SmartScreen may warn about an unrecognized publisher
(More info > Run anyway). The default shortcut is to hold `Ctrl+Win`.

### Linux

- **X11**: supported. The default shortcut is to hold `Ctrl+Super`.
- **Wayland**: the global shortcut is not available yet, and the app shows "Global hotkey unavailable on Wayland".
  Use an X11 session for now. Where synthetic paste is blocked, the text stays on the clipboard and you paste it yourself.
  Support through the XDG GlobalShortcuts portal is on the [Roadmap](#roadmap).
- Make the AppImage executable (`chmod +x FlowLocal_*.AppImage`) or install the `.deb` with `sudo apt install ./FlowLocal_*.deb`.

## Supported models

The library is bundled in the app ([`src-tauri/resources/catalog.json`](src-tauri/resources/catalog.json)),
with sizes and SHA-256 hashes pinned to specific Hugging Face revisions. You can also search Hugging Face
from the app for other sherpa-onnx repos. FlowLocal detects compatible transducer, Whisper, Moonshine and SenseVoice layouts.

| Model | Engine | Download | Languages | Min RAM |
|---|---|---|---|---|
| Parakeet TDT 0.6B v2 (int8), **default** | Transducer | ~661 MB | English | 4 GB |
| Parakeet TDT 0.6B v3 (int8) | Transducer | ~670 MB | 25 European languages | 4 GB |
| Moonshine Base (int8) | Moonshine | ~286 MB | English | 2 GB |
| Whisper tiny.en (int8) | Whisper | ~103 MB | English | 2 GB |
| Whisper small.en (int8) | Whisper | ~375 MB | English | 3 GB |
| SenseVoice small (int8) | SenseVoice | ~239 MB | Chinese, English, Japanese, Korean, Cantonese | 2 GB |
| Whisper large-v3-turbo (int8) | Whisper | ~1 GB | Multilingual (35 languages listed in the catalog) | 6 GB |

On first run FlowLocal recommends Moonshine Base on machines with little RAM, Parakeet v3 or SenseVoice when they match
your system language, and Parakeet v2 otherwise.

### Measured performance

From the project's integration test (Parakeet v2 int8, release build, Apple Silicon Mac). Your hardware will differ.

- Clips of 4 to 6 s transcribe in roughly 360 to 530 ms. The best run took about 362 ms for a 5 s clip.
- A 62.5 s streamed dictation finishes about 421 ms after key-up.
- Word error rate was 0.0 on the project's synthetic text-to-speech fixtures. The set is small and is not a general accuracy benchmark.

## How streaming chunked transcription works

Transcribing a long recording in one pass would make you wait after you release the key. Instead, while you
speak, a **chunk planner** watches the live 16 kHz audio and cuts it at natural pauses:

- Once a chunk is at least 4 s long, it is cut at the first pause of 300 ms or more.
- If a chunk reaches 15 s without a pause, it is cut at the quietest 20 ms frame in the last 3 s.
- Finished chunks go to a single STT worker thread, which transcribes them **in order** in the background.
- Silent chunks are skipped.

When you release the key, only the short final chunk is left. The wait after key-up depends on that chunk, not on how
long you talked. FlowLocal joins the chunk texts in order, post-processes them and pastes the result.

## Architecture

```mermaid
flowchart LR
    HK[hotkey] -->|Pressed / Released / DoubleTap / Cancel| DC[dictation controller]
    DC --> AU[audio recorder<br/>cpal, 300 ms pre-roll]
    AU -->|16 kHz mono frames| CP[chunk planner]
    CP -->|chunks| ST[stt worker<br/>sherpa-onnx]
    MM[models manager<br/>catalog, HF, downloader] -->|active model| ST
    ST -->|text| PP[postprocess]
    PP --> IN[insert<br/>clipboard + paste]
    DC --> DB[(store<br/>SQLite)]
    DC -->|events| UI[React UI<br/>main window + Flow Bar]
    UI -->|commands| DC
    UI -->|commands| MM
```

One Rust process owns audio, inference, hotkeys, I/O and persistence. The web UI only renders. Module map
(all under [`src-tauri/src`](src-tauri/src)):

| Module | Responsibility |
|---|---|
| `hotkey` | Global triggers (macOS `Fn` event tap, `rdev`, Wayland portal stub) and the tap and double-tap gesture logic |
| `audio` | `cpal` capture, resampling to 16 kHz mono, energy VAD, level meter |
| `dictation` | Pure state machine plus the controller that wires audio, STT, post-processing, insertion and storage |
| `stt` | `ChunkPlanner`, `SttSession`, the worker thread and the sherpa-onnx engine |
| `models` | Catalog, Hugging Face search, resumable verified downloader, hardware detection, lifecycle |
| `postprocess` | Spacing and capitalization fixes, dictionary replacements |
| `insert` | Clipboard save and restore, synthetic paste, frontmost-app lookup |
| `store` | SQLite: transcripts, settings, dictionary, downloads, migrations |
| `permissions` | Per-OS permission checks and deep links |
| `commands`, `events`, `tray`, `app_state` | Tauri command layer, event names, tray menu, shared state |

The React/TypeScript UI is in [`src`](src) (`flowbar.html` is the Flow Bar window).

## Build from source

Prerequisites: [Rust](https://rustup.rs) (the toolchain is pinned to 1.99.1 by `rust-toolchain.toml`; Rust 1.88 or newer is required), Node.js 20 or newer,
and the platform dependencies for [Tauri 2](https://tauri.app/start/prerequisites/). On Ubuntu:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libxdo-dev libxtst-dev libxi-dev
```

```bash
git clone https://github.com/Lets-Decode-AI/FlowLocal.git
cd FlowLocal
npm ci
npm run tauri dev      # run the app with hot reload
npm run tauri build    # produce an installer for your OS in src-tauri/target/release/bundle
```

The first build downloads prebuilt sherpa-onnx binaries and compiles a lot of Rust, so it takes a while.
On macOS, grant Microphone and Accessibility to the terminal or app you launch it from when you test the shortcut.

## Running tests

```bash
# Rust unit tests
cd src-tauri && cargo test

# Lint and formatting (also run in CI)
cargo fmt --check
cargo clippy --all-targets -- -D warnings

# Frontend
cd .. && npx tsc --noEmit && npx vitest run
```

The real-model integration tests (word error rate on fixtures, streaming latency) are `#[ignore]`d because they need a
~660 MB model. To run them:

```bash
scripts/fetch_test_model.sh          # downloads Parakeet v2 int8 to ~/Library/Caches/FlowLocal Test é/parakeet-v2
cd src-tauri
FLOWLOCAL_MODEL_DIR="$HOME/Library/Caches/FlowLocal Test é/parakeet-v2" \
  cargo test --release --test stt_integration -- --ignored --nocapture
```

On Linux pass a destination directory to the script and use the same path for `FLOWLOCAL_MODEL_DIR`.
The manual end-to-end checks for dictation in real apps are listed in [docs/E2E-CHECKLIST.md](docs/E2E-CHECKLIST.md).

## Roadmap

- **Linux/Wayland**: global shortcuts through the XDG GlobalShortcuts portal.
- **Part 2, text intelligence**: local LLM cleanup (llama.cpp), an optional API key for OpenAI or Anthropic,
  Command Mode, snippets and per-app styles.
- **Part 3, dashboard**: a dashboard like Wispr's with words, WPM, streaks and time saved, plus full history.

Not planned for Part 1: auto-update, accounts or sync. Release process: [docs/RELEASING.md](docs/RELEASING.md).

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and the
[Code of Conduct](CODE_OF_CONDUCT.md) first. Report security issues privately as described in [SECURITY.md](SECURITY.md).
For questions, see [SUPPORT.md](SUPPORT.md).

## License

[MIT](LICENSE) (c) 2026 Lets Decode AI.

## Acknowledgements

- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (k2-fsa), the [`sherpa-rs`](https://github.com/thewh1teagle/sherpa-rs) bindings, and [csukuangfj](https://huggingface.co/csukuangfj) for the converted model repos.
- [NVIDIA NeMo Parakeet](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) speech models.
- [OpenAI Whisper](https://github.com/openai/whisper), [Moonshine](https://github.com/moonshine-ai/moonshine) and [SenseVoice](https://github.com/FunAudioLLM/SenseVoice) models.
- [Tauri](https://tauri.app) for the app framework.
- Inspired by [Wispr Flow](https://wisprflow.ai). FlowLocal is an independent project and is not affiliated with Wispr.

Model weights are downloaded from their upstream repositories and remain under their own licenses.
