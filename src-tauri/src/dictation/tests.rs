//! Controller tests: fakes for every port, tokio paused time, polling instead of timers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::*;
use crate::audio::{FRAME_LEN, RecorderError, RecordingStats, SAMPLE_RATE};
use crate::hotkey::HotkeyEvent;
use crate::insert::{InsertOutcome, Inserter};
use crate::models::InstalledModel;
use crate::models::catalog::{CatalogEntry, EngineKind};
use crate::postprocess::InsertContext;
use crate::store::Store;
use crate::stt::{Chunk, SttError};

const SR: usize = SAMPLE_RATE as usize;

fn model(id: &str) -> InstalledModel {
    InstalledModel {
        entry: CatalogEntry {
            id: id.into(),
            name: id.into(),
            hf_repo: String::new(),
            revision: String::new(),
            files: vec![],
            engine: EngineKind::Transducer,
            languages: vec![],
            min_ram_gb: 0,
            notes: String::new(),
        },
        dir: std::path::PathBuf::new(),
    }
}

/// Speech stand-in (see stt::chunker tests): a tone with a shallow dip every 2 s.
fn voiced(secs: f32) -> Vec<f32> {
    let n = (secs * SR as f32) as usize / FRAME_LEN * FRAME_LEN;
    (0..n)
        .map(|i| {
            let s = 0.3 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / SR as f32).sin();
            if (i / FRAME_LEN).is_multiple_of(100) {
                s * 0.02
            } else {
                s
            }
        })
        .collect()
}
fn silence(secs: f32) -> Vec<f32> {
    vec![0.0; (secs * SR as f32) as usize / FRAME_LEN * FRAME_LEN]
}

// ---- fakes ----------------------------------------------------------------------------------

struct FakeRecorder {
    audio: Mutex<Vec<f32>>,
    start_error: Mutex<Option<RecorderError>>,
    /// Simulates the device vanishing: the frame sender is dropped after the audio is sent.
    lose_device: std::sync::atomic::AtomicBool,
    tx: Mutex<Option<Sender<Vec<f32>>>>,
    starts: AtomicUsize,
    stops: AtomicUsize,
    cancels: AtomicUsize,
}
impl FakeRecorder {
    fn new(audio: Vec<f32>) -> Arc<Self> {
        Arc::new(Self {
            audio: Mutex::new(audio),
            start_error: Mutex::default(),
            lose_device: Default::default(),
            tx: Mutex::default(),
            starts: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
        })
    }
}
impl RecorderPort for FakeRecorder {
    fn start(&self, frame_tx: Sender<Vec<f32>>) -> Result<(), RecorderError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = self.start_error.lock().unwrap().clone() {
            return Err(e);
        }
        let audio = self.audio.lock().unwrap().clone();
        if self.lose_device.load(Ordering::SeqCst) {
            std::thread::spawn(move || {
                for f in audio.chunks(FRAME_LEN) {
                    let _ = frame_tx.send(f.to_vec());
                }
            });
            return Ok(());
        }
        let tx = frame_tx.clone();
        std::thread::spawn(move || {
            for f in audio.chunks(FRAME_LEN) {
                if tx.send(f.to_vec()).is_err() {
                    break;
                }
            }
        });
        *self.tx.lock().unwrap() = Some(frame_tx);
        Ok(())
    }
    fn stop(&self) -> Result<RecordingStats, RecorderError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.tx.lock().unwrap().take();
        Ok(RecordingStats {
            duration_ms: 30_000,
        })
    }
    fn cancel(&self) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        self.tx.lock().unwrap().take();
    }
}

#[derive(Default)]
struct SessionLog {
    feeds: AtomicUsize,
    finishes: AtomicUsize,
    cancels: AtomicUsize,
}
struct Script {
    result: Result<String, SttError>,
    gate: Option<Arc<Notify>>,
}
struct FakeSession {
    log: Arc<SessionLog>,
    script: Script,
}
impl SessionPort for FakeSession {
    fn feed(&self, _chunk: Chunk) {
        self.log.feeds.fetch_add(1, Ordering::SeqCst);
    }
    fn finish(
        self: Box<Self>,
        _tail: Option<Chunk>,
    ) -> BoxFuture<'static, Result<String, SttError>> {
        Box::pin(async move {
            self.log.finishes.fetch_add(1, Ordering::SeqCst);
            if let Some(g) = &self.script.gate {
                g.notified().await;
            }
            self.script.result.clone()
        })
    }
    fn cancel(self: Box<Self>) {
        self.log.cancels.fetch_add(1, Ordering::SeqCst);
    }
}
struct FakeStt {
    scripts: Mutex<VecDeque<Script>>,
    logs: Mutex<Vec<Arc<SessionLog>>>,
}
impl FakeStt {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::default(),
            logs: Mutex::default(),
        })
    }
    fn push(&self, text: &str, gate: Option<Arc<Notify>>) {
        self.scripts.lock().unwrap().push_back(Script {
            result: Ok(text.into()),
            gate,
        });
    }
    fn push_err(&self, e: SttError) {
        self.scripts.lock().unwrap().push_back(Script {
            result: Err(e),
            gate: None,
        });
    }
    fn log(&self, i: usize) -> Arc<SessionLog> {
        self.logs.lock().unwrap()[i].clone()
    }
    fn sessions(&self) -> usize {
        self.logs.lock().unwrap().len()
    }
}
impl SttPort for FakeStt {
    fn session(&self, _m: InstalledModel) -> Box<dyn SessionPort> {
        let log = Arc::new(SessionLog::default());
        self.logs.lock().unwrap().push(log.clone());
        let script = self.scripts.lock().unwrap().pop_front().unwrap_or(Script {
            result: Ok("hello world".into()),
            gate: None,
        });
        Box::new(FakeSession { log, script })
    }
}

struct FakeModels(Mutex<Option<InstalledModel>>);
impl ModelSource for FakeModels {
    fn active(&self) -> Option<InstalledModel> {
        self.0.lock().unwrap().clone()
    }
}

struct FakeInserter {
    inserted: Mutex<Vec<String>>,
    outcome: Mutex<InsertOutcome>,
    panic_next: std::sync::atomic::AtomicBool,
}
impl Inserter for FakeInserter {
    fn insert(&self, text: &str) -> InsertOutcome {
        if self.panic_next.swap(false, Ordering::SeqCst) {
            panic!("inserter exploded");
        }
        self.inserted.lock().unwrap().push(text.to_string());
        self.outcome.lock().unwrap().clone()
    }
    fn context(&self) -> InsertContext {
        InsertContext { prev_char: None }
    }
}

struct Harness {
    handle: ControllerHandle,
    hk: Sender<HotkeyEvent>,
    rec: Arc<FakeRecorder>,
    stt: Arc<FakeStt>,
    ins: Arc<FakeInserter>,
    store: Arc<Store>,
    events: Arc<Mutex<Vec<(String, Value)>>>,
    esc: Arc<Mutex<Vec<bool>>>,
    level_tx: std::sync::mpsc::SyncSender<f32>,
}

fn harness(audio: Vec<f32>, with_model: bool) -> Harness {
    let rec = FakeRecorder::new(audio);
    let stt = FakeStt::new();
    let ins = Arc::new(FakeInserter {
        inserted: Mutex::default(),
        outcome: Mutex::new(InsertOutcome::Pasted),
        panic_next: Default::default(),
    });
    let store = Arc::new(Store::open_in_memory().unwrap());
    let events: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let esc: Arc<Mutex<Vec<bool>>> = Arc::default();
    let (hk, hk_rx) = std::sync::mpsc::channel();
    let (level_tx, level_rx) = std::sync::mpsc::sync_channel(64);
    let ev = events.clone();
    let esc2 = esc.clone();
    let handle = Controller::spawn(ControllerDeps {
        hotkeys: hk_rx,
        levels: Some(level_rx),
        recorder: rec.clone(),
        stt: stt.clone(),
        models: Arc::new(FakeModels(Mutex::new(with_model.then(|| model("m1"))))),
        inserter: ins.clone(),
        store: store.clone(),
        emit: Arc::new(move |name, v| ev.lock().unwrap().push((name.to_string(), v))),
        arm_esc: Arc::new(move |b| esc2.lock().unwrap().push(b)),
        app_name: Arc::new(|| Some("Notes".to_string())),
    });
    Harness {
        handle,
        hk,
        rec,
        stt,
        ins,
        store,
        events,
        esc,
        level_tx,
    }
}

impl Harness {
    fn send(&self, e: HotkeyEvent) {
        self.hk.send(e).unwrap();
    }
    fn states(&self) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == crate::events::STATE)
            .map(|(_, v)| v.clone())
            .collect()
    }
    fn kinds(&self) -> Vec<String> {
        self.states()
            .iter()
            .map(|v| v["kind"].as_str().unwrap().to_string())
            .collect()
    }
    fn inserted(&self) -> Vec<String> {
        self.ins.inserted.lock().unwrap().clone()
    }
    fn saved(&self) -> Vec<crate::store::Transcript> {
        self.store.list_transcripts(0, 50, None).unwrap()
    }
}

/// Lets the controller and its helper threads run without touching tokio timers.
async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..5000 {
        if cond() {
            return;
        }
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for: {what}");
}
async fn settle() {
    for _ in 0..40 {
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(1));
    }
}
async fn advance(ms: u64) {
    tokio::time::advance(Duration::from_millis(ms)).await;
}

// ---- tests ----------------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn hold_speak_release_inserts_and_persists() {
    let h = harness(voiced(2.0), true);
    h.stt.push("hello  world", None);
    h.send(HotkeyEvent::Pressed);
    wait_for("recording started", || {
        h.rec.starts.load(Ordering::SeqCst) == 1
    })
    .await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("persisted", || h.saved().len() == 1).await;
    wait_for("idle", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;

    assert_eq!(h.inserted(), vec!["Hello world".to_string()]);
    let t = &h.saved()[0];
    assert_eq!(
        (t.text.as_str(), t.raw_text.as_str()),
        ("Hello world", "hello  world")
    );
    assert_eq!(t.app_name.as_deref(), Some("Notes"));
    assert_eq!(t.model_id, "m1");
    assert_eq!((t.duration_ms, t.word_count), (30_000, 2));
    assert!((t.wpm - 4.0).abs() < 1e-9, "wpm {}", t.wpm); // 2 words in 0.5 min
    assert_eq!(
        h.kinds(),
        ["recording", "transcribing", "inserting", "idle"]
    );
    assert_eq!(*h.esc.lock().unwrap(), [true, false]);
    assert!(
        h.events
            .lock()
            .unwrap()
            .iter()
            .any(|(n, _)| n == crate::events::TRANSCRIPT_ADDED)
    );
}

#[tokio::test(start_paused = true)]
async fn short_tap_is_discarded() {
    let h = harness(voiced(1.0), true);
    h.send(HotkeyEvent::Pressed);
    wait_for("started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(100).await;
    h.send(HotkeyEvent::Released);
    wait_for("idle", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;
    wait_for("session cancelled", || {
        h.stt.log(0).cancels.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(h.rec.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(h.rec.stops.load(Ordering::SeqCst), 0);
    assert!(h.inserted().is_empty() && h.saved().is_empty());
    assert_eq!(h.stt.log(0).finishes.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn no_speech_is_discarded() {
    let h = harness(silence(1.0), true);
    h.send(HotkeyEvent::Pressed);
    wait_for("started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("back to idle", || {
        h.kinds().ends_with(&["transcribing".into(), "idle".into()])
    })
    .await;
    wait_for("session cancelled", || {
        h.stt.log(0).cancels.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(h.stt.log(0).finishes.load(Ordering::SeqCst), 0);
    assert!(h.inserted().is_empty() && h.saved().is_empty());
}

#[tokio::test(start_paused = true)]
async fn chunks_fed_during_recording() {
    // 6 s speech, 0.5 s pause, 5.5 s speech: one chunk is cut at the pause while still recording.
    let mut audio = voiced(6.0);
    audio.extend(silence(0.5));
    audio.extend(voiced(5.5));
    let h = harness(audio, true);
    h.send(HotkeyEvent::Pressed);
    wait_for("one chunk fed before release", || {
        h.stt.sessions() == 1 && h.stt.log(0).feeds.load(Ordering::SeqCst) == 1
    })
    .await;
    settle().await;
    assert_eq!(h.stt.log(0).feeds.load(Ordering::SeqCst), 1);
    assert_eq!(
        h.stt.log(0).finishes.load(Ordering::SeqCst),
        0,
        "not finished before key-up"
    );
    assert_eq!(h.kinds(), ["recording"]);
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("inserted", || h.inserted().len() == 1).await;
}

#[tokio::test(start_paused = true)]
async fn double_tap_hands_free_then_press_stops() {
    let h = harness(voiced(2.0), true);
    h.send(HotkeyEvent::Pressed);
    wait_for("started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(100).await;
    h.send(HotkeyEvent::Released);
    wait_for("cancelled", || h.rec.cancels.load(Ordering::SeqCst) == 1).await;
    advance(100).await;
    h.send(HotkeyEvent::DoubleTap);
    wait_for("hands-free recording", || {
        h.states()
            .last()
            .is_some_and(|s| s["kind"] == "recording" && s["handsFree"] == true)
    })
    .await;
    advance(5000).await;
    assert!(h.inserted().is_empty());
    h.send(HotkeyEvent::Pressed);
    wait_for("inserted", || h.inserted().len() == 1).await;
    assert_eq!(h.rec.starts.load(Ordering::SeqCst), 2);
    assert_eq!(h.rec.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn hands_free_auto_stops_at_6_min() {
    let h = harness(voiced(2.0), true);
    h.send(HotkeyEvent::DoubleTap);
    wait_for("hands-free recording", || {
        h.states().last().is_some_and(|s| s["handsFree"] == true)
    })
    .await;
    advance(5 * 60 * 1000).await;
    settle().await;
    assert!(h.inserted().is_empty(), "still recording at 5 min");
    advance(61 * 1000).await;
    wait_for("auto-stopped and inserted", || h.inserted().len() == 1).await;
    assert_eq!(h.rec.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn cancel_discards() {
    let h = harness(voiced(2.0), true);
    h.send(HotkeyEvent::Pressed);
    wait_for("started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(1000).await;
    h.send(HotkeyEvent::Cancel);
    wait_for("idle", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;
    wait_for("session cancelled", || {
        h.stt.log(0).cancels.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(h.rec.cancels.load(Ordering::SeqCst), 1);
    assert!(h.inserted().is_empty() && h.saved().is_empty());
    assert_eq!(*h.esc.lock().unwrap(), [true, false]);
}

#[tokio::test(start_paused = true)]
async fn no_model_shows_error() {
    let h = harness(voiced(1.0), false);
    h.send(HotkeyEvent::Pressed);
    wait_for("error state", || {
        h.states()
            .iter()
            .any(|s| s["kind"] == "error" && s["message"] == "No model installed")
    })
    .await;
    assert_eq!(h.rec.starts.load(Ordering::SeqCst), 0);
    assert!(
        !h.kinds().contains(&"recording".to_string()),
        "no recording flicker: {:?}",
        h.kinds()
    );
    assert!(h.esc.lock().unwrap().is_empty(), "Esc never armed");
    advance(2100).await;
    wait_for("idle after 2 s", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn press_during_transcribing_starts_new_recording() {
    let h = harness(voiced(2.0), true);
    let gate_a = Arc::new(Notify::new());
    h.stt.push("alpha", Some(gate_a.clone()));
    h.stt.push("beta", None);

    h.send(HotkeyEvent::Pressed);
    wait_for("A started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("A transcribing", || {
        h.stt.log(0).finishes.load(Ordering::SeqCst) == 1
    })
    .await;

    h.send(HotkeyEvent::Pressed);
    wait_for("B recording", || h.rec.starts.load(Ordering::SeqCst) == 2).await;
    wait_for("recording state", || {
        h.kinds().last().map(String::as_str) == Some("recording")
    })
    .await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("B finished", || {
        h.stt.log(1).finishes.load(Ordering::SeqCst) == 1
    })
    .await;
    settle().await;
    assert!(
        h.inserted().is_empty(),
        "B must wait for A: {:?}",
        h.inserted()
    );

    gate_a.notify_one();
    wait_for("both inserted", || h.inserted().len() == 2).await;
    assert_eq!(h.inserted(), ["Alpha", "Beta"]);
    wait_for("both persisted", || h.saved().len() == 2).await;
    let texts: Vec<_> = h.saved().into_iter().map(|t| t.text).collect(); // newest first
    assert_eq!(texts, ["Beta", "Alpha"]);
    wait_for("idle", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn recorder_error_goes_to_error_then_idle() {
    let h = harness(voiced(1.0), true);
    *h.rec.start_error.lock().unwrap() = Some(RecorderError::PermissionDenied);
    h.send(HotkeyEvent::Pressed);
    wait_for("error state", || {
        h.states()
            .iter()
            .any(|s| s["kind"] == "error" && s["message"] == "microphone permission denied")
    })
    .await;
    assert!(h.inserted().is_empty());
    advance(2100).await;
    wait_for("idle", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;
    assert!(!*h.esc.lock().unwrap().last().unwrap());
}

#[tokio::test(start_paused = true)]
async fn copied_only_emits_toast_and_still_persists() {
    let h = harness(voiced(2.0), true);
    *h.ins.outcome.lock().unwrap() = InsertOutcome::CopiedOnly("secure input is active".into());
    h.send(HotkeyEvent::Pressed);
    wait_for("started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("persisted", || h.saved().len() == 1).await;
    let toasts: Vec<Value> = h
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, _)| n == crate::events::TOAST)
        .map(|(_, v)| v.clone())
        .collect();
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0]["message"], json!("secure input is active"));
}

#[tokio::test(start_paused = true)]
async fn paused_ignores_hotkeys() {
    let h = harness(voiced(2.0), true);
    h.handle.set_paused(true);
    h.send(HotkeyEvent::Pressed);
    settle().await;
    assert_eq!(h.rec.starts.load(Ordering::SeqCst), 0);
    assert!(h.kinds().is_empty());

    // unpaused: works again; pausing mid-recording cancels it
    h.handle.set_paused(false);
    h.send(HotkeyEvent::Pressed);
    wait_for("started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    h.handle.set_paused(true);
    wait_for("cancelled", || h.rec.cancels.load(Ordering::SeqCst) == 1).await;
    wait_for("idle", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn levels_are_throttled_to_30hz() {
    let h = harness(voiced(1.0), true);
    for i in 0..200 {
        h.level_tx.send(i as f32 / 200.0).unwrap_or(());
        if i % 50 == 49 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    settle().await;
    let n = h
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == crate::events::LEVEL)
        .count();
    assert!((1..=5).contains(&n), "{n} level events for a ~25 ms burst");
}

#[tokio::test(start_paused = true)]
async fn device_lost_mid_recording_shows_error_promptly() {
    let h = harness(voiced(1.0), true);
    h.rec.lose_device.store(true, Ordering::SeqCst);
    h.send(HotkeyEvent::Pressed);
    // No Released is ever sent: the loss must be noticed on its own.
    wait_for("error state", || {
        h.states()
            .iter()
            .any(|s| s["kind"] == "error" && s["message"] == "input device was lost")
    })
    .await;
    wait_for("session cancelled", || {
        h.stt.log(0).cancels.load(Ordering::SeqCst) == 1
    })
    .await;
    wait_for("recorder cancelled", || {
        h.rec.cancels.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(*h.esc.lock().unwrap(), [true, false]);
    assert!(h.inserted().is_empty());
    advance(2100).await;
    wait_for("idle", || {
        h.kinds().last().map(String::as_str) == Some("idle")
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn insert_panic_does_not_block_next_utterance() {
    let h = harness(voiced(2.0), true);
    h.ins.panic_next.store(true, Ordering::SeqCst);
    h.send(HotkeyEvent::Pressed);
    wait_for("A started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("error after panic", || {
        h.states().iter().any(|s| s["kind"] == "error")
    })
    .await;
    assert!(h.saved().is_empty());

    h.send(HotkeyEvent::Pressed);
    wait_for("B started", || h.rec.starts.load(Ordering::SeqCst) == 2).await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("B inserted", || h.inserted().len() == 1).await;
    wait_for("B persisted", || h.saved().len() == 1).await;
}

#[tokio::test(start_paused = true)]
async fn stt_error_in_a_still_inserts_b() {
    let h = harness(voiced(2.0), true);
    h.stt.push_err(SttError::EngineFailed("boom".into()));
    h.send(HotkeyEvent::Pressed);
    wait_for("A started", || h.rec.starts.load(Ordering::SeqCst) == 1).await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("A error", || {
        h.states().iter().any(|s| {
            s["kind"] == "error" && s["message"].as_str().is_some_and(|m| m.contains("boom"))
        })
    })
    .await;

    h.send(HotkeyEvent::Pressed);
    wait_for("B started", || h.rec.starts.load(Ordering::SeqCst) == 2).await;
    advance(1000).await;
    h.send(HotkeyEvent::Released);
    wait_for("B inserted", || h.inserted().len() == 1).await;
    wait_for("B persisted", || h.saved().len() == 1).await;
}
