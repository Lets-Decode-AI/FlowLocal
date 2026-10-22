//! Energy-based voice activity detection on 20 ms frames of 16 kHz mono audio.

use std::collections::VecDeque;

use super::{FRAME_LEN, SAMPLE_RATE};

/// Floor on the speech threshold (RMS).
const MIN_THRESHOLD: f32 = 0.01;
/// Threshold = noise floor x this factor (when above `MIN_THRESHOLD`).
const FLOOR_MULTIPLIER: f32 = 3.0;
/// Initial noise-floor estimate before any audio has been seen.
const SEED_FLOOR: f32 = 0.003;
/// The noise floor is the minimum frame RMS over this many frames (10 s): it falls to
/// quieter frames immediately and rises only as old, quieter frames age out of the window,
/// so speech shorter than the window cannot lift it. The seed ages out the same way.
const FLOOR_WINDOW_FRAMES: u64 = 500;
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

/// Asymmetric noise-floor tracker (sliding-window minimum), shared by the online
/// classifier and the batch trimmer.
struct NoiseTracker {
    frame: u64,
    /// Monotonic deque of (frame index, rms), increasing in rms.
    mins: VecDeque<(u64, f32)>,
}

impl NoiseTracker {
    fn new() -> Self {
        Self { frame: 0, mins: VecDeque::from([(0, SEED_FLOOR)]) }
    }

    fn floor(&self) -> f32 {
        self.mins.front().map_or(SEED_FLOOR, |m| m.1)
    }

    fn observe(&mut self, level: f32) {
        self.frame += 1;
        while self.mins.back().is_some_and(|m| m.1 >= level) {
            self.mins.pop_back();
        }
        self.mins.push_back((self.frame, level));
        while self.mins.front().is_some_and(|m| m.0 + FLOOR_WINDOW_FRAMES <= self.frame) {
            self.mins.pop_front();
        }
    }
}

/// Adaptive speech/silence classifier for 20 ms frames.
pub struct FrameClassifier {
    tracker: NoiseTracker,
}

impl Default for FrameClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameClassifier {
    pub fn new() -> Self {
        Self { tracker: NoiseTracker::new() }
    }

    /// Current noise-floor estimate.
    pub fn noise_floor(&self) -> f32 {
        self.tracker.floor()
    }

    /// Classify one frame, then fold it into the noise-floor estimate.
    pub fn is_speech(&mut self, frame: &[f32]) -> bool {
        let level = rms(frame);
        let speech = level > threshold(self.tracker.floor());
        self.tracker.observe(level);
        speech
    }
}

/// Per-frame speech flags for a clip, using exactly the online classifier.
pub fn speech_flags(samples: &[f32]) -> Vec<bool> {
    let mut c = FrameClassifier::new();
    samples.chunks_exact(FRAME_LEN).map(|f| c.is_speech(f)).collect()
}

/// Trim leading/trailing non-speech (keeping 200 ms padding). `None` when no frame
/// is classified as speech.
pub fn trim_silence(samples: &[f32]) -> Option<&[f32]> {
    let flags = speech_flags(samples);
    let first = flags.iter().position(|&f| f)?;
    let last = flags.iter().rposition(|&f| f)?;
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

    /// Speech-like signal: 4 Hz amplitude-modulated 200 Hz tone with the given RMS.
    fn speechlike(secs: f32, rms_target: f32) -> Vec<f32> {
        (0..(16_000.0 * secs) as usize)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                let env = 1.0 + 0.3 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
                rms_target * 2f32.sqrt() * env * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
            })
            .collect()
    }

    fn longest_false_run(flags: &[bool]) -> usize {
        let (mut best, mut cur) = (0, 0);
        for &f in flags {
            cur = if f { 0 } else { cur + 1 };
            best = best.max(cur);
        }
        best
    }

    #[test]
    fn quiet_continuous_speech_no_false_pause() {
        let mut s = noise(0.2, 0.002 * 3f32.sqrt());
        s.extend(speechlike(8.0, 0.02));
        let flags = speech_flags(&s);
        let after = &flags[25..]; // skip first 0.5 s
        assert!(longest_false_run(after) * 20 < 300, "false pause {} ms", longest_false_run(after) * 20);
    }

    #[test]
    fn loud_room_pauses_detected() {
        let amp = 0.05 * 3f32.sqrt(); // uniform noise: rms = amp / sqrt(3)
        let mut s = noise(11.0, amp); // let the floor settle
        let mut gaps = Vec::new();
        for _ in 0..4 {
            let burst: Vec<f32> = speechlike(1.0, 0.3);
            let n = noise(1.0, amp);
            s.extend(burst.iter().zip(&n).map(|(a, b)| a + b));
            gaps.push(s.len() / FRAME_LEN + 3); // skip transition frames
            s.extend(noise(0.5, amp));
        }
        let flags = speech_flags(&s);
        for g in gaps {
            assert!(flags[g..g + 20].iter().all(|f| !f), "gap at frame {g} classified as speech");
        }
        assert!(flags[11 * 50 + 10..11 * 50 + 40].iter().all(|f| *f), "burst not speech");
    }

    #[test]
    fn classifier_matches_trim_frame_by_frame() {
        let mut s = noise(1.0, 0.004);
        s.extend(speechlike(1.5, 0.1));
        s.extend(noise(1.0, 0.004));
        let mut c = FrameClassifier::new();
        let flags: Vec<bool> = s.chunks_exact(FRAME_LEN).map(|f| c.is_speech(f)).collect();
        assert_eq!(flags, speech_flags(&s));
        let first = flags.iter().position(|&f| f).unwrap();
        let last = flags.iter().rposition(|&f| f).unwrap();
        let t = trim_silence(&s).unwrap();
        let start = (first * FRAME_LEN).saturating_sub(3200);
        let end = ((last + 1) * FRAME_LEN + 3200).min(s.len());
        assert_eq!(t.len(), end - start);
    }
}
