//! sherpa-onnx backed [`SttEngine`], dispatching on the model's [`EngineKind`].

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use sherpa_rs::moonshine::{MoonshineConfig, MoonshineRecognizer};
use sherpa_rs::sense_voice::{SenseVoiceConfig, SenseVoiceRecognizer};
use sherpa_rs::transducer::{TransducerConfig, TransducerRecognizer};
use sherpa_rs::whisper::{WhisperConfig, WhisperRecognizer};

use super::SttEngine;
use crate::audio::SAMPLE_RATE;
use crate::models::catalog::EngineKind;
use crate::models::InstalledModel;

enum Recognizer {
    Transducer(TransducerRecognizer),
    Whisper(WhisperRecognizer),
    Moonshine(MoonshineRecognizer),
    SenseVoice(SenseVoiceRecognizer),
}

#[derive(Default)]
pub struct SherpaEngine {
    recognizer: Option<Recognizer>,
    id: Option<String>,
}

impl SherpaEngine {
    pub fn new() -> Self {
        Self::default()
    }
}

/// `min(cores, 4)`.
fn num_threads() -> i32 {
    std::thread::available_parallelism().map_or(1, |n| n.get()).min(4) as i32
}

fn path(p: PathBuf) -> Result<String> {
    p.into_os_string().into_string().map_err(|p| anyhow!("model path is not valid UTF-8: {p:?}"))
}

/// Execution providers to try, in order. CPU by default: measured on Apple Silicon, CoreML was
/// ~1.5x slower than CPU for Parakeet (5 s clip: ~690 ms vs ~390 ms). `FLOWLOCAL_STT_PROVIDER=coreml`
/// opts in to CoreML (falling back to CPU if it fails to load).
fn transducer_providers() -> Vec<&'static str> {
    match std::env::var("FLOWLOCAL_STT_PROVIDER").as_deref() {
        Ok("coreml") => vec!["coreml", "cpu"],
        _ => vec!["cpu"],
    }
}

fn load_transducer(m: &InstalledModel) -> Result<TransducerRecognizer> {
    let mut last = anyhow!("no provider");
    for provider in transducer_providers() {
        let cfg = TransducerConfig {
            encoder: path(m.file("encoder"))?,
            decoder: path(m.file("decoder"))?,
            joiner: path(m.file("joiner"))?,
            tokens: path(m.file("tokens.txt"))?,
            model_type: "nemo_transducer".into(),
            num_threads: num_threads(),
            sample_rate: SAMPLE_RATE as i32,
            feature_dim: 80,
            provider: Some(provider.into()),
            ..Default::default()
        };
        match TransducerRecognizer::new(cfg) {
            Ok(r) => {
                tracing::info!("transducer loaded with provider {provider}");
                return Ok(r);
            }
            Err(e) => {
                tracing::warn!("transducer provider {provider} failed: {e}");
                last = anyhow!("{e}");
            }
        }
    }
    Err(last)
}

impl SttEngine for SherpaEngine {
    fn load(&mut self, m: &InstalledModel) -> Result<()> {
        self.unload();
        let threads = Some(num_threads());
        let provider = Some("cpu".to_string());
        let r = match m.entry.engine {
            EngineKind::Transducer => Recognizer::Transducer(load_transducer(m)?),
            EngineKind::Whisper => Recognizer::Whisper(
                WhisperRecognizer::new(WhisperConfig {
                    encoder: path(m.file("encoder"))?,
                    decoder: path(m.file("decoder"))?,
                    tokens: path(m.file("tokens"))?,
                    language: m.entry.languages.first().cloned().unwrap_or_else(|| "en".into()),
                    num_threads: threads,
                    provider,
                    ..Default::default()
                })
                .map_err(|e| anyhow!("{e}"))?,
            ),
            EngineKind::Moonshine => Recognizer::Moonshine(
                MoonshineRecognizer::new(MoonshineConfig {
                    preprocessor: path(m.file("preprocess.onnx"))?,
                    encoder: path(m.file("encode.int8.onnx"))?,
                    uncached_decoder: path(m.file("uncached_decode.int8.onnx"))?,
                    cached_decoder: path(m.file("cached_decode.int8.onnx"))?,
                    tokens: path(m.file("tokens.txt"))?,
                    num_threads: threads,
                    provider,
                    ..Default::default()
                })
                .map_err(|e| anyhow!("{e}"))?,
            ),
            EngineKind::SenseVoice => Recognizer::SenseVoice(
                SenseVoiceRecognizer::new(SenseVoiceConfig {
                    model: path(m.file("model.int8.onnx"))?,
                    tokens: path(m.file("tokens.txt"))?,
                    language: "auto".into(),
                    use_itn: true,
                    num_threads: threads,
                    provider,
                    ..Default::default()
                })
                .map_err(|e| anyhow!("{e}"))?,
            ),
        };
        self.recognizer = Some(r);
        self.id = Some(m.entry.id.clone());
        Ok(())
    }

    fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        let sr = SAMPLE_RATE;
        let text = match self.recognizer.as_mut() {
            None => bail!("no model loaded"),
            Some(Recognizer::Transducer(r)) => r.transcribe(sr, samples),
            Some(Recognizer::Whisper(r)) => r.transcribe(sr, samples).text,
            Some(Recognizer::Moonshine(r)) => r.transcribe(sr, samples).text,
            Some(Recognizer::SenseVoice(r)) => r.transcribe(sr, samples).text,
        };
        Ok(text.trim().to_string())
    }

    fn unload(&mut self) {
        self.recognizer = None;
        self.id = None;
    }

    fn loaded_id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}


#[cfg(test)]
mod tests {
    /// Proves sherpa-rs (and its native sherpa-onnx libs) link into the test binary.
    #[test]
    fn sherpa_links() {
        let cfg = sherpa_rs::transducer::TransducerConfig::default();
        assert!(cfg.encoder.is_empty());
        // Calls into native sherpa-onnx, so the linker must resolve it. With no model
        // files configured it is expected to fail gracefully rather than succeed.
        let r = sherpa_rs::transducer::TransducerRecognizer::new(cfg);
        assert!(r.is_err());
    }
}
