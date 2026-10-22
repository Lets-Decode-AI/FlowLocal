//! Dictation controller: hotkeys -> recorder -> streaming chunked STT -> post-process ->
//! insert at the cursor -> history. All decisions live in the pure [`machine`]; the
//! [`Controller`] (a tokio task) performs the machine's effects.
//!
//! Collaborators sit behind small ports so tests can fake them.

pub mod machine;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::time::{Instant, MissedTickBehavior};

use crate::audio::{Recorder, RecorderError, RecordingStats};
use crate::events;
use crate::hotkey::HotkeyEvent;
use crate::insert::{InsertOutcome, Inserter};
use crate::models::{InstalledModel, Manager};
use crate::postprocess::postprocess;
use crate::store::{NewTranscript, Store};
use crate::stt::{Chunk, ChunkPlanner, ChunkerConfig, SttError, SttHandle, SttSession};
use machine::{step, Effect, Input, State};

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
    fn finish(self: Box<Self>, tail: Option<Chunk>) -> BoxFuture<'static, Result<String, SttError>>;
    fn cancel(self: Box<Self>);
}

impl SessionPort for SttSession {
    fn feed(&self, chunk: Chunk) {
        SttSession::feed(self, chunk)
    }
    fn finish(self: Box<Self>, tail: Option<Chunk>) -> BoxFuture<'static, Result<String, SttError>> {
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
        Manager::active(self)
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
    pub emit: Arc<dyn Fn(&str, Value) + Send + Sync>,
    /// `HotkeyService::set_armed`.
    pub arm_esc: Arc<dyn Fn(bool) + Send + Sync>,
    /// Name of the frontmost app (`insert::frontmost_app_name`), read at key-down.
    pub app_name: Arc<dyn Fn() -> Option<String> + Send + Sync>,
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

/// Minimum spacing of level events (~30 Hz).
const LEVEL_EVERY: Duration = Duration::from_millis(33);
const TICK_EVERY: Duration = Duration::from_millis(250);

enum Msg {
    Hotkey(HotkeyEvent),
    Paused(bool),
    /// An utterance's transcription finished (`Ok(None)`: no speech).
    Finalized { seq: u64, res: Result<Option<String>, SttError> },
    InsertDone(Option<(String, InsertOutcome)>),
    Shutdown,
}

type Pump = JoinHandle<(ChunkPlanner, Box<dyn SessionPort>)>;

/// A recording in progress.
struct Active {
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
    res: Option<Result<Option<String>, SttError>>,
}

struct Job {
    raw: String,
    meta: Meta,
    /// Post-processed text that was inserted (empty until the insert finishes).
    text: String,
}

pub struct Controller {
    d: ControllerDeps2,
    tx: UnboundedSender<Msg>,
    state: State,
    last_emitted: Option<Value>,
    paused: bool,
    active: Option<Active>,
    /// Recording stopped by `StopRecording`, waiting for the `Transcribe` effect.
    stopped: Option<(Active, RecordingStats)>,
    slots: VecDeque<Slot>,
    next_seq: u64,
    cur_meta: Option<Meta>,
    inserting: Option<Job>,
    queue: VecDeque<Input>,
}

/// `ControllerDeps` minus the receivers that move into helper threads.
struct ControllerDeps2 {
    recorder: Arc<dyn RecorderPort>,
    stt: Arc<dyn SttPort>,
    models: Arc<dyn ModelSource>,
    inserter: Arc<dyn Inserter>,
    store: Arc<Store>,
    emit: Arc<dyn Fn(&str, Value) + Send + Sync>,
    arm_esc: Arc<dyn Fn(bool) + Send + Sync>,
    app_name: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

fn spawn_pump(frames: Receiver<Vec<f32>>, session: Box<dyn SessionPort>) -> std::io::Result<Pump> {
    std::thread::Builder::new().name("flowlocal-pump".into()).spawn(move || {
        let mut planner = ChunkPlanner::new(ChunkerConfig::default());
        while let Ok(frame) = frames.recv() {
            for chunk in planner.push(&frame) {
                session.feed(chunk);
            }
        }
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
        let ControllerDeps { hotkeys, levels, recorder, stt, models, inserter, store, emit, arm_esc, app_name } = deps;

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
            d: ControllerDeps2 { recorder, stt, models, inserter, store, emit, arm_esc, app_name },
            tx: tx.clone(),
            state: State::Idle,
            last_emitted: None,
            paused: false,
            active: None,
            stopped: None,
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
                let needs_tick = matches!(c.state, State::Recording { hands_free: true, .. } | State::Error { .. });
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
        match msg {
            Msg::Hotkey(ev) if !self.paused => self.run(Input::Hotkey(ev)),
            Msg::Hotkey(_) | Msg::Shutdown => {}
            Msg::Paused(p) => {
                self.paused = p;
                if p {
                    self.run(Input::Hotkey(HotkeyEvent::Cancel));
                }
            }
            Msg::Finalized { seq, res } => {
                if let Some(slot) = self.slots.iter_mut().find(|s| s.seq == seq) {
                    slot.res = Some(res);
                }
                self.pump_queue();
            }
            Msg::InsertDone(out) => {
                let outcome = match out {
                    Some((text, outcome)) => {
                        if let Some(job) = self.inserting.as_mut() {
                            job.text = text;
                        }
                        if let InsertOutcome::CopiedOnly(reason) = &outcome {
                            self.toast("info", reason);
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
        while self.inserting.is_none() && self.slots.front().is_some_and(|s| s.res.is_some()) {
            let slot = self.slots.pop_front().expect("front checked");
            self.cur_meta = Some(slot.meta);
            let input = match slot.res.expect("ready") {
                Ok(Some(raw)) => Input::Transcribed(Ok(raw)),
                Ok(None) => Input::NoSpeech,
                Err(e) => Input::Transcribed(Err(e)),
            };
            self.run(input);
            self.cur_meta = None;
        }
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
                    self.d.recorder.cancel();
                    discard_pump(active.pump);
                }
            }
            Effect::StopRecording => self.stop_recording(),
            Effect::Transcribe => self.transcribe(),
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
            self.d.recorder.cancel();
            discard_pump(stale.pump);
        }
        let app_name = (self.d.app_name)();
        let model_id = model.entry.id.clone();
        let session = self.d.stt.session(model);
        let (frame_tx, frame_rx) = std::sync::mpsc::channel();
        let pump = match spawn_pump(frame_rx, session) {
            Ok(p) => p,
            Err(e) => {
                self.queue.push_back(Input::RecorderFailed(RecorderError::Other(e.to_string())));
                return;
            }
        };
        self.active = Some(Active { pump, app_name, model_id });
        if let Err(e) = self.d.recorder.start(frame_tx) {
            if let Some(a) = self.active.take() {
                discard_pump(a.pump);
            }
            self.queue.push_back(Input::RecorderFailed(e));
        }
    }

    fn stop_recording(&mut self) {
        let Some(active) = self.active.take() else { return };
        match self.d.recorder.stop() {
            Ok(stats) => self.stopped = Some((active, stats)),
            Err(e) => {
                discard_pump(active.pump);
                self.queue.push_back(Input::RecorderFailed(e));
            }
        }
    }

    /// Joins the pump, then finishes the session with the planner's tail in the background.
    fn transcribe(&mut self) {
        let Some((active, stats)) = self.stopped.take() else { return };
        let seq = self.next_seq;
        self.next_seq += 1;
        let meta = Meta { app_name: active.app_name, model_id: active.model_id, duration_ms: stats.duration_ms };
        self.slots.push_back(Slot { seq, meta, res: None });
        let pump = active.pump;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let joined = tokio::task::spawn_blocking(move || pump.join()).await;
            let res = match joined {
                Ok(Ok((mut planner, session))) => {
                    if planner.any_speech() {
                        let tail = planner.finish();
                        session.finish(tail).await.map(Some)
                    } else {
                        session.cancel();
                        Ok(None)
                    }
                }
                _ => Err(SttError::EngineFailed("audio pump failed".into())),
            };
            let _ = tx.send(Msg::Finalized { seq, res });
        });
    }

    fn insert(&mut self, raw: String) {
        let Some(meta) = self.cur_meta.take() else { return };
        self.inserting = Some(Job { raw: raw.clone(), meta, text: String::new() });
        let inserter = self.d.inserter.clone();
        let store = self.d.store.clone();
        let tx = self.tx.clone();
        // Blocking: reads the dictionary and the insert sleeps for the clipboard restore.
        tokio::task::spawn_blocking(move || {
            let dict = store.dictionary_list().unwrap_or_default();
            let text = postprocess(&raw, &dict, inserter.context());
            let out = (!text.is_empty()).then(|| {
                let outcome = inserter.insert(&text);
                (text, outcome)
            });
            let _ = tx.send(Msg::InsertDone(out));
        });
    }

    fn persist(&mut self) {
        let Some(job) = self.inserting.take() else { return };
        if job.text.is_empty() {
            return;
        }
        let words = job.text.split_whitespace().count() as i64;
        let duration_ms = job.meta.duration_ms as i64;
        let wpm = if duration_ms > 0 { words as f64 / (duration_ms as f64 / 60_000.0) } else { 0.0 };
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
                self.toast("error", "Could not save to history");
            }
        }
    }
}
