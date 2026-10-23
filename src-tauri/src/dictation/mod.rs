//! Dictation controller: hotkeys -> recorder -> streaming chunked STT -> post-process ->
//! insert at the cursor -> history. All decisions live in the pure [`machine`]; the
//! [`Controller`] (a tokio task) performs the machine's effects.
//!
//! Collaborators sit behind small ports so tests can fake them.

pub mod machine;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::time::{Instant, MissedTickBehavior};

use crate::audio::{Recorder, RecorderError, RecordingStats};
use crate::events;
use crate::hotkey::HotkeyEvent;
use crate::insert::{InsertOutcome, Inserter};
use crate::models::{InstalledModel, Manager};
use crate::postprocess::postprocess;
use crate::store::{NewTranscript, Store};
use crate::stt::{Chunk, ChunkPlanner, ChunkerConfig, SttError, SttHandle, SttSession};
/// Event emitter handed to the controller: `(event name, payload)`.
pub type EmitFn = Arc<dyn Fn(&str, Value) + Send + Sync>;

use machine::{Effect, Input, State, step};

// ---- ports ----------------------------------------------------------------------------------

pub trait RecorderPort: Send + Sync {
    fn start(&self, frame_tx: Sender<Vec<f32>>) -> Result<(), RecorderError>;
    fn stop(&self) -> Result<RecordingStats, RecorderError>;
    fn cancel(&self);
}

impl RecorderPort for Recorder {
    fn start(&self, frame_tx: Sender<Vec<f32>>) -> Result<(), RecorderError> {
        Recorder::start(self, frame_tx)
    }
    fn stop(&self) -> Result<RecordingStats, RecorderError> {
        Recorder::stop(self)
    }
    fn cancel(&self) {
        Recorder::cancel(self)
    }
}

/// One utterance's streaming transcription session.
pub trait SessionPort: Send {
    fn feed(&self, chunk: Chunk);
    fn finish(self: Box<Self>, tail: Option<Chunk>)
    -> BoxFuture<'static, Result<String, SttError>>;
    fn cancel(self: Box<Self>);
}

impl SessionPort for SttSession {
    fn feed(&self, chunk: Chunk) {
        SttSession::feed(self, chunk)
    }
    fn finish(
        self: Box<Self>,
        tail: Option<Chunk>,
    ) -> BoxFuture<'static, Result<String, SttError>> {
        Box::pin((*self).finish(tail))
    }
    fn cancel(self: Box<Self>) {
        (*self).cancel()
    }
}

pub trait SttPort: Send + Sync {
    fn session(&self, model: InstalledModel) -> Box<dyn SessionPort>;
}

impl SttPort for SttHandle {
    fn session(&self, model: InstalledModel) -> Box<dyn SessionPort> {
        Box::new(SttHandle::session(self, model))
    }
}

pub trait ModelSource: Send + Sync {
    fn active(&self) -> Option<InstalledModel>;
}

impl ModelSource for Manager {
    fn active(&self) -> Option<InstalledModel> {
        Manager::ensure_active(self)
    }
}

// ---- public surface -------------------------------------------------------------------------

pub struct ControllerDeps {
    /// Events from `HotkeyService` (the receiving end of the sender given to `start`).
    pub hotkeys: Receiver<HotkeyEvent>,
    /// Receiving end of the `SyncSender<f32>` given to `Recorder::new`.
    pub levels: Option<Receiver<f32>>,
    pub recorder: Arc<dyn RecorderPort>,
    pub stt: Arc<dyn SttPort>,
    pub models: Arc<dyn ModelSource>,
    pub inserter: Arc<dyn Inserter>,
    pub store: Arc<Store>,
    /// Forwards `(event name, payload)` to the UI (Tauri `emit` in Task 12).
    pub emit: EmitFn,
    /// `HotkeyService::set_armed`.
    pub arm_esc: Arc<dyn Fn(bool) + Send + Sync>,
    /// Name of the frontmost app (`insert::frontmost_app_name`), read at key-down.
    pub app_name: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// Whether the OS has denied microphone access (probed at key-down; macOS only).
    pub mic_denied: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Called when a dictation was requested but no speech model is installed.
    pub on_no_model: Arc<dyn Fn() + Send + Sync>,
}

pub struct ControllerHandle {
    tx: UnboundedSender<Msg>,
}

impl ControllerHandle {
    /// While paused, hotkeys are ignored and a recording in progress is cancelled.
    pub fn set_paused(&self, paused: bool) {
        let _ = self.tx.send(Msg::Paused(paused));
    }
}

impl Drop for ControllerHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

// ---- controller -----------------------------------------------------------------------------

/// Flow Bar message when the text was only copied (the reason is logged).
#[cfg(target_os = "macos")]
pub const COPIED_ONLY_MESSAGE: &str = "Copied. Press \u{2318}V to paste.";
#[cfg(not(target_os = "macos"))]
pub const COPIED_ONLY_MESSAGE: &str = "Copied. Press Ctrl+V to paste.";

/// Minimum spacing of level events (~30 Hz).
const LEVEL_EVERY: Duration = Duration::from_millis(33);
const TICK_EVERY: Duration = Duration::from_millis(250);

enum Msg {
    Hotkey(HotkeyEvent),
    Paused(bool),
    /// An utterance's transcription finished (`Ok(None)`: no speech).
    Finalized {
        seq: u64,
        res: Result<Option<String>, SttError>,
    },
    /// `Err`: the insert task panicked.
    InsertDone(Result<Option<(String, InsertOutcome)>, String>),
    /// The recorder thread started recording `id`.
    RecStarted {
        id: u64,
    },
    RecStartFailed {
        id: u64,
        err: RecorderError,
    },
    RecStopped {
        seq: u64,
        res: Result<RecordingStats, RecorderError>,
    },
    /// The frame channel of recording `id` closed (normal stop/cancel, or the device was lost).
    PumpEnded {
        id: u64,
    },
    Shutdown,
}

/// Commands for the recorder thread. Recorder calls can block for seconds, so they never run on
/// the controller task; one thread keeps them strictly ordered.
enum RecCmd {
    Start { id: u64, frame_tx: Sender<Vec<f32>> },
    Stop { seq: u64 },
    Cancel,
}

fn guarded<T>(f: impl FnOnce() -> Result<T, RecorderError>) -> Result<T, RecorderError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(RecorderError::Other("recorder panicked".into())))
}

fn spawn_recorder_thread(
    recorder: Arc<dyn RecorderPort>,
    tx: UnboundedSender<Msg>,
) -> Sender<RecCmd> {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("flowlocal-recorder-ctl".into())
        .spawn(move || {
            for cmd in cmd_rx {
                match cmd {
                    RecCmd::Start { id, frame_tx } => {
                        let msg = match guarded(|| recorder.start(frame_tx)) {
                            Ok(()) => Msg::RecStarted { id },
                            Err(err) => Msg::RecStartFailed { id, err },
                        };
                        let _ = tx.send(msg);
                    }
                    RecCmd::Stop { seq } => {
                        let res = guarded(|| recorder.stop());
                        let _ = tx.send(Msg::RecStopped { seq, res });
                    }
                    RecCmd::Cancel => {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            recorder.cancel()
                        }));
                    }
                }
            }
        })
        .expect("spawn recorder control thread");
    cmd_tx
}

type Pump = JoinHandle<(ChunkPlanner, Box<dyn SessionPort>)>;

/// A recording in progress.
struct Active {
    id: u64,
    /// The recorder thread confirmed the start.
    started: bool,
    /// The frame channel closed before the start was confirmed.
    pump_ended: bool,
    pump: Pump,
    app_name: Option<String>,
    model_id: String,
}

#[derive(Clone)]
struct Meta {
    app_name: Option<String>,
    model_id: String,
    duration_ms: u64,
}

/// An utterance between key-up and insertion; kept in utterance order.
struct Slot {
    seq: u64,
    meta: Meta,
    res: Option<SlotRes>,
}

enum SlotRes {
    Text(String),
    NoSpeech,
    Stt(SttError),
    /// Failed outside the speech engine (e.g. the recorder could not stop).
    Failed(String),
}

struct Job {
    raw: String,
    meta: Meta,
    /// Post-processed text that was inserted (empty until the insert finishes).
    text: String,
}

pub struct Controller {
    d: ControllerDeps2,
    rec_tx: Sender<RecCmd>,
    next_rec_id: u64,
    /// Stopped recordings whose `RecStopped` has not arrived yet, by utterance seq.
    stopping: HashMap<u64, Pump>,
    tx: UnboundedSender<Msg>,
    state: State,
    last_emitted: Option<Value>,
    paused: bool,
    active: Option<Active>,
    slots: VecDeque<Slot>,
    next_seq: u64,
    cur_meta: Option<Meta>,
    inserting: Option<Job>,
    queue: VecDeque<Input>,
}

/// `ControllerDeps` minus the receivers that move into helper threads.
struct ControllerDeps2 {
    stt: Arc<dyn SttPort>,
    models: Arc<dyn ModelSource>,
    inserter: Arc<dyn Inserter>,
    store: Arc<Store>,
    emit: EmitFn,
    arm_esc: Arc<dyn Fn(bool) + Send + Sync>,
    app_name: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    mic_denied: Arc<dyn Fn() -> bool + Send + Sync>,
    on_no_model: Arc<dyn Fn() + Send + Sync>,
}

fn spawn_pump(
    frames: Receiver<Vec<f32>>,
    session: Box<dyn SessionPort>,
    id: u64,
    tx: UnboundedSender<Msg>,
) -> std::io::Result<Pump> {
    std::thread::Builder::new()
        .name("flowlocal-pump".into())
        .spawn(move || {
            let mut planner = ChunkPlanner::new(ChunkerConfig::default());
            while let Ok(frame) = frames.recv() {
                for chunk in planner.push(&frame) {
                    session.feed(chunk);
                }
            }
            let _ = tx.send(Msg::PumpEnded { id });
            (planner, session)
        })
}

/// Joins the pump off the async threads and cancels its session.
fn discard_pump(pump: Pump) {
    tokio::task::spawn_blocking(move || {
        if let Ok((_, session)) = pump.join() {
            session.cancel();
        }
    });
}

impl Controller {
    /// Starts the controller. Must be called from within a tokio runtime.
    pub fn spawn(deps: ControllerDeps) -> ControllerHandle {
        let (tx, mut rx) = unbounded_channel();
        let ControllerDeps {
            hotkeys,
            levels,
            recorder,
            stt,
            models,
            inserter,
            store,
            emit,
            arm_esc,
            app_name,
            mic_denied,
            on_no_model,
        } = deps;

        let hk_tx = tx.clone();
        std::thread::spawn(move || {
            for ev in hotkeys {
                if hk_tx.send(Msg::Hotkey(ev)).is_err() {
                    break;
                }
            }
        });
        if let Some(levels) = levels {
            let emit = emit.clone();
            std::thread::spawn(move || {
                let mut last: Option<std::time::Instant> = None;
                while let Ok(level) = levels.recv() {
                    if last.is_none_or(|t| t.elapsed() >= LEVEL_EVERY) {
                        last = Some(std::time::Instant::now());
                        emit(events::LEVEL, json!(level));
                    }
                }
            });
        }

        let mut c = Controller {
            rec_tx: spawn_recorder_thread(recorder, tx.clone()),
            next_rec_id: 0,
            stopping: HashMap::new(),
            d: ControllerDeps2 {
                stt,
                models,
                inserter,
                store,
                emit,
                arm_esc,
                app_name,
                mic_denied,
                on_no_model,
            },
            tx: tx.clone(),
            state: State::Idle,
            last_emitted: None,
            paused: false,
            active: None,
            slots: VecDeque::new(),
            next_seq: 0,
            cur_meta: None,
            inserting: None,
            queue: VecDeque::new(),
        };
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(TICK_EVERY);
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                let needs_tick = matches!(
                    c.state,
                    State::Recording {
                        hands_free: true,
                        ..
                    } | State::Error { .. }
                );
                tokio::select! {
                    msg = rx.recv() => match msg {
                        None | Some(Msg::Shutdown) => break,
                        Some(m) => c.handle(m),
                    },
                    _ = tick.tick(), if needs_tick => c.run(Input::Tick),
                }
            }
            c.run(Input::Hotkey(HotkeyEvent::Cancel));
        });
        ControllerHandle { tx }
    }

    fn handle(&mut self, msg: Msg) {
        self.handle_msg(msg);
        // A hotkey release (or any state change away from "held") may unblock queued inserts.
        self.pump_queue();
    }

    /// While the chord is physically held, pasting would turn the synthetic Ctrl/Cmd+V into
    /// Ctrl+Win+V / Cmd+Opt+V. Hands-free recordings do not hold the chord.
    fn chord_held(&self) -> bool {
        matches!(
            self.state,
            State::Recording {
                hands_free: false,
                ..
            }
        )
    }

    fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Hotkey(ev) if !self.paused => {
                let starts = matches!(ev, HotkeyEvent::Pressed | HotkeyEvent::DoubleTap)
                    && !matches!(self.state, State::Recording { .. });
                if starts && self.d.models.active().is_none() {
                    // Straight to the error: no Recording flicker, Esc never armed.
                    (self.d.on_no_model)();
                    self.run(Input::NoModel);
                } else if starts && (self.d.mic_denied)() {
                    self.toast("error", machine::MIC_DENIED);
                    self.run(Input::MicDenied);
                } else {
                    self.run(Input::Hotkey(ev));
                }
            }
            Msg::Hotkey(_) | Msg::Shutdown => {}
            Msg::Paused(p) => {
                self.paused = p;
                if p {
                    self.run(Input::Hotkey(HotkeyEvent::Cancel));
                }
            }
            Msg::Finalized { seq, res } => {
                let res = match res {
                    Ok(Some(text)) => SlotRes::Text(text),
                    Ok(None) => SlotRes::NoSpeech,
                    Err(e) => SlotRes::Stt(e),
                };
                self.resolve(seq, res);
            }
            Msg::RecStarted { id } => {
                if let Some(a) = self.active.as_mut().filter(|a| a.id == id) {
                    a.started = true;
                    if a.pump_ended {
                        self.run(Input::RecorderFailed(RecorderError::DeviceLost));
                    }
                }
            }
            Msg::RecStartFailed { id, err } => {
                if self.active.as_ref().is_some_and(|a| a.id == id) {
                    if let Some(a) = self.active.take() {
                        discard_pump(a.pump);
                    }
                    self.run(Input::RecorderFailed(err));
                }
            }
            Msg::PumpEnded { id } => {
                if let Some(a) = self.active.as_mut().filter(|a| a.id == id) {
                    // Stop and cancel take `active` first, so this is an unexpected end.
                    if a.started {
                        self.run(Input::RecorderFailed(RecorderError::DeviceLost));
                    } else {
                        a.pump_ended = true;
                    }
                }
            }
            Msg::RecStopped { seq, res } => self.recorder_stopped(seq, res),
            Msg::InsertDone(Err(msg)) => {
                self.inserting = None;
                self.run(Input::Failed(msg));
                self.pump_queue();
            }
            Msg::InsertDone(Ok(out)) => {
                let outcome = match out {
                    Some((text, outcome)) => {
                        if let Some(job) = self.inserting.as_mut() {
                            job.text = text;
                        }
                        if let InsertOutcome::CopiedOnly(reason) = &outcome {
                            tracing::info!("text was copied but not pasted: {reason}");
                            self.toast("warning", COPIED_ONLY_MESSAGE);
                        }
                        outcome
                    }
                    None => InsertOutcome::Pasted, // nothing left after post-processing
                };
                self.run(Input::Inserted(outcome));
                self.inserting = None;
                self.pump_queue();
            }
        }
    }

    /// Feeds finished utterances to the machine strictly in utterance order, one insert at a time.
    fn pump_queue(&mut self) {
        while self.inserting.is_none()
            && !self.chord_held()
            && self.slots.front().is_some_and(|s| s.res.is_some())
        {
            let slot = self.slots.pop_front().expect("front checked");
            self.cur_meta = Some(slot.meta);
            let input = match slot.res.expect("ready") {
                SlotRes::Text(raw) => Input::Transcribed(Ok(raw)),
                SlotRes::NoSpeech => Input::NoSpeech,
                SlotRes::Stt(e) => Input::Transcribed(Err(e)),
                SlotRes::Failed(msg) => Input::Failed(msg),
            };
            self.run(input);
            self.cur_meta = None;
        }
    }

    fn resolve(&mut self, seq: u64, res: SlotRes) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.seq == seq) {
            slot.res = Some(res);
        }
        self.pump_queue();
    }

    /// Steps the machine and performs effects until no follow-up inputs remain.
    fn run(&mut self, input: Input) {
        self.queue.push_back(input);
        while let Some(input) = self.queue.pop_front() {
            let (mut next, effects) = step(self.state.clone(), input, Instant::now());
            if next == State::Idle && !self.slots.is_empty() {
                // Earlier utterances are still being transcribed or are waiting to be inserted.
                next = State::Transcribing;
            }
            self.state = next;
            for e in effects {
                self.perform(e);
            }
        }
    }

    fn perform(&mut self, effect: Effect) {
        match effect {
            Effect::StartRecording => self.start_recording(),
            Effect::CancelRecording => {
                if let Some(active) = self.active.take() {
                    let _ = self.rec_tx.send(RecCmd::Cancel);
                    discard_pump(active.pump);
                }
            }
            Effect::StopRecording => self.stop_recording(),
            // Handled by `stop_recording`, which reserves the utterance's queue slot.
            Effect::Transcribe => {}
            Effect::Insert(raw) => self.insert(raw),
            Effect::Persist => self.persist(),
            Effect::ShowError(msg) => self.toast("error", &msg),
            Effect::ArmEsc(armed) => (self.d.arm_esc)(armed),
            Effect::Emit(_) => self.emit_state(),
        }
    }

    fn emit_state(&mut self) {
        let v = serde_json::to_value(&self.state).unwrap_or(Value::Null);
        if self.last_emitted.as_ref() != Some(&v) {
            self.last_emitted = Some(v.clone());
            (self.d.emit)(events::STATE, v);
        }
    }

    fn toast(&self, level: &str, message: &str) {
        (self.d.emit)(events::TOAST, json!({ "level": level, "message": message }));
    }

    fn start_recording(&mut self) {
        let Some(model) = self.d.models.active() else {
            self.queue.push_back(Input::NoModel);
            return;
        };
        if let Some(stale) = self.active.take() {
            let _ = self.rec_tx.send(RecCmd::Cancel);
            discard_pump(stale.pump);
        }
        let app_name = (self.d.app_name)();
        let model_id = model.entry.id.clone();
        let session = self.d.stt.session(model);
        let (frame_tx, frame_rx) = std::sync::mpsc::channel();
        let id = self.next_rec_id;
        self.next_rec_id += 1;
        let pump = match spawn_pump(frame_rx, session, id, self.tx.clone()) {
            Ok(p) => p,
            Err(e) => {
                self.queue
                    .push_back(Input::RecorderFailed(RecorderError::Other(e.to_string())));
                return;
            }
        };
        self.active = Some(Active {
            id,
            started: false,
            pump_ended: false,
            pump,
            app_name,
            model_id,
        });
        let _ = self.rec_tx.send(RecCmd::Start { id, frame_tx });
    }

    /// Reserves the utterance's place in the queue and asks the recorder thread to stop; the
    /// finalize task starts when `RecStopped` arrives.
    fn stop_recording(&mut self) {
        let Some(active) = self.active.take() else {
            return;
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        let meta = Meta {
            app_name: active.app_name,
            model_id: active.model_id,
            duration_ms: 0,
        };
        self.slots.push_back(Slot {
            seq,
            meta,
            res: None,
        });
        self.stopping.insert(seq, active.pump);
        let _ = self.rec_tx.send(RecCmd::Stop { seq });
    }

    fn recorder_stopped(&mut self, seq: u64, res: Result<RecordingStats, RecorderError>) {
        let Some(pump) = self.stopping.remove(&seq) else {
            return;
        };
        let stats = match res {
            Ok(stats) => stats,
            Err(e) => {
                discard_pump(pump);
                self.resolve(seq, SlotRes::Failed(e.to_string()));
                return;
            }
        };
        if let Some(slot) = self.slots.iter_mut().find(|s| s.seq == seq) {
            slot.meta.duration_ms = stats.duration_ms;
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            // Inner task so that a panic in the engine/session becomes an error, not a wedge.
            let work = tokio::spawn(async move {
                let (mut planner, session) = tokio::task::spawn_blocking(move || pump.join())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .ok_or_else(|| SttError::EngineFailed("audio pump failed".into()))?;
                if planner.any_speech() {
                    let tail = planner.finish();
                    session.finish(tail).await.map(Some)
                } else {
                    session.cancel();
                    Ok(None)
                }
            });
            let res = work.await.unwrap_or_else(|_| {
                Err(SttError::EngineFailed("transcription task crashed".into()))
            });
            let _ = tx.send(Msg::Finalized { seq, res });
        });
    }

    fn insert(&mut self, raw: String) {
        let Some(meta) = self.cur_meta.take() else {
            return;
        };
        self.inserting = Some(Job {
            raw: raw.clone(),
            meta,
            text: String::new(),
        });
        let inserter = self.d.inserter.clone();
        let store = self.d.store.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            // Blocking: reads the dictionary and the insert sleeps for the clipboard restore.
            let done = tokio::task::spawn_blocking(move || {
                let dict = store.dictionary_list().unwrap_or_else(|e| {
                    tracing::warn!("could not read the dictionary, inserting without it: {e:#}");
                    Vec::new()
                });
                let text = postprocess(&raw, &dict, inserter.context());
                (!text.is_empty()).then(|| {
                    let outcome = inserter.insert(&text);
                    (text, outcome)
                })
            })
            .await
            .map_err(|e| {
                tracing::error!("insert task failed: {e}");
                "Inserting the text failed unexpectedly.".to_string()
            });
            let _ = tx.send(Msg::InsertDone(done));
        });
    }

    fn persist(&mut self) {
        let Some(job) = self.inserting.take() else {
            return;
        };
        if job.text.is_empty() {
            return;
        }
        let words = job.text.split_whitespace().count() as i64;
        let duration_ms = job.meta.duration_ms as i64;
        let wpm = if duration_ms > 0 {
            words as f64 / (duration_ms as f64 / 60_000.0)
        } else {
            0.0
        };
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let row = NewTranscript {
            created_at,
            text: job.text,
            raw_text: job.raw,
            app_name: job.meta.app_name,
            duration_ms,
            word_count: words,
            wpm,
            model_id: job.meta.model_id,
        };
        match self.d.store.insert_transcript(row) {
            Ok(id) => (self.d.emit)(events::TRANSCRIPT_ADDED, json!({ "id": id })),
            Err(e) => {
                tracing::error!("saving transcript failed: {e:#}");
                self.toast("error", "Could not save the dictation to history.");
            }
        }
    }
}
