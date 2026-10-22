//! Streaming chunk planner: cuts live 16 kHz audio into chunks at natural pauses while the user
//! is still speaking, so chunks can be transcribed in the background.

use crate::audio::vad::{rms, trim_silence, FrameClassifier};
use crate::audio::{FRAME_LEN, FRAME_MS, SAMPLE_RATE};

/// A span of audio to transcribe.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub samples: Vec<f32>,
    pub has_speech: bool,
    /// Offset of the first sample from the start of the stream.
    pub start_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ChunkerConfig {
    /// A chunk is only cut at a pause once it is at least this long.
    pub min_secs: f32,
    /// Silence needed to count as a pause.
    pub pause_ms: u32,
    /// A chunk is force-cut when it reaches this length.
    pub max_secs: f32,
    /// Forced cuts land on the quietest frame within the last this-many seconds.
    pub search_back_secs: f32,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self { min_secs: 4.0, pause_ms: 300, max_secs: 15.0, search_back_secs: 3.0 }
    }
}

#[derive(Clone, Copy)]
struct FrameInfo {
    speech: bool,
    rms: f32,
}

/// Pure, incremental planner. Feed it audio of any size with [`push`](Self::push); it returns the
/// chunks completed so far. [`finish`](Self::finish) returns the trimmed tail.
pub struct ChunkPlanner {
    cfg: ChunkerConfig,
    classifier: FrameClassifier,
    /// Samples of the current (open) chunk, including whole frames only.
    buf: Vec<f32>,
    frames: Vec<FrameInfo>,
    /// Trailing non-speech frames of `frames`.
    silence_run: usize,
    chunk_has_speech: bool,
    /// Samples received that do not yet form a whole frame.
    carry: Vec<f32>,
    /// Samples already emitted in earlier chunks.
    emitted: u64,
    any_speech: bool,
}

fn ms_of(samples: u64) -> u64 {
    samples * 1000 / SAMPLE_RATE as u64
}

impl ChunkPlanner {
    pub fn new(cfg: ChunkerConfig) -> Self {
        Self {
            cfg,
            classifier: FrameClassifier::new(),
            buf: Vec::new(),
            frames: Vec::new(),
            silence_run: 0,
            chunk_has_speech: false,
            carry: Vec::new(),
            emitted: 0,
            any_speech: false,
        }
    }

    /// True once any frame so far was classified as speech.
    pub fn any_speech(&self) -> bool {
        self.any_speech
    }

    fn frames_for(secs: f32) -> usize {
        (secs * 1000.0 / FRAME_MS as f32).round() as usize
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Chunk> {
        let mut out = Vec::new();
        self.carry.extend_from_slice(samples);
        let whole = self.carry.len() / FRAME_LEN * FRAME_LEN;
        let rest = self.carry.split_off(whole);
        let data = std::mem::replace(&mut self.carry, rest);
        for frame in data.chunks_exact(FRAME_LEN) {
            self.add_frame(frame);
            if let Some(cut) = self.cut_point() {
                out.push(self.take_chunk(cut));
            }
        }
        out
    }

    fn add_frame(&mut self, frame: &[f32]) {
        let speech = self.classifier.is_speech(frame);
        self.buf.extend_from_slice(frame);
        self.frames.push(FrameInfo { speech, rms: rms(frame) });
        if speech {
            self.silence_run = 0;
            self.chunk_has_speech = true;
            self.any_speech = true;
        } else {
            self.silence_run += 1;
        }
    }

    /// Frame index at which to cut the open chunk, if it should be cut now.
    fn cut_point(&self) -> Option<usize> {
        let n = self.frames.len();
        let pause = Self::frames_for(self.cfg.pause_ms as f32 / 1000.0).max(1);
        if n >= Self::frames_for(self.cfg.min_secs) && self.silence_run >= pause {
            // The pause started at frame n - silence_run; cut in the middle of what we have seen.
            return Some(n - self.silence_run + self.silence_run / 2);
        }
        if n >= Self::frames_for(self.cfg.max_secs) {
            let from = n.saturating_sub(Self::frames_for(self.cfg.search_back_secs)).max(1);
            // Quietest frame; the latest one on ties, to keep chunks long.
            let quietest = (from..n)
                .min_by(|&a, &b| {
                    self.frames[a].rms.total_cmp(&self.frames[b].rms).then(b.cmp(&a))
                })
                .unwrap_or(n);
            return Some(quietest);
        }
        None
    }

    fn take_chunk(&mut self, cut_frame: usize) -> Chunk {
        let cut = cut_frame * FRAME_LEN;
        let rest = self.buf.split_off(cut);
        let samples = std::mem::replace(&mut self.buf, rest);
        let tail_frames = self.frames.split_off(cut_frame);
        let head_frames = std::mem::replace(&mut self.frames, tail_frames);
        let chunk = Chunk {
            has_speech: head_frames.iter().any(|f| f.speech),
            start_ms: ms_of(self.emitted),
            samples,
        };
        self.emitted += cut as u64;
        self.chunk_has_speech = self.frames.iter().any(|f| f.speech);
        self.silence_run = self.frames.iter().rev().take_while(|f| !f.speech).count();
        chunk
    }

    /// The remaining audio, trimmed of leading/trailing silence; `None` if it has no speech.
    pub fn finish(&mut self) -> Option<Chunk> {
        let mut all = std::mem::take(&mut self.buf);
        all.append(&mut self.carry);
        let start = self.emitted;
        self.emitted += all.len() as u64;
        self.frames.clear();
        self.silence_run = 0;
        self.chunk_has_speech = false;
        let trimmed = trim_silence(&all)?;
        // `trimmed` is a subslice of `all`: recover its offset.
        let offset = (trimmed.as_ptr() as usize - all.as_ptr() as usize) / std::mem::size_of::<f32>();
        Some(Chunk { samples: trimmed.to_vec(), has_speech: true, start_ms: ms_of(start + offset as u64) })
    }
}

/// Joins chunk texts with single spaces. A chunk that continues a sentence (the previous text did
/// not end in `.`, `?` or `!`) has its first letter lower-cased unless it starts an acronym.
pub fn join_chunks(texts: &[String]) -> String {
    let mut out = String::new();
    for t in texts.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if out.is_empty() {
            out.push_str(t);
            continue;
        }
        let ends_sentence = out.ends_with(['.', '?', '!']);
        let mut chars = t.chars();
        let first = chars.next().unwrap();
        out.push(' ');
        let continues = !ends_sentence
            && first.is_uppercase()
            && chars.next().is_some_and(|c| c.is_lowercase());
        if continues {
            out.extend(first.to_lowercase());
            out.push_str(&t[first.len_utf8()..]);
        } else {
            out.push_str(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{FRAME_LEN, SAMPLE_RATE};

    const SR: usize = SAMPLE_RATE as usize;

    /// Speech stand-in: a 300 Hz tone with a shallow 20 ms "breath" dip every 2 s, which keeps the
    /// adaptive noise floor low (a perfectly continuous tone would become the floor after 10 s).
    fn voiced(secs: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize / FRAME_LEN * FRAME_LEN;
        (0..n)
            .map(|i| {
                let s = 0.3 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / SR as f32).sin();
                if (i / FRAME_LEN) % 100 == 0 { s * 0.02 } else { s }
            })
            .collect()
    }

    fn silence(secs: f32) -> Vec<f32> {
        vec![0.0; (secs * SR as f32) as usize / FRAME_LEN * FRAME_LEN]
    }

    fn run(p: &mut ChunkPlanner, audio: &[f32]) -> Vec<Chunk> {
        audio.chunks(FRAME_LEN).flat_map(|f| p.push(f)).collect()
    }

    fn secs(n: usize) -> f32 {
        n as f32 / SR as f32
    }

    #[test]
    fn chunker_short_utterance_only_tail() {
        let mut p = ChunkPlanner::new(ChunkerConfig::default());
        assert!(run(&mut p, &voiced(3.0)).is_empty());
        let tail = p.finish().expect("tail");
        assert!(tail.has_speech);
        assert!(p.any_speech());
    }

    #[test]
    fn chunker_cuts_at_pause_after_min() {
        let mut p = ChunkPlanner::new(ChunkerConfig::default());
        let audio = [voiced(5.0), silence(0.4), voiced(3.0)].concat();
        let chunks = run(&mut p, &audio);
        assert_eq!(chunks.len(), 1);
        let c = &chunks[0];
        assert!(c.has_speech);
        assert_eq!(c.start_ms, 0);
        // cut lands inside the 5.0..5.4 s pause, at its middle for the 300 ms detection point
        let len = secs(c.samples.len());
        assert!((5.1..=5.3).contains(&len), "chunk is {len} s");
        let tail = p.finish().unwrap();
        assert!(tail.start_ms as f32 / 1000.0 >= len);
    }

    #[test]
    fn chunker_ignores_pause_before_min() {
        let mut p = ChunkPlanner::new(ChunkerConfig::default());
        let audio = [voiced(2.0), silence(0.4), voiced(2.0)].concat();
        assert!(run(&mut p, &audio).is_empty());
    }

    #[test]
    fn chunker_forces_cut_at_max_at_quietest_frame() {
        let mut p = ChunkPlanner::new(ChunkerConfig::default());
        let mut audio = voiced(20.0);
        let at = 675 * FRAME_LEN; // 13.5 s, deepest dip
        audio[at..at + FRAME_LEN].fill(0.0);
        let chunks = run(&mut p, &audio);
        let c = &chunks[0];
        assert!(secs(c.samples.len()) <= 15.0);
        let diff = (c.samples.len() as i64 - at as i64).abs();
        assert!(diff <= FRAME_LEN as i64, "cut at {} s", secs(c.samples.len()));
    }

    /// Deterministic stream of speech bursts and pauses, `total` seconds long.
    fn stream(total: f32) -> Vec<f32> {
        let mut x: u32 = 7;
        let mut rnd = move || {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut out = Vec::new();
        let mut first = true;
        while secs(out.len()) < total {
            out.extend(voiced(if first { 22.0 } else { 0.5 + rnd() * 11.0 }));
            first = false;
            out.extend(silence(0.06 + rnd() * 0.8));
        }
        out.extend(voiced(3.0));
        out
    }

    #[test]
    fn six_minute_stream_yields_ordered_chunks_le_15s() {
        let audio = stream(360.0);
        let mut p = ChunkPlanner::new(ChunkerConfig::default());
        let chunks = run(&mut p, &audio);
        let tail = p.finish().expect("tail");
        assert!(chunks.len() >= 24, "{} chunks", chunks.len());
        let mut pos = 0usize;
        let mut joined = Vec::new();
        for c in &chunks {
            assert!(secs(c.samples.len()) <= 15.0, "chunk of {} s", secs(c.samples.len()));
            assert_eq!(c.start_ms as usize, pos * 1000 / SR, "contiguous start");
            joined.extend_from_slice(&c.samples);
            pos += c.samples.len();
        }
        assert_eq!(joined, audio[..pos], "chunks are exact consecutive slices");
        assert!(secs(tail.samples.len()) <= 15.0);
        let ts = tail.start_ms as usize * SR / 1000;
        assert!(ts >= pos);
        assert_eq!(tail.samples, audio[ts..ts + tail.samples.len()]);
        // only silence is dropped (between last chunk and the tail, and after it)
        let dropped = (ts - pos) + (audio.len() - ts - tail.samples.len());
        assert!(secs(dropped) < 3.0, "dropped {} s", secs(dropped));
    }

    #[test]
    fn silent_chunk_has_no_speech() {
        let mut p = ChunkPlanner::new(ChunkerConfig::default());
        let chunks = run(&mut p, &silence(6.0));
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| !c.has_speech));
        assert!(p.finish().is_none());
        assert!(!p.any_speech());
    }

    #[test]
    fn join_chunks_lowercases_continuation() {
        let j = |v: &[&str]| join_chunks(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(j(&["So I went", "And then"]), "So I went and then");
        assert_eq!(j(&["Done.", "Next"]), "Done. Next");
        assert_eq!(j(&["Hi", "NASA rocks"]), "Hi NASA rocks");
        assert_eq!(j(&[" a ", "", "  ", "I know"]), "a I know");
        assert_eq!(j(&["Really?", "Yes", "so"]), "Really? Yes so");
        assert_eq!(j(&[]), "");
    }
}
