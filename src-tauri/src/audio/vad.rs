//! Energy-based voice activity detection on 20 ms frames of 16 kHz mono audio.

use std::collections::VecDeque;

use super::{FRAME_LEN, SAMPLE_RATE};

/// Floor on the speech threshold (RMS).
const MIN_THRESHOLD: f32 = 0.01;
/// Threshold = noise floor x this factor (when above `MIN_THRESHOLD`).
const FLOOR_MULTIPLIER: f32 = 3.0;
/// Initial noise-floor estimate before any audio has been seen.
const SEED_FLOOR: f32 = 0.003;
/// The noise floor never exceeds this, so sustained speech cannot raise the threshold
/// past quiet speech.
const MAX_FLOOR: f32 = 0.01;
/// Rolling window of recent frame RMS values used for the online floor (~2 s).
const WINDOW_FRAMES: usize = 100;
/// Seed entries (~0.5 s) that age out as real frames arrive.
const SEED_FRAMES: usize = 25;
/// Padding kept around detected speech by [`trim_silence`] (200 ms).
const PAD_SAMPLES: usize = SAMPLE_RATE as usize / 5;

/// RMS of a frame (0.0 for an empty frame).
pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

fn threshold(noise_floor: f32) -> f32 {
    (noise_floor * FLOOR_MULTIPLIER).max(MIN_THRESHOLD)
}

/// 10th percentile of `values` (sorts in place).
fn percentile_10(values: &mut [f32]) -> f32 {
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 10]
}

/// Adaptive speech/silence classifier for 20 ms frames. The noise floor is the 10th
/// percentile of the last ~2 s of frame RMS values.
pub struct FrameClassifier {
    window: VecDeque<f32>,
}

impl Default for FrameClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameClassifier {
    pub fn new() -> Self {
        Self { window: std::iter::repeat_n(SEED_FLOOR, SEED_FRAMES).collect() }
    }

    /// Current noise-floor estimate.
    pub fn noise_floor(&self) -> f32 {
        let mut v: Vec<f32> = self.window.iter().copied().collect();
        percentile_10(&mut v).min(MAX_FLOOR)
    }

    /// Classify one frame, then fold it into the noise-floor estimate.
    pub fn is_speech(&mut self, frame: &[f32]) -> bool {
        let level = rms(frame);
        let speech = level > threshold(self.noise_floor());
        if self.window.len() == WINDOW_FRAMES {
            self.window.pop_front();
        }
        self.window.push_back(level.min(MAX_FLOOR));
        speech
    }
}

/// Trim leading/trailing non-speech (keeping 200 ms padding). `None` when no frame
/// exceeds the adaptive threshold.
pub fn trim_silence(samples: &[f32]) -> Option<&[f32]> {
    let levels: Vec<f32> = samples.chunks_exact(FRAME_LEN).map(rms).collect();
    if levels.is_empty() {
        return None;
    }
    let mut sorted = levels.clone();
    let floor = percentile_10(&mut sorted).min(MAX_FLOOR);
    let thr = threshold(floor);
    let first = levels.iter().position(|&l| l > thr)?;
    let last = levels.iter().rposition(|&l| l > thr)?;
    let start = (first * FRAME_LEN).saturating_sub(PAD_SAMPLES);
    let end = ((last + 1) * FRAME_LEN + PAD_SAMPLES).min(samples.len());
    Some(&samples[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32) -> Vec<f32> {
        (0..(16_000.0 * secs) as usize)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / 16_000.0).sin())
            .collect()
    }

    /// Deterministic pseudo-noise with the given peak amplitude.
    fn noise(secs: f32, amp: f32) -> Vec<f32> {
        let mut x: u32 = 12345;
        (0..(16_000.0 * secs) as usize)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn trim_removes_leading_trailing_silence() {
        let mut s = vec![0.0; 8_000];
        s.extend(tone(1.0));
        s.extend(vec![0.0; 8_000]);
        let t = trim_silence(&s).expect("speech");
        let secs = t.len() as f32 / 16_000.0;
        assert!((secs - 1.4).abs() <= 0.05, "trimmed {secs}");
    }

    #[test]
    fn trim_returns_none_for_silence_and_low_noise() {
        assert!(trim_silence(&vec![0.0; 16_000]).is_none());
        assert!(trim_silence(&noise(1.0, 0.005)).is_none());
    }

    #[test]
    fn frame_classifier_separates_tone_from_noise() {
        let mut c = FrameClassifier::new();
        let n = noise(1.0, 0.005);
        let t = tone(0.5);
        assert!(n.chunks_exact(FRAME_LEN).all(|f| !c.is_speech(f)));
        assert!(t.chunks_exact(FRAME_LEN).all(|f| c.is_speech(f)));
        assert!(n.chunks_exact(FRAME_LEN).skip(5).all(|f| !c.is_speech(f)));
    }

    #[test]
    fn classifier_and_trim_agree_on_tone_vs_silence() {
        let mut s = vec![0.0; 16_000];
        s.extend(tone(1.0));
        let mut c = FrameClassifier::new();
        let online: usize = s.chunks_exact(FRAME_LEN).filter(|f| c.is_speech(f)).count();
        assert!(online >= 45 && online <= 55, "online speech frames {online}");
        assert!(trim_silence(&s).is_some());
    }
}
