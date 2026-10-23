//! A streaming transcription session: chunks are queued FIFO on the worker while recording continues.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

use super::chunker::join_chunks;
use super::worker::{Job, SttHandle};
use super::{Chunk, SttError};
use crate::models::InstalledModel;

pub struct SttSession {
    handle: SttHandle,
    model: InstalledModel,
    cancelled: Arc<AtomicBool>,
    /// A chunk could not be queued because the worker is gone.
    worker_gone: AtomicBool,
    pending: Mutex<Vec<oneshot::Receiver<Result<String, SttError>>>>,
}

impl SttSession {
    pub(super) fn new(handle: SttHandle, model: InstalledModel) -> Self {
        Self {
            handle,
            model,
            cancelled: Arc::default(),
            worker_gone: AtomicBool::new(false),
            pending: Mutex::default(),
        }
    }

    /// Queue a chunk for transcription; returns immediately. Chunks without speech are skipped.
    pub fn feed(&self, chunk: Chunk) {
        if !chunk.has_speech || chunk.samples.is_empty() {
            return;
        }
        let (reply, rx) = oneshot::channel();
        let job = Job {
            model: self.model.clone(),
            samples: chunk.samples,
            cancelled: self.cancelled.clone(),
            reply,
        };
        if self.handle.submit(job) {
            self.pending.lock().unwrap().push(rx);
        } else {
            self.worker_gone.store(true, Ordering::SeqCst);
        }
    }

    /// Waits for every queued chunk (in order) plus `tail`, and returns the joined text.
    pub async fn finish(self, tail: Option<Chunk>) -> Result<String, SttError> {
        if let Some(t) = tail {
            self.feed(t);
        }
        if self.worker_gone.load(Ordering::SeqCst) {
            return Err(SttError::WorkerGone);
        }
        let receivers = std::mem::take(&mut *self.pending.lock().unwrap());
        let mut texts = Vec::with_capacity(receivers.len());
        for rx in receivers {
            texts.push(rx.await.map_err(|_| SttError::WorkerGone)??);
        }
        Ok(join_chunks(&texts))
    }

    /// Drops chunks that have not started yet.
    pub fn cancel(self) {
        drop(self);
    }
}

impl Drop for SttSession {
    fn drop(&mut self) {
        // After `finish` every job has already completed, so this only affects abandoned work.
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::stt::testutil::*;
    use crate::stt::{Chunk, SttError, SttHandle, SttSession, SttWorker};

    #[tokio::test]
    async fn session_transcribes_chunks_in_order_while_feeding() {
        let probe = Probe::default();
        let h = SttWorker::spawn(
            Box::new(FakeEngine::new(&probe, Duration::from_millis(50))),
            None,
        );
        let s = h.session(fake_model("a"));
        for i in 0..3 {
            s.feed(speech_chunk(i));
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        let before_finish = Instant::now();
        let text = s.finish(None).await.unwrap();
        assert_eq!(text, "c0 c1 c2");
        let started = probe.time_of("start c0").unwrap();
        assert!(
            started < before_finish,
            "chunk 0 started before finish was called"
        );
        let order: Vec<_> = probe
            .names()
            .into_iter()
            .filter(|n| n.starts_with("start"))
            .collect();
        assert_eq!(order, ["start c0", "start c1", "start c2"]);
    }

    #[tokio::test]
    async fn session_finish_latency_is_only_tail() {
        let probe = Probe::default();
        let h = SttWorker::spawn(
            Box::new(FakeEngine::new(&probe, Duration::from_millis(100))),
            None,
        );
        h.preload(fake_model("a")).await.unwrap();
        let s = h.session(fake_model("a"));
        for i in 0..5 {
            s.feed(speech_chunk(i));
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let t = Instant::now();
        let text = s.finish(Some(speech_chunk(5))).await.unwrap();
        let took = t.elapsed();
        assert_eq!(text, "c0 c1 c2 c3 c4 c5");
        assert!(took < Duration::from_millis(250), "finish took {took:?}");
    }

    #[tokio::test]
    async fn session_cancel_drops_pending() {
        let probe = Probe::default();
        let h = SttWorker::spawn(
            Box::new(FakeEngine::new(&probe, Duration::from_millis(100))),
            None,
        );
        let s = h.session(fake_model("a"));
        for i in 0..4 {
            s.feed(speech_chunk(i));
        }
        tokio::time::sleep(Duration::from_millis(30)).await; // c0 in flight
        s.cancel();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(
            probe.count("start"),
            1,
            "queued chunks dropped: {:?}",
            probe.names()
        );
        // the worker still serves the next session
        let s2 = h.session(fake_model("a"));
        s2.feed(speech_chunk(9));
        assert_eq!(s2.finish(None).await.unwrap(), "c9");
    }

    #[tokio::test]
    async fn silent_chunks_not_sent_to_engine() {
        let probe = Probe::default();
        let h = SttWorker::spawn(Box::new(FakeEngine::new(&probe, Duration::ZERO)), None);
        let s = h.session(fake_model("a"));
        s.feed(Chunk {
            samples: vec![7.0; 160],
            has_speech: false,
            start_ms: 0,
        });
        s.feed(speech_chunk(1));
        let silent_tail = Chunk {
            samples: vec![8.0; 160],
            has_speech: false,
            start_ms: 0,
        };
        assert_eq!(s.finish(Some(silent_tail)).await.unwrap(), "c1");
        assert_eq!(probe.count("start"), 1);
    }

    #[tokio::test]
    async fn finish_errors_if_feed_failed_worker_gone() {
        let s = SttSession::new(SttHandle::dead(), fake_model("a"));
        s.feed(speech_chunk(1));
        assert_eq!(s.finish(None).await, Err(SttError::WorkerGone));
    }
}
