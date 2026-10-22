//! Fake engine and helpers shared by the stt unit tests.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use super::SttEngine;
use crate::models::catalog::{CatalogEntry, EngineKind};
use crate::models::InstalledModel;

/// Event log shared between a fake engine and its test.
#[derive(Clone, Default)]
pub struct Probe {
    events: Arc<Mutex<Vec<(String, Instant)>>>,
}

impl Probe {
    pub fn push(&self, s: impl Into<String>) {
        self.events.lock().unwrap().push((s.into(), Instant::now()));
    }
    pub fn names(&self) -> Vec<String> {
        self.events.lock().unwrap().iter().map(|e| e.0.clone()).collect()
    }
    pub fn count(&self, prefix: &str) -> usize {
        self.names().iter().filter(|n| n.starts_with(prefix)).count()
    }
    pub fn time_of(&self, name: &str) -> Option<Instant> {
        self.events.lock().unwrap().iter().find(|e| e.0 == name).map(|e| e.1)
    }
}

pub struct FakeEngine {
    pub probe: Probe,
    pub delay: Duration,
    /// Number of upcoming `transcribe` calls that panic.
    pub panics: Arc<AtomicUsize>,
    loaded: Option<String>,
}

impl FakeEngine {
    pub fn new(probe: &Probe, delay: Duration) -> Self {
        Self { probe: probe.clone(), delay, panics: Arc::default(), loaded: None }
    }
}

impl SttEngine for FakeEngine {
    fn load(&mut self, m: &InstalledModel) -> Result<()> {
        self.probe.push(format!("load {}", m.entry.id));
        self.loaded = Some(m.entry.id.clone());
        Ok(())
    }
    fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        if self.loaded.is_none() {
            bail!("not loaded");
        }
        let id = samples[0] as usize;
        self.probe.push(format!("start c{id}"));
        if self
            .panics
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            panic!("fake engine panic");
        }
        std::thread::sleep(self.delay);
        self.probe.push(format!("end c{id}"));
        Ok(format!("c{id}"))
    }
    fn unload(&mut self) {
        self.probe.push("unload");
        self.loaded = None;
    }
    fn loaded_id(&self) -> Option<&str> {
        self.loaded.as_deref()
    }
}

pub fn fake_model(id: &str) -> InstalledModel {
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

/// A chunk whose "audio" is `[id; 160]`, so the fake engine answers `c<id>`.
pub fn speech_chunk(id: usize) -> super::Chunk {
    super::Chunk { samples: vec![id as f32; 160], has_speech: true, start_ms: 0 }
}
