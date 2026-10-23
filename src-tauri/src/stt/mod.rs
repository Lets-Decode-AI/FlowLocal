//! Speech-to-text: engine abstraction, worker thread, streaming chunked sessions.

pub mod chunker;
pub mod session;
pub mod sherpa;
pub mod worker;

#[cfg(test)]
mod testutil;

use crate::models::InstalledModel;

pub use chunker::{Chunk, ChunkPlanner, ChunkerConfig, join_chunks};
pub use session::SttSession;
pub use sherpa::SherpaEngine;
pub use worker::{SttHandle, SttWorker};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SttError {
    #[error("could not load speech model: {0}")]
    LoadFailed(String),
    #[error("speech engine failed: {0}")]
    EngineFailed(String),
    #[error("transcription cancelled")]
    Cancelled,
    #[error("speech worker is not running")]
    WorkerGone,
}

/// A speech recognizer. Instances live on the STT worker thread only.
pub trait SttEngine: Send {
    fn load(&mut self, m: &InstalledModel) -> anyhow::Result<()>;
    /// `samples` are 16 kHz mono f32.
    fn transcribe(&mut self, samples: &[f32]) -> anyhow::Result<String>;
    fn unload(&mut self);
    fn loaded_id(&self) -> Option<&str>;
}
