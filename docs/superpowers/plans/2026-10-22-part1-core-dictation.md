# Part 1 — Core Dictation & Local Model Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A lightweight Tauri 2 desktop app (macOS/Windows/Linux) that dictates into any focused app with a global hotkey, using local sherpa-onnx STT models downloaded from Hugging Face inside the app.

**Architecture:** One Rust process owns hotkeys, audio, STT, downloads, insertion and SQLite; the React UI (two Tauri webviews: Flow Bar + lazily-created main window) only renders and calls Tauri commands / listens to events. Every OS- or hardware-facing piece sits behind a trait so the dictation state machine is tested with fakes.

**Tech Stack:** Rust (stable), Tauri 2, sherpa-rs 0.6.x (sherpa-onnx), cpal, rubato, rdev, core-graphics (macOS Fn tap), arboard, enigo, reqwest (rustls, stream), sha2, rusqlite (bundled), keyring, sysinfo, tokio; React 18 + TypeScript + Vite + Tailwind; Vitest.

**Spec:** `docs/superpowers/specs/2026-10-22-part1-core-dictation-design.md`

## Global Constraints

- Platforms: macOS arm64 + x64, Windows x64, Linux x64 (X11 full; Wayland best-effort).
- End users install nothing outside the app; all runtimes (sherpa-onnx/ONNX Runtime shared libs) ship inside the bundle.
- Idle RAM (tray-only, model unloaded) < 60 MB; installer < 25 MB excluding models.
- Parakeet TDT 0.6B v2 transcription of a ~5 s utterance < 500 ms on Apple Silicon.
- Default model: Parakeet TDT 0.6B v2 int8 (`csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`), `model_type = "nemo_transducer"`.
- Streaming chunked STT: chunks cut at pauses ≥ 300 ms once ≥ 4 s long; forced cut at quietest 20 ms frame in last 3 s at 15 s; chunks transcribed sequentially in order during recording; 60 s dictation inserted < 1 s after key-up on Apple Silicon.
- Audio fed to STT: 16 000 Hz mono f32. Pre-roll 300 ms. Taps < 250 ms discarded. Hands-free auto-stop at 6 min.
- Clipboard restore delay ~150 ms after synthetic paste.
- Model idle unload default 10 min (configurable, "never" allowed).
- Download progress events ≤ 10 Hz; 3 auto-retries with backoff, then paused.
- Default hotkeys: macOS hold `Fn` (fallback Right-⌥ if event tap unavailable); Windows/Linux hold `Ctrl+Super`; double-tap = hands-free; `Esc` = cancel.
- App data: Tauri `app_data_dir()`; DB `flow.db`; models in `models/<id>/`.
- HF token stored only in OS keychain (`keyring`, service `"flowlocal"`, user `"hf_token"`).
- App identifier `com.flowlocal.app`, product name "FlowLocal".

## Review Focus

1. **Long hands-free recordings (up to 6 min)** — offline transducers degrade or OOM on very long input; expected: full text returned. Pinned in Task 6 by streaming chunking (`ChunkPlanner` never emits a chunk > 15 s; `six_minute_stream_yields_ordered_chunks_le_15s`).
2. **Paths with spaces / non-ASCII** (this repo's path has a space; usernames can be non-ASCII) — model paths go through C strings; expected: model loads. Pinned in Task 6 (`catalog_paths_with_spaces_and_unicode`) and Task 5 (download into such a dir).
3. **Non-text clipboard contents** (user had an image copied) — expected: restored after dictation, not wiped. Pinned in Task 9 (`ClipboardSnapshot` captures text and image; test with fake clipboard).
4. **Hotkey pressed again while Transcribing/Inserting** — expected: new recording starts immediately; earlier result still inserts, in order. Pinned in Task 11 (`press_during_transcribing_starts_new_recording`).
5. **Mic unplugged / no input device mid-recording** — expected: Flow Bar error, state returns to Idle, no hang. Pinned in Task 7 (stream error → `RecorderError::DeviceLost`) and Task 11 (`recorder_error_goes_to_error_then_idle`).

---

## File Structure

```
package.json, vite.config.ts, tailwind.config.ts, tsconfig.json, index.html, flowbar.html
src/                          # React UI
  ipc.ts                      # typed invoke/listen wrappers (single source of IPC types)
  main.tsx, App.tsx           # main window router
  flowbar/FlowBar.tsx, flowbar/main.tsx
  pages/Onboarding.tsx, Models.tsx, History.tsx, Settings.tsx
  components/ProgressBar.tsx, HotkeyInput.tsx, Waveform.tsx
  test/*.test.tsx
src-tauri/
  Cargo.toml, tauri.conf.json, capabilities/default.json, build.rs
  resources/catalog.json      # curated model manifest
  tests/fixtures/*.wav        # integration fixtures (LibriSpeech-style short clips)
  src/
    main.rs                   # calls lib::run()
    lib.rs                    # Tauri builder, state wiring, tray, windows
    app_state.rs              # AppState { store, models, dictation handle, settings }
    commands.rs               # #[tauri::command] fns
    events.rs                 # event name consts + payload structs (serde)
    store/mod.rs, store/migrations.rs
    models/mod.rs (Manager), catalog.rs, hf.rs, download.rs, hardware.rs
    stt/mod.rs (trait), sherpa.rs, worker.rs, split.rs
    audio/mod.rs (Recorder), resample.rs, vad.rs
    postprocess.rs
    insert/mod.rs (trait + ClipboardInserter), clipboard.rs, keystroke.rs, frontmost.rs
    hotkey/mod.rs, gesture.rs, macos_fn.rs, rdev_backend.rs, portal.rs
    dictation/mod.rs (Controller), machine.rs
    permissions.rs
.github/workflows/build.yml
```

---

### Task 1: Toolchain, scaffold, tray-only shell

**Files:**
- Create: everything under "File Structure" as empty modules, `package.json`, Vite/Tailwind config, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `.gitignore`
- Test: `src-tauri/src/lib.rs` (`#[cfg(test)] smoke`)

**Interfaces:**
- Produces: `lib::run()`; windows labelled `"main"` (created lazily) and `"flowbar"`; tray with items `open`, `pause`, `quit`.

- [ ] **Step 1:** Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y` then `source ~/.cargo/env`. Verify `cargo --version` prints ≥ 1.80.
- [ ] **Step 2:** Scaffold with `npm create tauri-app@latest` (React + TS, npm) into the repo root; add Tailwind; set identifier `com.flowlocal.app`, productName `FlowLocal`; add `flowbar.html` as a second Vite entry.
- [ ] **Step 3:** Add Cargo deps (versions current stable): `tauri` (features `tray-icon`), `tauri-plugin-autostart`, `sherpa-rs = { version = "0.6", default-features = false, features = ["download-binaries"] }`, `cpal`, `rubato`, `rdev`, `arboard` (feature `image-data`), `enigo`, `reqwest` (`rustls-tls`, `stream`, `json`), `sha2`, `hex`, `rusqlite` (`bundled`), `keyring`, `sysinfo`, `tokio` (`full`), `serde`, `serde_json`, `thiserror`, `anyhow`, `tracing`, `tracing-subscriber`; macOS target: `core-graphics`, `core-foundation`. Dev-deps: `tempfile`, `wiremock`, `hound`.
- [ ] **Step 4:** In `tauri.conf.json`: `flowbar` window — 220×56, `transparent`, `decorations:false`, `alwaysOnTop`, `skipTaskbar`, `focus:false`, `visible:true`, `resizable:false`; no main window at startup. macOS `"macOSPrivateApi": true` (transparency) and `LSUIElement` (tray-only, no Dock icon). Bundle sherpa-onnx shared libs via `bundle.resources`/`frameworks` (copy from `sherpa-rs-sys` build output in `build.rs`).
- [ ] **Step 5:** Implement `run()`: tray menu (`Open FlowLocal`, `Pause dictation`, `Quit`); `open` creates the `main` window if absent; closing `main` destroys it (`WindowEvent::CloseRequested` → `window.destroy()`), app keeps running.
- [ ] **Step 6:** Run `npm run tauri dev`. Expected: tray icon appears, Flow Bar pill visible bottom-center, no Dock icon; "Open FlowLocal" opens a window; closing it leaves the app running. Run `cargo test` → smoke passes.
- [ ] **Step 7:** Commit `chore: scaffold Tauri app with tray and flow bar windows`.

### Task 2: Store (SQLite)

**Files:**
- Create: `src-tauri/src/store/mod.rs`, `store/migrations.rs`
- Test: inline `#[cfg(test)]` in `store/mod.rs`

**Interfaces:**
- Produces:
  - `Store::open(path: &Path) -> Result<Store>`; `Store::open_in_memory()`
  - `struct Transcript { id: i64, created_at: i64 /*unix ms*/, text: String, raw_text: String, app_name: Option<String>, duration_ms: i64, word_count: i64, wpm: f64, model_id: String }` (`Serialize`)
  - `insert_transcript(&self, t: NewTranscript) -> Result<i64>`; `list_transcripts(&self, page: u32, page_size: u32, query: Option<&str>) -> Result<Vec<Transcript>>` (newest first, `LIKE` on text); `delete_transcript(&self, id: i64)`
  - `get_setting<T: DeserializeOwned>(&self, key) -> Result<Option<T>>`; `set_setting<T: Serialize>(&self, key, &T)`
  - `dictionary_list() -> Vec<DictEntry{id, term, replacement: Option<String>}>`, `dictionary_add(term, replacement)`, `dictionary_delete(id)`
  - `download_upsert(model_id, file, bytes_done, total, status)`, `downloads_for(model_id)`
  - `Store` is `Clone` (wraps `Arc<Mutex<Connection>>`).

- [ ] **Step 1: Failing tests** — `migrates_fresh_db` (schema_version == 1), `transcript_roundtrip_and_search` (insert 3, `list(0,10,Some("hello"))` returns only matches, newest first), `settings_roundtrip` (`set_setting("idle_unload_min", &10)` → `Some(10)`), `dictionary_crud`, `reopen_does_not_remigrate` (open twice on same tempfile).
- [ ] **Step 2:** `cargo test store::` → FAIL (unresolved).
- [ ] **Step 3:** Implement; migrations table `schema_version(version INTEGER)`; tables exactly as spec §8; enable WAL.
- [ ] **Step 4:** `cargo test store::` → PASS.
- [ ] **Step 5:** Commit `feat(store): sqlite store with transcripts, settings, dictionary, downloads`.

### Task 3: Catalog, hardware detection, recommendation

**Files:**
- Create: `src-tauri/resources/catalog.json`, `models/catalog.rs`, `models/hardware.rs`, `scripts/build_catalog.py` (dev-only; pins sha256 + sizes from `https://huggingface.co/api/models/<repo>?blobs=true`, `lfs.sha256`)
- Test: inline tests in both modules

**Interfaces:**
- Produces:
  - `enum EngineKind { Transducer, Whisper, Moonshine, SenseVoice }` (serde lowercase)
  - `struct ModelFile { path: String, size: u64, sha256: String }`
  - `struct CatalogEntry { id: String, name: String, hf_repo: String, revision: String, files: Vec<ModelFile>, engine: EngineKind, languages: Vec<String>, min_ram_gb: u32, notes: String }`
  - `Catalog::bundled() -> Catalog` (`include_str!`), `Catalog::get(&self, id) -> Option<&CatalogEntry>`, `entries()`
  - `struct Hardware { total_ram_gb: f64, cores: usize, arch: String, apple_silicon: bool, gpu_vendor: Option<String> }`; `Hardware::detect()`
  - `recommend(hw: &Hardware, locale: &str) -> &'static str` (model id)

Catalog ids: `parakeet-v2`, `parakeet-v3`, `moonshine-base`, `whisper-tiny-en`, `whisper-small-en`, `sensevoice-small`, `whisper-turbo`. Files: int8 ONNX + `tokens.txt` only (Whisper: `*-encoder.int8.onnx`, `*-decoder.int8.onnx`, `*-tokens.txt`; Moonshine: `preprocess.onnx`, `encode.int8.onnx`, `uncached_decode.int8.onnx`, `cached_decode.int8.onnx`, `tokens.txt`).

- [ ] **Step 1: Failing tests** — `bundled_catalog_parses_and_has_default` (`get("parakeet-v2")` engine Transducer, has `encoder.int8.onnx`, every file has 64-hex sha256); `recommend_low_ram_is_moonshine` (3.5 GB, "en-US" → `moonshine-base`); `recommend_german_is_parakeet_v3` ("de-DE", 16 GB); `recommend_japanese_is_sensevoice` ("ja-JP"); `recommend_default_is_parakeet_v2` ("en-GB", 8 GB). Low RAM rule takes precedence over locale.
- [ ] **Step 2:** `cargo test models::` → FAIL.
- [ ] **Step 3:** Write `build_catalog.py`, run it (`python3 scripts/build_catalog.py > src-tauri/resources/catalog.json`), implement modules. Parakeet v3 language set: bg, hr, cs, da, nl, en, et, fi, fr, de, el, hu, it, lv, lt, mt, pl, pt, ro, sk, sl, es, sv, ru, uk. CJK: zh, ja, ko, yue.
- [ ] **Step 4:** `cargo test models::` → PASS.
- [ ] **Step 5:** Commit `feat(models): curated catalog, hardware detection, recommendation`.

### Task 4: Hugging Face search & engine detection

**Files:**
- Create: `models/hf.rs`
- Test: inline (pure detection) + `wiremock` test for search

**Interfaces:**
- Consumes: `EngineKind`, `ModelFile`, `CatalogEntry` (Task 3)
- Produces:
  - `struct HfRepoSummary { repo: String, downloads: u64, likes: u64, last_modified: String }`
  - `struct HfClient { base: String, token: Option<String> }`; `HfClient::new(token)`, `with_base(url)` (tests)
  - `async search(&self, q: &str) -> Result<Vec<HfRepoSummary>>` — `GET {base}/api/models?search={q}&filter=sherpa-onnx&limit=30`; if empty, retry without `filter`
  - `async inspect(&self, repo: &str) -> Result<Inspection>` where `enum Inspection { Compatible(CatalogEntry), Incompatible { reason: String } }` — `GET {base}/api/models/{repo}?blobs=true`
  - `detect_engine(files: &[String]) -> Option<(EngineKind, Vec<String>)>` (pure; returns chosen file list, int8 preferred)

- [ ] **Step 1: Failing tests** — `detects_parakeet_transducer` (files `encoder.int8.onnx, decoder.int8.onnx, joiner.int8.onnx, tokens.txt, encoder.onnx` → Transducer, picks the 3 int8 + tokens, not `encoder.onnx`); `detects_whisper`; `detects_moonshine`; `detects_sensevoice` (`model.int8.onnx`, `model.onnx`, `tokens.txt`, repo name contains `sense-voice`); `rejects_unknown` (`pytorch_model.bin`) → None; `search_falls_back_without_filter` (wiremock: filtered → `[]`, unfiltered → 1 result); `inspect_builds_entry_with_sha_from_lfs`.
- [ ] **Step 2:** `cargo test hf::` → FAIL.
- [ ] **Step 3:** Implement. Generated entry id = `hf:{repo}`; `min_ram_gb` = ceil(total bytes ×2 / 1e9).
- [ ] **Step 4:** `cargo test hf::` → PASS.
- [ ] **Step 5:** Commit `feat(models): hugging face search and engine detection`.

### Task 5: Downloader & model Manager

**Files:**
- Create: `models/download.rs`, `models/mod.rs`
- Test: inline with `wiremock` + `tempfile`

**Interfaces:**
- Consumes: `Store` download_* (Task 2), `CatalogEntry` (Task 3), `HfClient` (Task 4)
- Produces:
  - `struct Progress { model_id: String, bytes_done: u64, total: u64, bytes_per_sec: f64, eta_secs: Option<u64>, status: DlStatus }`; `enum DlStatus { Queued, Downloading, Paused, Verifying, Installed, Failed(String) }`
  - `async download_file(client, url, dest: &Path, expected_sha: &str, total: u64, cancel: CancellationToken, on_progress: impl Fn(u64)) -> Result<()>` — writes `dest.part`, sends `Range: bytes={len}-` when `.part` exists, verifies sha256 over the whole file, `rename` to `dest`; mismatch → delete `.part`, `Err(DownloadError::Checksum)`
  - `struct Manager` (Clone): `new(root: PathBuf, store: Store, catalog: Catalog, hf: HfClient, emit: Arc<dyn Fn(Progress)+Send+Sync>)`; `install(entry: CatalogEntry)`; `pause(id)`; `resume(id)`; `cancel(id)` (deletes partials); `delete(id)`; `installed() -> Vec<InstalledModel>`; `active() -> Option<InstalledModel>`; `set_active(id) -> Result<()>` (persists setting `active_model`); `disk_usage(id) -> u64`; `resume_incomplete_on_startup()`
  - `struct InstalledModel { entry: CatalogEntry, dir: PathBuf }`; `InstalledModel::file(&self, name_suffix: &str) -> PathBuf`
  - HF URL: `https://huggingface.co/{repo}/resolve/{revision}/{path}`; token as `Authorization: Bearer`.
  - Installed marker: `models/<id>/manifest.json` (the entry) written last.

- [ ] **Step 1: Failing tests** — `downloads_and_verifies` ; `resumes_with_range_header` (pre-create `.part` with first half; wiremock asserts `Range: bytes=N-` and serves 206 rest; final sha ok); `checksum_mismatch_deletes_part`; `cancel_stops_and_removes_part`; `retries_three_times_then_paused` (500 ×4 → status Paused with error); `progress_throttled_to_10hz` (≤ 11 callbacks for a 1 s transfer); `installs_into_dir_with_space_and_unicode` (root `tempdir/"My Models ñ"`); `manager_marks_installed_and_lists`; `insufficient_disk_space_fails_early` (inject free-space fn returning 1 byte).
- [ ] **Step 2:** `cargo test download:: models::tests` → FAIL.
- [ ] **Step 3:** Implement with `reqwest` streaming, `tokio_util::sync::CancellationToken`, free-space check via `fs4::available_space` (add dep), state persisted to `downloads` table each ≥ 1 MB.
- [ ] **Step 4:** Tests → PASS.
- [ ] **Step 5:** Commit `feat(models): resumable verified downloader and model manager`.

### Task 6: STT engine & worker thread

**Files:**
- Create: `stt/mod.rs`, `stt/sherpa.rs`, `stt/worker.rs`, `stt/chunker.rs`, `stt/session.rs`, `src-tauri/tests/stt_integration.rs`, `tests/fixtures/*.wav` (+ `fixtures/expected.json`)

**Interfaces:**
- Consumes: `InstalledModel`, `EngineKind` (Tasks 3, 5)
- Produces:
  - `trait SttEngine: Send { fn load(&mut self, m: &InstalledModel) -> Result<()>; fn transcribe(&mut self, samples: &[f32]) -> Result<String>; fn unload(&mut self); fn loaded_id(&self) -> Option<&str>; }`
  - `SherpaEngine` — dispatches on `EngineKind` to `sherpa_rs::{transducer::TransducerRecognizer (model_type "nemo_transducer"), whisper::WhisperRecognizer, moonshine::MoonshineRecognizer, sense_voice::SenseVoiceRecognizer}`; `num_threads = min(cores, 4)`; provider `"coreml"` on Apple Silicon when transducer load succeeds with it, else `"cpu"`.
  - `struct Chunk { samples: Vec<f32>, has_speech: bool, start_ms: u64 }`
  - `struct ChunkerConfig { min_secs: f32 /*4*/, pause_ms: u32 /*300*/, max_secs: f32 /*15*/, search_back_secs: f32 /*3*/ }` (`Default`)
  - `ChunkPlanner` (pure, `chunker.rs`): `new(cfg)`, `push(&mut self, frames: &[f32]) -> Vec<Chunk>` (any number of completed chunks), `finish(&mut self) -> Option<Chunk>` (remaining tail, trimmed with `audio::vad::trim_silence`; `None` if empty), `any_speech(&self) -> bool`. Speech/silence per 20 ms frame uses the same adaptive RMS threshold as `trim_silence` (expose `audio::vad::FrameClassifier`). A cut at a pause lands in the middle of the pause.
  - `join_chunks(texts: &[String]) -> String` — trims each, skips empties, joins with one space; lower-cases a chunk's first letter when the previous text does not end in `.?!` and the letter starts a non-acronym word (next char lowercase).
  - `SttWorker::spawn(engine: Box<dyn SttEngine>, idle_unload: Option<Duration>) -> SttHandle`; `SttHandle` (Clone): `session(&self, model: InstalledModel) -> SttSession` (begins loading the model immediately if not loaded), `async preload(model)`, `set_idle_unload(Option<Duration>)`. Panics caught with `catch_unwind`; one automatic reload then `Err(SttError::EngineFailed)`.
  - `SttSession` (`session.rs`): `feed(&self, chunk: Chunk)` (non-blocking; queued FIFO on the worker; skipped if `!has_speech`); `async finish(self, tail: Option<Chunk>) -> Result<String>` (awaits all queued chunks in order + tail, returns `join_chunks`); `cancel(self)` (drops queued, not-yet-started chunks). Chunks of different sessions never interleave out of order: the worker processes one FIFO queue.

- [ ] **Step 1: Failing unit tests** (fake engine) — `worker_loads_once_and_reuses`; `worker_swaps_on_model_change`; `worker_unloads_after_idle` (idle 50 ms); `panic_reloads_once_then_errors`; `chunker_short_utterance_only_tail` (3 s speech → `push` yields nothing, `finish` yields one chunk); `chunker_cuts_at_pause_after_min` (5 s speech + 400 ms silence + 3 s speech → one chunk emitted ≈ 5.2 s, cut inside pause); `chunker_ignores_pause_before_min` (2 s speech + 400 ms pause + 2 s → no cut); `chunker_forces_cut_at_max_at_quietest_frame` (continuous 20 s tone with a dip at 13.5 s → first chunk ends at 13.5 s ± 20 ms, ≤ 15 s); `six_minute_stream_yields_ordered_chunks_le_15s` (pushed in 20 ms frames; concatenated chunks + tail == input length minus trimmed silence; every chunk ≤ 15 s); `silent_chunk_has_no_speech`; `join_chunks_lowercases_continuation` (`["So I went", "And then"]` → `"So I went and then"`; `["Done.", "Next"]` → `"Done. Next"`; `["Hi", "NASA rocks"]` keeps `NASA`); `session_transcribes_chunks_in_order_while_feeding` (fake engine with 50 ms delay; feed 3 chunks; `finish` returns texts in order; assert chunk 1 started before `finish` was called); `session_finish_latency_is_only_tail` (fake engine 100 ms per chunk; feed 5 chunks spaced 150 ms apart, then `finish(tail)` returns within 250 ms); `session_cancel_drops_pending`; `silent_chunks_not_sent_to_engine`.
- [ ] **Step 2:** `cargo test stt::` → FAIL.
- [ ] **Step 3:** Implement worker as `std::thread` + `std::sync::mpsc` command channel with `oneshot` replies.
- [ ] **Step 4:** `cargo test stt::` → PASS.
- [ ] **Step 5: Integration test** `parakeet_wer_below_threshold` — `#[ignore]` unless env `FLOWLOCAL_MODEL_DIR` set; loads `parakeet-v2` from that dir (place the dir under a path containing a space and `é` to cover `catalog_paths_with_spaces_and_unicode`); for each fixture WAV, WER vs expected < 0.08; asserts a 5 s clip transcribes in < 500 ms on Apple Silicon; `streaming_60s_finish_under_1s` — concatenate fixtures to ~60 s, feed through `ChunkPlanner` + `SttSession` at real-time pace (20 ms frames with sleeps), assert `finish` returns < 1 s after the last frame and WER of the streamed result < 0.10. Fetch model once via the Manager (or `huggingface-cli`-free curl script `scripts/fetch_test_model.sh`). Run `FLOWLOCAL_MODEL_DIR=... cargo test --test stt_integration -- --ignored` → PASS.
- [ ] **Step 6:** Commit `feat(stt): sherpa-onnx engine, worker thread, streaming chunked sessions`.

### Task 7: Audio capture, resampling, VAD trim

**Files:**
- Create: `audio/mod.rs`, `audio/resample.rs`, `audio/vad.rs`

**Interfaces:**
- Produces:
  - `to_mono_16k(input: &[f32], channels: u16, rate: u32) -> Vec<f32>` (rubato `FftFixedIn` or sinc; pass-through when already 16k mono)
  - `trim_silence(samples: &[f32]) -> Option<&[f32]>` — 20 ms frames, RMS threshold adaptive = max(0.01, noise_floor×3) where noise_floor = 10th-percentile frame RMS; keep 200 ms padding; `None` when no frame above threshold (no speech)
  - `struct StreamResampler` — stateful incremental version of `to_mono_16k` (`process(&mut self, input: &[f32]) -> Vec<f32>`, `flush()`), so live frames can be streamed
  - `audio::vad::FrameClassifier` — the adaptive 20 ms speech/silence classifier shared by `trim_silence` and the Task 6 `ChunkPlanner` (`is_speech(&mut self, frame: &[f32]) -> bool`, noise floor updated online)
  - `struct Recorder`: `new(device_name: Option<String>, level_tx: Sender<f32>) -> Result<Recorder>`; `start(frame_tx: std::sync::mpsc::Sender<Vec<f32>>)` — streams 16 kHz mono frames (~20 ms each, pre-roll sent first) to `frame_tx` from a non-audio thread; `stop() -> Result<RecordingStats, RecorderError>` (flushes the resampler, sends remaining frames, then drops `frame_tx`; `RecordingStats { duration_ms: u64 }`); `cancel()`; `list_devices() -> Vec<String>`
  - `enum RecorderError { NoDevice, PermissionDenied, DeviceLost, Other(String) }`
  - Stream stays open between dictations only while app is "warm" (recording within last 60 s) for pre-roll; otherwise closed to save power and to avoid the macOS mic indicator at rest. Pre-roll applies only when warm.

- [ ] **Step 1: Failing tests** — `resample_48k_stereo_to_16k_mono_length` (1 s → 16000 ± 32 samples); `resample_preserves_440hz_tone` (dominant freq via zero crossings ≈ 440 ± 5); `trim_removes_leading_trailing_silence` (0.5 s silence + 1 s tone + 0.5 s silence → 1.4 s ± 0.05); `trim_returns_none_for_silence_and_low_noise`; `stream_resampler_matches_batch` (48 kHz input fed in 7 uneven pieces == `to_mono_16k` of the whole within 1e-3 RMS); `frame_classifier_separates_tone_from_noise`; `stream_error_maps_to_device_lost` (feed cpal `StreamError::DeviceNotAvailable` into the error mapper fn `map_stream_error`).
- [ ] **Step 2:** `cargo test audio::` → FAIL.
- [ ] **Step 3:** Implement; level events = RMS per ~33 ms, sent with `try_send` (never block audio thread).
- [ ] **Step 4:** `cargo test audio::` → PASS. Manual: temporary `examples/record.rs` records 3 s, writes WAV, play back.
- [ ] **Step 5:** Commit `feat(audio): cpal recorder, resampling, silence trimming`.

### Task 8: Post-processing

**Files:**
- Create: `postprocess.rs`

**Interfaces:**
- Consumes: `DictEntry` (Task 2)
- Produces: `postprocess(raw: &str, dict: &[DictEntry], ctx: InsertContext) -> String`; `struct InsertContext { prev_char: Option<char> }` (`None` = unknown → no leading space)

- [ ] **Step 1: Failing tests** — `collapses_whitespace_and_trims`; `capitalizes_first_letter`; `dictionary_replaces_whole_word_case_insensitive` (`"open ai"`→`"OpenAI"`, does not touch `"opener"`); `dictionary_term_without_replacement_fixes_casing` (`"kubernetes"` term `Kubernetes`); `adds_leading_space_after_word_char` (prev `'d'` → `" Hello."`); `no_leading_space_after_newline_or_space`; `lowercases_first_letter_mid_sentence` (prev `'a'`, raw `"Then we go."` → `" then we go."`).
- [ ] **Step 2–4:** fail → implement → pass (`cargo test postprocess::`).
- [ ] **Step 5:** Commit `feat: transcript post-processing and personal dictionary`.

(Note: `prev_char` is best-effort; Task 9 provides it via accessibility on macOS where available, otherwise `None`.)

### Task 9: Text insertion

**Files:**
- Create: `insert/mod.rs`, `insert/clipboard.rs`, `insert/keystroke.rs`, `insert/frontmost.rs`

**Interfaces:**
- Consumes: `InsertContext` (Task 8)
- Produces:
  - `trait Inserter: Send + Sync { fn insert(&self, text: &str) -> InsertOutcome; fn context(&self) -> InsertContext; }`
  - `enum InsertOutcome { Pasted, CopiedOnly(String /*reason*/) }`
  - `trait Clipboard { fn snapshot(&mut self) -> ClipboardSnapshot; fn set_text(&mut self, s: &str) -> Result<()>; fn restore(&mut self, s: ClipboardSnapshot) -> Result<()>; }`; `enum ClipboardSnapshot { Empty, Text(String), Image(ImageData<'static>) }`
  - `trait Keystroke { fn paste(&self) -> Result<()>; }` (enigo: ⌘V macOS, Ctrl+V else)
  - `ClipboardInserter<C: Clipboard, K: Keystroke> { restore_delay: Duration /*150ms*/ }`
  - `frontmost_app_name() -> Option<String>` (macOS `NSWorkspace.frontmostApplication.localizedName`; Windows `GetForegroundWindow`→process name; Linux X11 `_NET_ACTIVE_WINDOW` WM_CLASS; Wayland `None`)
  - `secure_input_active() -> bool` (macOS `IsSecureEventInputEnabled`; others false)
  - `is_wayland() -> bool`

- [ ] **Step 1: Failing tests** (fake clipboard/keystroke) — `pastes_then_restores_text_clipboard`; `restores_image_clipboard` (snapshot Image → after insert, restore called with same Image); `empty_clipboard_restored_as_empty`; `paste_error_returns_copied_only_and_leaves_text` (no restore); `secure_input_returns_copied_only` (inject `secure_input_active` = true).
- [ ] **Step 2–4:** fail → implement → pass (`cargo test insert::`).
- [ ] **Step 5:** Manual: example `examples/insert.rs "héllo 👋 世界"` with TextEdit focused after 3 s delay → text appears; prior clipboard restored.
- [ ] **Step 6:** Commit `feat(insert): clipboard-paste inserter with snapshot/restore`.

### Task 10: Global hotkeys

**Files:**
- Create: `hotkey/mod.rs`, `hotkey/gesture.rs`, `hotkey/macos_fn.rs`, `hotkey/rdev_backend.rs`, `hotkey/portal.rs`

**Interfaces:**
- Produces:
  - `enum HotkeyEvent { Pressed, Released, DoubleTap, Cancel }`
  - `struct Binding { keys: Vec<Key> }` (serde, e.g. `["Fn"]`, `["ControlLeft","MetaLeft"]`); `default_binding() -> Binding` per OS
  - `struct GestureDetector` (pure): `fn on_key(&mut self, down: bool, at: Instant) -> Vec<HotkeyEvent>` — emits `Pressed` on chord-down, `Released` on chord-up; if a chord-up with hold < 250 ms is followed by chord-down within 300 ms → `DoubleTap` (and the second up does not emit `Released`)
  - `HotkeyService::start(binding: Binding, tx: Sender<HotkeyEvent>) -> Result<HotkeyService>`; `set_binding(Binding)`; `stop()`. Backend: macOS CGEventTap (listens `flagsChanged` for `Fn` via `kCGEventFlagMaskSecondaryFn`, plus keyDown for `Esc` and other chords); Windows/Linux-X11 `rdev::listen` on a thread; Wayland → `portal.rs` (`ashpd` GlobalShortcuts) if available else `Err(HotkeyError::Unsupported)`.
  - Esc → `Cancel` only emitted while service's `armed` flag is set (`set_armed(bool)`, set by dictation controller during recording) so Esc isn't hijacked otherwise; Esc is never swallowed.

- [ ] **Step 1: Failing tests** (GestureDetector) — `hold_emits_pressed_then_released`; `short_tap_emits_pressed_released` (controller discards < 250 ms); `double_tap_within_300ms_emits_doubletap`; `slow_second_tap_is_new_press`; `chord_requires_all_keys` (Ctrl alone → nothing; Ctrl+Super → Pressed); `binding_serde_roundtrip`.
- [ ] **Step 2–4:** fail → implement → pass (`cargo test hotkey::`).
- [ ] **Step 5:** Manual on macOS: `examples/hotkeys.rs` prints events while holding/double-tapping `Fn` (needs Accessibility granted to the terminal). If the event tap fails to install, service falls back to Right-⌥ and logs a warning.
- [ ] **Step 6:** Commit `feat(hotkey): global hold/double-tap hotkeys with macOS Fn support`.

### Task 11: Dictation state machine & controller

**Files:**
- Create: `dictation/machine.rs`, `dictation/mod.rs`, `events.rs`

**Interfaces:**
- Consumes: `HotkeyEvent` (T10), `Recorder`/`RecorderError` (T7), `SttHandle`/`SttSession`/`ChunkPlanner` (T6), `Inserter` (T9), `postprocess` (T8), `Store` (T2), `Manager::active()` (T5)
- Produces:
  - `enum State { Idle, Recording { started: Instant, hands_free: bool }, Transcribing, Inserting, Error(String) }` (serde for UI as `{kind, message?}`)
  - `machine::step(state, input: Input, now) -> (State, Vec<Effect>)` (pure). `Input` = `Hotkey(HotkeyEvent) | RecorderFailed(RecorderError) | Transcribed(Result<String>) | Inserted(InsertOutcome) | Tick`; `Effect` = `StartRecording | StopRecording | CancelRecording | Transcribe | Insert(String) | Persist | ShowError(String) | ArmEsc(bool) | Emit(State)`.
  - Rules: Pressed in Idle/Transcribing/Inserting → Recording (pipeline of the previous utterance continues; results inserted in order via a FIFO queue in the controller); Released < 250 ms → Idle (cancel); DoubleTap → Recording{hands_free:true}; Pressed while hands_free → stop & Transcribe; Tick with hands_free and elapsed ≥ 6 min → stop & Transcribe; Cancel → Idle; any failure → Error, then Tick after 2 s → Idle; no active model on Pressed → Error("No model installed").
  - Streaming: on `StartRecording` the controller opens `SttSession` for the active model and a `ChunkPlanner`; a pump task drains recorder frames → `planner.push` → `session.feed` for each chunk during recording. On `StopRecording` it calls `planner.finish()`; if `!planner.any_speech()` → discard (Idle), else `session.finish(tail)` → `Transcribed`. Cancel → `session.cancel()`.
  - `Controller::spawn(deps: ControllerDeps) -> ControllerHandle` (tokio task); `ControllerHandle::set_paused(bool)`; emits Tauri event `dictation://state` and `dictation://level` (f32) via `deps.emit`.
  - Persist: `NewTranscript { text, raw_text, app_name: frontmost_app_name() captured at key-down, duration_ms, word_count = text.split_whitespace().count(), wpm = words / (duration_ms/60000), model_id }`.
- `events.rs`: consts `STATE = "dictation://state"`, `LEVEL = "dictation://level"`, `DL_PROGRESS = "models://progress"`, `TOAST = "app://toast"`, `TRANSCRIPT_ADDED = "history://added"`.

- [ ] **Step 1: Failing tests** (pure machine + controller with fakes) — `hold_speak_release_inserts_and_persists`; `short_tap_is_discarded`; `no_speech_is_discarded` (planner `any_speech()==false` ⇒ Idle, nothing inserted); `chunks_fed_during_recording` (fake recorder streams 12 s with a pause at 6 s → fake session receives 1 chunk before Released); `double_tap_hands_free_then_press_stops`; `hands_free_auto_stops_at_6_min`; `cancel_discards`; `no_model_shows_error`; `press_during_transcribing_starts_new_recording` (both results inserted in order A, B); `recorder_error_goes_to_error_then_idle`; `copied_only_emits_toast_and_still_persists`; `paused_ignores_hotkeys`.
- [ ] **Step 2–4:** fail → implement → pass (`cargo test dictation::`).
- [ ] **Step 5:** Commit `feat(dictation): state machine and controller pipeline`.

### Task 12: Permissions, commands, IPC layer, wiring

**Files:**
- Create: `permissions.rs`, `commands.rs`, `app_state.rs`, `src/ipc.ts`; Modify: `lib.rs`
- Test: `src/test/ipc.test.ts` (mocked `@tauri-apps/api`)

**Interfaces:**
- Produces (Rust commands, mirrored 1:1 in `ipc.ts` with TS types):
  - `get_state() -> State`; `set_paused(bool)`
  - `permissions_status() -> { microphone: Perm, accessibility: Perm }` (`Perm = "granted"|"denied"|"unknown"|"notRequired"`); `open_permission_settings(kind: "microphone"|"accessibility")` (macOS `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` / `?Privacy_Accessibility`; `AXIsProcessTrustedWithOptions` prompt; mic via `AVCaptureDevice authorizationStatus`/request)
  - `catalog() -> CatalogEntry[]`; `installed() -> InstalledModelDto[]` (with `disk_bytes`, `active`); `recommend() -> { id, hardware }`; `hf_search(q) -> HfRepoSummary[]`; `hf_inspect(repo) -> Inspection`; `model_install(id | entry)`; `model_pause/resume/cancel/delete(id)`; `model_set_active(id)`
  - `history_list(page, query?) -> Transcript[]`; `history_delete(id)`
  - `settings_get() -> Settings`; `settings_set(Settings)` where `Settings { hotkey: Binding, mic: Option<String>, show_flowbar: bool, idle_unload_min: Option<u32>, preload: bool, launch_at_login: bool, onboarded: bool }`; `hf_token_set(Option<String>)` (keyring)
  - `dictionary_list/add/delete`; `mic_list() -> string[]`
  - `ipc.ts`: `api.<command>(…)` wrappers + `on(event, cb)` typed listeners for `events.rs` names.
- Wiring in `lib.rs`: build `AppState`, start `Manager.resume_incomplete_on_startup()`, `SttWorker`, `HotkeyService`, `Controller`; apply settings changes live (binding, mic, idle unload, autostart via `tauri-plugin-autostart`); first launch with `onboarded=false` opens main window at `/onboarding`.

- [ ] **Step 1: Failing tests** — Rust: `settings_default_values` (hotkey = OS default, `show_flowbar=true`, `idle_unload_min=Some(10)`, `preload=false`); TS: `ipc_wrappers_invoke_correct_commands` (each wrapper calls `invoke` with snake_case name and args).
- [ ] **Step 2–4:** fail → implement → pass (`cargo test && npx vitest run`).
- [ ] **Step 5:** `npm run tauri dev`, then in devtools console `await window.__TAURI__.core.invoke('recommend')` returns `parakeet-v2` on this Mac.
- [ ] **Step 6:** Commit `feat: tauri commands, permissions, ipc layer, app wiring`.

### Task 13: Flow Bar UI

**Files:**
- Create: `src/flowbar/FlowBar.tsx`, `src/flowbar/main.tsx`, `src/components/Waveform.tsx`; Test: `src/test/FlowBar.test.tsx`

**Interfaces:**
- Consumes: `on("dictation://state")`, `on("dictation://level")`, `on("app://toast")` (Task 12)
- Produces: visual states per spec §5 — Idle (dim 40×6 handle, hidden if `show_flowbar=false`), Recording (pill expands to ~180×40, 16-bar waveform from levels, red dot when hands-free), Transcribing (shimmer), Done (✓ for 600 ms), Error (red pill, message, 2 s). Window repositions to bottom-center of the monitor containing the cursor at each Pressed (Rust side, `Window::set_position` using `cursor_position` + `available_monitors`). Window ignores cursor events (`set_ignore_cursor_events(true)`) so it never steals clicks/focus.

- [ ] **Step 1: Failing tests** — `renders_idle_handle`; `renders_waveform_when_recording` (16 bars, heights reflect fed levels); `renders_error_message`; `hidden_when_flowbar_disabled_and_idle`.
- [ ] **Step 2–4:** fail → implement → pass (`npx vitest run FlowBar`).
- [ ] **Step 5:** Manual: hold Fn in TextEdit — pill animates with voice, shimmers, text appears, TextEdit keeps focus.
- [ ] **Step 6:** Commit `feat(ui): flow bar overlay`.

### Task 14: Main window — Onboarding, Models, History, Settings

**Files:**
- Create: `src/App.tsx`, `src/pages/*.tsx`, `src/components/ProgressBar.tsx`, `src/components/HotkeyInput.tsx`; Test: `src/test/pages.test.tsx`

**Interfaces:**
- Consumes: all `api.*` from Task 12.
- Produces: routes `/onboarding`, `/models`, `/history`, `/settings`; left sidebar nav (Home placeholder reserved for Part 3 points to History). Visual language: neutral off-white / near-black, one accent, Inter font (bundled locally, no network), 8 px radius, light/dark via `prefers-color-scheme`.
  - **Onboarding:** 4 steps — Welcome → Permissions (live status, polls `permissions_status` every 1 s, "Open Settings" buttons; Linux/Windows show "Not required") → Model (shows recommended model, size, "Download" with ProgressBar; "Choose another" opens catalog) → Try it (textarea focused; instructs "Hold Fn and speak"; detects a `history://added` event → "You're all set", sets `onboarded=true`).
  - **Models:** tabs Installed / Catalog / Search Hugging Face. Rows show name, languages, size, status; actions Download / Pause / Resume / Cancel / Set active / Delete (confirm). Search tab: debounced 300 ms query → results → "Inspect" → compatible shows Install; incompatible shows reason.
  - **History:** search box, list (text, app, time, word count), copy and delete; live-prepends on `history://added`; infinite scroll pages of 50.
  - **Settings:** HotkeyInput (records a chord), mic dropdown, Flow Bar toggle, idle-unload select (5/10/30 min/Never), preload toggle, launch-at-login toggle, HF token (password field, saved to keychain), personal dictionary (term → optional replacement).
  - Tray: add "Active model ▸" submenu (installed models, radio) — rebuilt on `models://progress` installed events and `model_set_active`.

- [ ] **Step 1: Failing tests** — `onboarding_advances_when_permissions_granted`; `onboarding_download_shows_progress` (mock progress events → bar 50%); `models_install_button_calls_api`; `models_search_shows_incompatible_reason`; `history_search_debounces_and_queries`; `settings_hotkey_change_persists`.
- [ ] **Step 2–4:** fail → implement → pass (`npx vitest run`).
- [ ] **Step 5:** Commit `feat(ui): onboarding, models, history, settings pages and tray model menu`.

### Task 15: Packaging, CI, end-to-end verification

**Files:**
- Create: `.github/workflows/build.yml`, `README.md` (build + dev instructions)
- Modify: `tauri.conf.json` (bundle targets, icons)

- [ ] **Step 1:** Workflow matrix: `macos-14` (aarch64-apple-darwin), `macos-13` (x86_64-apple-darwin), `windows-latest`, `ubuntu-22.04` (deps: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libxdo-dev`). Steps: `cargo test`, `npx vitest run`, `tauri-apps/tauri-action` build, upload artifacts. Bundle targets: `dmg`, `nsis`, `appimage`, `deb`.
- [ ] **Step 2:** Local `npm run tauri build`. Expected: `.dmg` produced; size < 25 MB; sherpa-onnx/onnxruntime dylibs present inside `FlowLocal.app/Contents/Frameworks` (check with `ls`); app launches from `/Applications` on a clean user account path with no Homebrew dependencies (`otool -L` shows only `@rpath`/system libs).
- [ ] **Step 3: Manual E2E on this Mac (fresh app data dir):** onboarding → grant permissions → download Parakeet v2 → dictate into TextEdit, Safari textarea, VS Code, Slack. Check: punctuated text inserted; clipboard (text and an image) restored; Esc cancels; double-tap hands-free works; 5 s utterance latency < 500 ms (log timing); idle RAM after 10 min with model unloaded < 60 MB (Activity Monitor, all FlowLocal processes incl. WebKit content for Flow Bar).
- [ ] **Step 4:** Push branch, confirm all four CI jobs green.
- [ ] **Step 5:** Commit `chore: packaging and CI build matrix`.

---

## Spec coverage check

| Spec section | Tasks |
|---|---|
| §2 Technology / packaging | 1, 15 |
| §3 Architecture & module interfaces | 1, 6, 7, 9, 10, 11, 12 |
| §4 Catalog / HF search / downloader / hardware / lifecycle | 3, 4, 5, 6 |
| §5 Triggers, state machine, pipeline, Flow Bar | 7, 8, 9, 10, 11, 13 |
| §6 UI + tray | 1, 14 |
| §7 Error handling | 5, 6, 7, 9, 10, 11, 12 |
| §8 Data | 2 |
| §9 Testing | every task; 6 (integration), 15 (CI, manual) |
| §1 Success criteria | 6 (latency), 15 (RAM, size, E2E, CI) |
