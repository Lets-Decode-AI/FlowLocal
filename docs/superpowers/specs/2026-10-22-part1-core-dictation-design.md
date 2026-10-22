# Part 1 — Core Dictation & Local Model Manager (Design)

Date: 2026-10-22
Status: Approved in brainstorming; pending written-spec review

## 1. Context

We are building a lightweight, cross-platform clone of Wispr Flow: press a hotkey
in any application, speak, and accurate text is inserted at the cursor. The key
difference from Wispr Flow is that speech-to-text runs **locally** on the user's
hardware using models downloaded from Hugging Face inside the app. The app must
work out of the box: the installer bundles every runtime; nothing is installed
outside the app.

The full product is split into three sub-projects, each with its own
spec → plan → build cycle:

1. **Part 1 (this spec):** core dictation engine + local model manager.
2. **Part 2:** text intelligence — local LLM cleanup (llama.cpp), optional
   BYO API key (OpenAI/Anthropic), Command Mode, snippets, per-app styles.
3. **Part 3:** Wispr-style dashboard — stats (words, WPM, streak, time saved),
   full history, polished settings and onboarding.

### Goals (Part 1)

- Global push-to-talk and hands-free dictation into any focused text field on
  macOS, Windows and Linux (X11 fully; Wayland best-effort).
- In-app browsing, downloading and managing of sherpa-onnx compatible STT models
  from Hugging Face, with a hardware-appropriate default fetched on first run.
- Very low resource use: tray-only at rest, model unloaded when idle.
- Persist each dictation (text + metadata) to a local database that Part 3 reads.

### Non-goals (Part 1)

- LLM cleanup, Command Mode, snippets, per-app styles, API keys (Part 2).
- Dashboard analytics UI (Part 3) — Part 1 ships only a basic History list.
- Code signing / notarization, auto-update, accounts, sync, teams, billing.

### Success criteria

- On this Mac (Apple Silicon), holding the hotkey, saying a sentence and
  releasing inserts correctly punctuated text into TextEdit, a browser textarea,
  VS Code and Slack, and the user's clipboard is restored afterwards.
- Parakeet TDT 0.6B v2 transcription of a ~5 s utterance completes in < 500 ms
  on Apple Silicon.
- Idle RAM (tray-only, model unloaded) < 60 MB; installer < 25 MB (excluding models).
- A fresh install reaches first dictation with only in-app steps
  (permissions + one-click model download).
- CI builds succeed for macOS (arm64, x64), Windows (x64) and Linux (x64).

## 2. Technology

| Concern | Choice |
|---|---|
| Shell | Tauri 2 (Rust core + OS webview) |
| UI | React + TypeScript + Tailwind, Vite |
| STT runtime | sherpa-onnx C API (via Rust bindings, e.g. `sherpa-rs`), ONNX Runtime; CPU int8 default, CoreML (macOS) / DirectML (Windows) execution providers where available |
| Audio | `cpal` capture, resampled to 16 kHz mono f32 |
| Hotkeys | `rdev` / platform hooks; macOS `Fn` via CGEventTap; Linux Wayland via XDG GlobalShortcuts portal when present |
| Insertion | `arboard` clipboard + `enigo` synthetic keystroke |
| Downloads | `reqwest` (rustls), HTTP Range resume, `sha2` |
| Storage | `rusqlite` (bundled SQLite) |
| Secrets | `keyring` (HF token) |
| Packaging | Tauri bundler: `.dmg` (arm64, x64), NSIS `.exe` (x64), `.AppImage`/`.deb` (x64) |
| CI | GitHub Actions matrix build |

Developer prerequisite only: Rust toolchain via `rustup`. End users install nothing.

## 3. Architecture

Single Rust process. The web UI renders only; all audio, inference, hotkeys,
I/O and persistence live in Rust.

```
Rust core
├── hotkey      global triggers → DictationEvent
├── audio       cpal capture → ring buffer (with 300 ms pre-roll) + RMS levels
├── dictation   state machine; orchestrates audio, stt, postprocess, insert, store
├── stt         SttEngine trait; sherpa-onnx backend on a dedicated thread
├── models      catalog, HF search, downloader, disk management, hw detection
├── postprocess spacing/capitalization fixes + dictionary replacements
├── insert      Inserter trait; clipboard-paste implementation per OS
├── store       SQLite: transcripts, settings, dictionary
└── permissions per-OS permission checks + deep links

Windows (Tauri webviews)
├── flowbar     small transparent always-on-top pill, never focused
└── main        onboarding / Models / History / Settings; created lazily,
                destroyed on close (app lives in the tray)
```

### Module interfaces

- `hotkey`: emits `Pressed`, `Released`, `DoubleTap`, `Cancel` on a channel.
  Bindings come from settings and are hot-reloadable.
- `audio::Recorder`: `start()`, `stop() -> Vec<f32>` (16 kHz mono), `cancel()`;
  emits `level(f32)` at ~30 Hz while recording.
- `stt::SttEngine` (trait): `load(model: &InstalledModel) -> Result<()>`,
  `transcribe(samples: &[f32]) -> Result<String>`, `unload()`. One
  implementation (`SherpaEngine`) dispatching on engine kind: `Transducer`
  (Parakeet), `Whisper`, `Moonshine`, `SenseVoice`. Runs on its own thread
  reached via a command channel; panics are caught with `catch_unwind`.
- `insert::Inserter` (trait): `insert(text: &str) -> InsertOutcome`
  (`Pasted` | `CopiedOnly(reason)`).
- `models::Manager`: `catalog()`, `search_hf(query)`, `install(id)`,
  `pause/resume/cancel(id)`, `delete(id)`, `installed()`, `active()`,
  `set_active(id)`, `recommend(hw) -> ModelId`. Emits progress events.
- `store::Store`: `insert_transcript(..)`, `list_transcripts(page, query)`,
  `delete_transcript(id)`, settings get/set, dictionary CRUD.

UI ↔ core communication uses Tauri commands (request/response) and Tauri
events (progress, state, levels). The TypeScript IPC layer is generated or
hand-typed in a single `ipc.ts` module.

## 4. Model manager

### Catalog

A JSON manifest compiled into the binary (and refreshable from the repo later).
Each entry: `id`, display name, `hf_repo`, `revision`, list of `{path, size,
sha256}`, `engine_kind`, languages, RAM requirement, and notes. Only the files
the engine needs are downloaded (int8 weights + `tokens.txt`), never the full repo.

| Model | HF repo (csukuangfj/…) | Download | Languages |
|---|---|---|---|
| Parakeet TDT 0.6B v2 int8 (**default**) | sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 | ~661 MB | English |
| Parakeet TDT 0.6B v3 int8 | sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8 | ~671 MB | 25 European |
| Moonshine Base int8 | sherpa-onnx-moonshine-base-en-int8 | ~288 MB | English |
| Whisper tiny.en int8 | sherpa-onnx-whisper-tiny.en | ~105 MB | English |
| Whisper small.en int8 | sherpa-onnx-whisper-small.en | ~375 MB | English |
| SenseVoice small int8 | sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17 | ~240 MB | zh/en/ja/ko/yue |
| Whisper large-v3-turbo int8 | sherpa-onnx-whisper-turbo | ~1 GB | 99 languages |

The sha256 of each file is resolved from the HF API (`?blobs=true` LFS oid) at
catalog-build time and pinned in the manifest.

### Hugging Face search

Queries `https://huggingface.co/api/models?search=<q>&filter=sherpa-onnx` (plus
unfiltered fallback). For a selected repo, file listing determines engine kind:

- `encoder*.onnx` + `decoder*.onnx` + `joiner*.onnx` → Transducer
- `*-encoder*.onnx` + `*-decoder*.onnx` → Whisper
- `preprocess.onnx` + `encode*.onnx` + `*decode*.onnx` → Moonshine
- single `model*.onnx` + `tokens.txt` with SenseVoice tags → SenseVoice
- anything else → shown as "incompatible" (not installable)

int8 variants are preferred when present. Sha256 for searched repos comes from
the API at install time.

### Downloader

- Streams each file to `<id>/<file>.part` with HTTP Range resume.
- Verifies sha256, then atomically renames. Mismatch → delete `.part`, mark
  failed, allow retry.
- Pause / resume / cancel; download state persisted so it survives restarts.
- Emits progress (bytes, total, speed, ETA) at ≤ 10 Hz.
- Optional HF token (gated/private repos), stored in OS keychain.
- Storage: `<app-data>/models/<id>/`; per-model disk usage shown; delete frees disk.

### Hardware detection & recommendation

Detect total RAM, CPU core count, CPU arch, Apple Silicon, GPU vendor (best effort).

- RAM ≤ 4 GB → Moonshine Base.
- OS locale not English and in Parakeet v3 language set → Parakeet v3.
- OS locale CJK → SenseVoice.
- Otherwise → Parakeet v2.

### Activation & lifecycle

- Exactly one active model. Switching hot-swaps on the STT thread; an in-flight
  dictation completes on the previous model.
- Model loads on first dictation (or at startup if "preload" is on) and unloads
  after an idle timeout (default 10 min, configurable, "never" allowed).

## 5. Dictation flow

### Triggers (rebindable)

| Action | macOS default | Windows/Linux default |
|---|---|---|
| Hold to talk | `Fn` | `Ctrl+Win` (Super) |
| Hands-free toggle | double-tap the same key | double-tap |
| Cancel | `Esc` | `Esc` |

Taps shorter than 250 ms, and recordings with no detected speech (energy VAD),
are discarded silently. Hands-free recordings auto-stop after 6 minutes.

### State machine

```
Idle ──Pressed──▶ Recording ──Released──▶ Transcribing ──ok──▶ Inserting ──▶ Idle
  ▲                  │ DoubleTap → Locked (hands-free) ─Pressed─▶ Transcribing
  │                  │ Cancel / too short / no speech ───────────────────────▶ Idle
  └──────── Error (shown 2 s) ◀── any failure
```

The dictation state machine is a pure struct driven by events and fakes, so it
is unit-testable without audio, models or OS hooks.

### Pipeline

1. Key down → recorder starts (ring buffer keeps 300 ms pre-roll).
2. Key up → stop; trim leading/trailing silence via VAD; send to STT thread.
3. Post-process: whitespace and capitalization fixes, leading-space logic,
   personal dictionary replacements (case-insensitive whole-word).
4. Insert: record frontmost app → save clipboard → write text → synthetic
   ⌘V / Ctrl+V → restore clipboard after ~150 ms.
5. Persist: transcript text, raw text, app name, timestamp, audio duration, word
   count, WPM, model id.

### Flow Bar

Bottom-center of the active screen, transparent, always on top, never takes focus.
States: Idle (dim handle, hideable) → Recording (expanded, live waveform from
level events) → Transcribing (shimmer) → Done (✓, collapses) → Error (red,
one-line reason).

## 6. UI (Part 1 scope)

Main window pages, minimal and clean (Wispr-like visual language, light/dark):

- **Onboarding:** welcome → permissions (mic, accessibility on macOS) with live
  status → recommended model download with progress → try-it field.
- **Models:** Installed / Catalog / Search Hugging Face tabs; download progress,
  pause/resume/cancel, set active, delete, disk usage.
- **History:** paginated, searchable list of transcripts with copy and delete.
- **Settings:** hotkeys, microphone selection, Flow Bar visibility, model idle
  timeout, launch at login, HF token, personal dictionary list.

Tray menu: status, active model submenu, open dashboard, pause dictation, quit.

## 7. Error handling

| Situation | Behavior |
|---|---|
| Missing mic / accessibility permission (macOS) | Onboarding & Settings deep-link to System Settings pane; poll status each second; Flow Bar error if triggered without it |
| No model installed | Flow Bar error "No model installed" + open Models page |
| Secure input / paste blocked | Text left on clipboard; toast "Copied — press ⌘V"; transcript still saved |
| Wayland | Hotkeys via GlobalShortcuts portal if available; insertion falls back to clipboard + notification |
| STT panic / load failure | Caught; one automatic reload; then model marked failed with message |
| Download network error | Auto-retry with backoff (3×), then paused with error; resumable |
| Checksum mismatch | Delete partial file, mark failed, offer retry |
| Disk full | Detected before download (free space check) and on write error |

## 8. Data

SQLite at `<app-data>/flow.db`.

- `transcripts(id, created_at, text, raw_text, app_name, duration_ms, word_count, wpm, model_id)`
- `settings(key, value_json)`
- `dictionary(id, term, replacement NULL)`
- `downloads(model_id, file, bytes_done, total, status)`

Schema migrations via a simple versioned migration table.

## 9. Testing

- **Rust unit tests:** dictation state machine (fake engine / inserter / clock),
  downloader resume + checksum (local test HTTP server), catalog parsing, HF repo
  engine detection, post-processing, resample + VAD trim.
- **Integration test:** real Parakeet model on bundled WAV fixtures; assert WER
  below threshold (skipped in CI if model not cached).
- **Frontend:** Vitest for IPC hooks and key components.
- **Manual (this Mac):** TextEdit, browser textarea, VS Code, Slack — insertion
  and clipboard restore.
- **CI:** GitHub Actions builds for macOS arm64/x64, Windows x64, Linux x64.
  Windows and Linux runtime behavior is not manually verified in Part 1.

## 10. Open risks

- `sherpa-rs` coverage of all four engine kinds; fallback is calling the
  sherpa-onnx C API directly via `bindgen`.
- macOS `Fn` key capture requires an event tap and Accessibility permission;
  fallback default is Right-⌥ if the tap is unavailable.
- Synthetic paste can be blocked by some apps (secure input, some games/remote
  desktops); mitigated by the clipboard fallback.
