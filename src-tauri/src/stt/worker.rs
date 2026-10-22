//! The STT worker: one thread owns the engine and drains one FIFO command queue.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use tokio::sync::oneshot;

use super::{SttEngine, SttError, SttSession};
use crate::models::InstalledModel;

/// One chunk of audio to transcribe.
pub struct Job {
    pub model: InstalledModel,
    pub samples: Vec<f32>,
    pub cancelled: Arc<AtomicBool>,
    pub reply: oneshot::Sender<Result<String, SttError>>,
}

enum Cmd {
    Load {
        model: InstalledModel,
        reply: Option<oneshot::Sender<Result<(), SttError>>>,
    },
    Job(Job),
    SetIdle(Option<Duration>),
}

pub struct SttWorker;

impl SttWorker {
    /// Starts the worker thread. The engine lives only on that thread. The thread exits when the
    /// last [`SttHandle`] is dropped.
    pub fn spawn(engine: Box<dyn SttEngine>, idle_unload: Option<Duration>) -> SttHandle {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("flowlocal-stt".into())
            .spawn(move || run(engine, idle_unload, rx))
            .expect("spawn stt worker thread");
        SttHandle { tx }
    }
}

#[derive(Clone)]
pub struct SttHandle {
    tx: Sender<Cmd>,
}

impl SttHandle {
    /// Starts a session and begins loading `model` right away if it is not loaded.
    pub fn session(&self, model: InstalledModel) -> SttSession {
        let _ = self.tx.send(Cmd::Load {
            model: model.clone(),
            reply: None,
        });
        SttSession::new(self.clone(), model)
    }

    /// Loads `model` (no-op if already loaded) and waits for it.
    pub async fn preload(&self, model: InstalledModel) -> Result<(), SttError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Load {
                model,
                reply: Some(reply),
            })
            .map_err(|_| SttError::WorkerGone)?;
        rx.await.map_err(|_| SttError::WorkerGone)?
    }

    pub fn set_idle_unload(&self, idle: Option<Duration>) {
        let _ = self.tx.send(Cmd::SetIdle(idle));
    }

    /// A handle whose worker is gone (tests).
    #[cfg(test)]
    pub(super) fn dead() -> SttHandle {
        SttHandle {
            tx: mpsc::channel().0,
        }
    }

    /// Queues a job; false if the worker is gone.
    pub(super) fn submit(&self, job: Job) -> bool {
        self.tx.send(Cmd::Job(job)).is_ok()
    }
}

fn ensure_loaded(engine: &mut dyn SttEngine, model: &InstalledModel) -> Result<(), SttError> {
    if engine.loaded_id() == Some(model.entry.id.as_str()) {
        return Ok(());
    }
    if engine.loaded_id().is_some() {
        engine.unload();
    }
    // One automatic retry: a failed (or panicking) load is attempted a second time.
    let mut last = String::new();
    for _ in 0..2 {
        match catch_unwind(AssertUnwindSafe(|| engine.load(model))) {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(e)) => last = format!("{e:#}"),
            Err(_) => {
                engine.unload();
                last = "engine panicked while loading".into();
            }
        }
    }
    Err(SttError::LoadFailed(last))
}

fn guarded_transcribe(
    engine: &mut dyn SttEngine,
    samples: &[f32],
) -> Result<String, Option<String>> {
    match catch_unwind(AssertUnwindSafe(|| engine.transcribe(samples))) {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(e)) => Err(Some(format!("{e:#}"))),
        Err(_) => Err(None), // panicked
    }
}

fn transcribe(engine: &mut dyn SttEngine, job: &Job) -> Result<String, SttError> {
    ensure_loaded(engine, &job.model)?;
    match guarded_transcribe(engine, &job.samples) {
        Ok(t) => Ok(t),
        Err(Some(msg)) => Err(SttError::EngineFailed(msg)),
        Err(None) => {
            // Panic: drop the engine state, reload once and retry once.
            tracing::error!("stt engine panicked; reloading");
            engine.unload();
            ensure_loaded(engine, &job.model)?;
            guarded_transcribe(engine, &job.samples).map_err(|e| {
                engine.unload();
                SttError::EngineFailed(e.unwrap_or_else(|| "engine panicked twice".into()))
            })
        }
    }
}

fn run(mut engine: Box<dyn SttEngine>, mut idle: Option<Duration>, rx: mpsc::Receiver<Cmd>) {
    loop {
        let cmd = match idle.filter(|_| engine.loaded_id().is_some()) {
            Some(d) => match rx.recv_timeout(d) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => {
                    engine.unload();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(c) => c,
                Err(_) => break,
            },
        };
        match cmd {
            Cmd::SetIdle(d) => idle = d,
            Cmd::Load { model, reply } => {
                let r = ensure_loaded(engine.as_mut(), &model);
                if let Err(e) = &r {
                    tracing::warn!("model load failed: {e}");
                }
                if let Some(reply) = reply {
                    let _ = reply.send(r);
                }
            }
            Cmd::Job(job) => {
                let r = if job.cancelled.load(Ordering::SeqCst) {
                    Err(SttError::Cancelled)
                } else {
                    transcribe(engine.as_mut(), &job)
                };
                let _ = job.reply.send(r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use super::*;
    use crate::stt::SttError;
    use crate::stt::testutil::*;

    async fn one(h: &SttHandle, model: &str, id: usize) -> Result<String, SttError> {
        let s = h.session(fake_model(model));
        s.feed(speech_chunk(id));
        s.finish(None).await
    }

    #[tokio::test]
    async fn worker_loads_once_and_reuses() {
        let probe = Probe::default();
        let h = SttWorker::spawn(Box::new(FakeEngine::new(&probe, Duration::ZERO)), None);
        assert_eq!(one(&h, "a", 1).await.unwrap(), "c1");
        assert_eq!(one(&h, "a", 2).await.unwrap(), "c2");
        assert_eq!(probe.count("load"), 1);
    }

    #[tokio::test]
    async fn worker_swaps_on_model_change() {
        let probe = Probe::default();
        let h = SttWorker::spawn(Box::new(FakeEngine::new(&probe, Duration::ZERO)), None);
        one(&h, "a", 1).await.unwrap();
        one(&h, "b", 2).await.unwrap();
        let ev: Vec<_> = probe
            .names()
            .into_iter()
            .filter(|n| !n.contains(" c"))
            .collect();
        assert_eq!(ev, ["load a", "unload", "load b"]);
    }

    #[tokio::test]
    async fn worker_unloads_after_idle() {
        let probe = Probe::default();
        let h = SttWorker::spawn(
            Box::new(FakeEngine::new(&probe, Duration::ZERO)),
            Some(Duration::from_millis(50)),
        );
        one(&h, "a", 1).await.unwrap();
        assert_eq!(probe.count("unload"), 0);
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(probe.count("unload"), 1);
        one(&h, "a", 2).await.unwrap();
        assert_eq!(probe.count("load"), 2);
        h.set_idle_unload(None);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(probe.count("unload"), 1, "idle unload disabled");
    }

    #[tokio::test]
    async fn preload_loads_without_a_session() {
        let probe = Probe::default();
        let h = SttWorker::spawn(Box::new(FakeEngine::new(&probe, Duration::ZERO)), None);
        h.preload(fake_model("a")).await.unwrap();
        assert_eq!(probe.names(), ["load a"]);
    }

    #[tokio::test]
    async fn panic_reloads_once_then_errors() {
        // one panic: reload and retry succeeds
        let probe = Probe::default();
        let eng = FakeEngine::new(&probe, Duration::ZERO);
        eng.panics.store(1, Ordering::SeqCst);
        let h = SttWorker::spawn(Box::new(eng), None);
        assert_eq!(one(&h, "a", 3).await.unwrap(), "c3");
        assert_eq!(probe.count("load"), 2);

        // panics every time: exactly one reload, then EngineFailed
        let probe = Probe::default();
        let eng = FakeEngine::new(&probe, Duration::ZERO);
        eng.panics.store(usize::MAX, Ordering::SeqCst);
        let h = SttWorker::spawn(Box::new(eng), None);
        assert!(matches!(
            one(&h, "a", 3).await,
            Err(SttError::EngineFailed(_))
        ));
        assert_eq!(probe.count("load"), 2);
        assert_eq!(probe.count("start"), 2);
    }

    #[tokio::test]
    async fn load_failure_retries_once_then_errors() {
        let probe = Probe::default();
        let eng = FakeEngine::new(&probe, Duration::ZERO);
        eng.load_failures.store(1, Ordering::SeqCst);
        let h = SttWorker::spawn(Box::new(eng), None);
        assert_eq!(one(&h, "a", 1).await.unwrap(), "c1");
        assert_eq!(probe.count("load"), 2);

        let probe = Probe::default();
        let eng = FakeEngine::new(&probe, Duration::ZERO);
        eng.load_failures.store(2, Ordering::SeqCst);
        let h = SttWorker::spawn(Box::new(eng), None);
        assert!(matches!(
            h.preload(fake_model("a")).await,
            Err(SttError::LoadFailed(_))
        ));
        assert_eq!(probe.count("load"), 2);
    }
}
