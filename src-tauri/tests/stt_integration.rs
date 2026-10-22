//! Real-model STT tests. Ignored unless run with `--ignored`; they need a model directory:
//!
//!   scripts/fetch_test_model.sh
//!   FLOWLOCAL_MODEL_DIR="$HOME/Library/Caches/FlowLocal Test é/parakeet-v2" \
//!     cargo test --release --test stt_integration -- --ignored --nocapture
//!
//! The directory must hold the model files plus a `manifest.json` (a catalog entry).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use flowlocal_lib::audio::{FRAME_LEN, SAMPLE_RATE};
use flowlocal_lib::models::InstalledModel;
use flowlocal_lib::models::catalog::CatalogEntry;
use flowlocal_lib::stt::{Chunk, ChunkPlanner, ChunkerConfig, SherpaEngine, SttHandle, SttWorker};

fn model() -> Option<InstalledModel> {
    let dir = PathBuf::from(std::env::var_os("FLOWLOCAL_MODEL_DIR")?);
    let entry: CatalogEntry =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("manifest.json"))
            .expect("manifest.json is a catalog entry");
    Some(InstalledModel { entry, dir })
}

fn fixtures() -> Vec<(String, Vec<f32>, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let expected: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();
    expected
        .into_iter()
        .map(|(name, text)| {
            let mut r = hound::WavReader::open(dir.join(&name)).unwrap();
            assert_eq!(r.spec().sample_rate, SAMPLE_RATE);
            let samples = r
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / 32768.0)
                .collect();
            (name, samples, text)
        })
        .collect()
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word error rate: word-level Levenshtein distance / reference length.
fn wer(reference: &str, hypothesis: &str) -> f32 {
    let (r, h) = (words(reference), words(hypothesis));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut cur = vec![i];
        for j in 1..=h.len() {
            let sub = prev[j - 1] + usize::from(r[i - 1] != h[j - 1]);
            cur.push(sub.min(prev[j] + 1).min(cur[j - 1] + 1));
        }
        prev = cur;
    }
    prev[h.len()] as f32 / r.len() as f32
}

async fn transcribe_whole(
    h: &SttHandle,
    m: &InstalledModel,
    samples: &[f32],
) -> (String, Duration) {
    let s = h.session(m.clone());
    let t = Instant::now();
    let text = s
        .finish(Some(Chunk {
            samples: samples.to_vec(),
            has_speech: true,
            start_ms: 0,
        }))
        .await
        .expect("transcribe");
    (text, t.elapsed())
}

#[test]
fn wer_helper_sanity() {
    assert_eq!(wer("the cat sat", "The cat sat."), 0.0);
    assert!((wer("the cat sat", "the dog sat down") - 2.0 / 3.0).abs() < 1e-6);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs FLOWLOCAL_MODEL_DIR (see scripts/fetch_test_model.sh)"]
async fn parakeet_wer_below_threshold() {
    let Some(m) = model() else {
        return eprintln!("FLOWLOCAL_MODEL_DIR not set; skipping");
    };
    let h = SttWorker::spawn(Box::new(SherpaEngine::new()), None);
    let t = Instant::now();
    h.preload(m.clone()).await.expect("load model");
    eprintln!("model load: {:?} (dir {})", t.elapsed(), m.dir.display());
    let files = fixtures();
    transcribe_whole(&h, &m, &files[0].1).await; // warm-up

    let mut worst: f32 = 0.0;
    let mut latency_5s = None;
    for (name, samples, expected) in &files {
        let (text, took) = transcribe_whole(&h, &m, samples).await;
        let w = wer(expected, &text);
        eprintln!(
            "{name}: {:.1}s audio, {took:?}, WER {w:.3}\n  -> {text}",
            samples.len() as f32 / 16000.0
        );
        worst = worst.max(w);
        assert!(w < 0.08, "{name}: WER {w}");
        if name == "s0.wav" {
            latency_5s = Some(took);
        }
    }
    let took = latency_5s.unwrap();
    eprintln!("5 s clip latency: {took:?}; worst WER {worst:.3}");
    if !cfg!(debug_assertions) && cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert!(took < Duration::from_millis(500), "5 s clip took {took:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs FLOWLOCAL_MODEL_DIR (see scripts/fetch_test_model.sh)"]
async fn streaming_60s_finish_under_1s() {
    let Some(m) = model() else {
        return eprintln!("FLOWLOCAL_MODEL_DIR not set; skipping");
    };
    let h = SttWorker::spawn(Box::new(SherpaEngine::new()), None);
    h.preload(m.clone()).await.expect("load model");
    let files = fixtures();
    transcribe_whole(&h, &m, &files[0].1).await; // warm-up

    // ~60 s: every fixture, then the first one again, 600 ms apart (none after the last).
    let gap = vec![0.0f32; SAMPLE_RATE as usize * 6 / 10];
    let mut audio = Vec::new();
    let mut expected = Vec::new();
    for (_, samples, text) in files.iter().chain(files.iter().take(1)) {
        if !audio.is_empty() {
            audio.extend_from_slice(&gap);
        }
        audio.extend_from_slice(samples);
        expected.push(text.clone());
    }
    let total_secs = audio.len() as f32 / SAMPLE_RATE as f32;

    let mut planner = ChunkPlanner::new(ChunkerConfig::default());
    let session = h.session(m.clone());
    let t0 = Instant::now();
    let mut chunks = 0;
    for (i, frame) in audio.chunks(FRAME_LEN).enumerate() {
        tokio::time::sleep_until((t0 + Duration::from_millis(20 * (i as u64 + 1))).into()).await;
        for c in planner.push(frame) {
            chunks += 1;
            session.feed(c);
        }
    }
    let key_up = Instant::now();
    let tail = planner.finish();
    let tail_secs = tail
        .as_ref()
        .map_or(0.0, |t| t.samples.len() as f32 / 16000.0);
    let text = session.finish(tail).await.expect("finish");
    let latency = key_up.elapsed();

    let w = wer(&expected.join(" "), &text);
    eprintln!(
        "streamed {total_secs:.1}s in {chunks}+1 chunks (tail {tail_secs:.1}s): finish latency {latency:?}, WER {w:.3}\n  -> {text}"
    );
    assert!(latency < Duration::from_secs(1), "finish took {latency:?}");
    assert!(w < 0.10, "streamed WER {w}");
}
