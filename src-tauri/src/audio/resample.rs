//! Downmix + resample to 16 kHz mono, batch and streaming.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

use super::SAMPLE_RATE;

/// Incremental downmix + resample to 16 kHz mono. Feeding a signal in any number of
/// pieces then calling [`flush`](Self::flush) yields the same samples as one batch call.
pub struct StreamResampler {
    channels: usize,
    rate: u32,
    rs: Option<Fft<f32>>,
    /// Incomplete trailing interleaved frame from the previous call.
    carry: Vec<f32>,
    /// Mono input not yet consumed by the fixed-size resampler.
    pending: Vec<f32>,
    out_buf: Vec<f32>,
    /// Leading output frames still to drop (resampler delay).
    skip: usize,
    in_total: u64,
    out_total: u64,
}

impl StreamResampler {
    pub fn new(channels: u16, rate: u32) -> Self {
        let channels = channels.max(1) as usize;
        let rs = if rate == SAMPLE_RATE || rate == 0 {
            None
        } else {
            let chunk = (rate as usize / 50).max(64);
            Fft::<f32>::new(rate as usize, SAMPLE_RATE as usize, chunk, 1, FixedSync::Input).ok()
        };
        let (skip, out_buf) = match &rs {
            Some(r) => (r.output_delay(), vec![0.0; r.output_frames_max()]),
            None => (0, Vec::new()),
        };
        Self {
            channels,
            rate,
            rs,
            carry: Vec::new(),
            pending: Vec::new(),
            out_buf,
            skip,
            in_total: 0,
            out_total: 0,
        }
    }

    fn downmix(&mut self, input: &[f32]) -> Vec<f32> {
        if self.channels == 1 {
            return input.to_vec();
        }
        self.carry.extend_from_slice(input);
        let whole = self.carry.len() / self.channels * self.channels;
        let mono = self.carry[..whole]
            .chunks_exact(self.channels)
            .map(|f| f.iter().sum::<f32>() / self.channels as f32)
            .collect();
        self.carry.drain(..whole);
        mono
    }

    /// Run one resampler chunk; `partial` = number of valid frames when padding.
    fn run_chunk(&mut self, partial: Option<usize>, out: &mut Vec<f32>) {
        let Some(rs) = self.rs.as_mut() else { return };
        let need = rs.input_frames_next();
        let mut chunk = vec![0.0f32; need];
        let take = partial.unwrap_or(need).min(self.pending.len());
        chunk[..take].copy_from_slice(&self.pending[..take]);
        let consumed = partial.map_or(need, |_| take);
        self.pending.drain(..consumed);
        let input = InterleavedSlice::new(&chunk, 1, need).expect("input adapter");
        let max = self.out_buf.len();
        let mut output = InterleavedSlice::new_mut(&mut self.out_buf, 1, max).expect("output adapter");
        let indexing = partial.map(|n| Indexing::new().partial_len(n));
        let Ok((_, n_out)) = rs.process_into_buffer(&input, &mut output, indexing.as_ref()) else {
            return;
        };
        let start = self.skip.min(n_out);
        self.skip -= start;
        out.extend_from_slice(&self.out_buf[start..n_out]);
    }

    /// Feed interleaved input; returns whatever 16 kHz mono output is ready.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mono = self.downmix(input);
        if self.rs.is_none() {
            return mono;
        }
        self.in_total += mono.len() as u64;
        self.pending.extend_from_slice(&mono);
        let mut out = Vec::new();
        while self.rs.as_ref().is_some_and(|r| self.pending.len() >= r.input_frames_next()) {
            self.run_chunk(None, &mut out);
        }
        self.out_total += out.len() as u64;
        out
    }

    /// Emit the remaining tail (including the resampler delay) and reset for reuse.
    pub fn flush(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        if self.rs.is_some() {
            let expected =
                (self.in_total as f64 * SAMPLE_RATE as f64 / self.rate as f64).round() as u64;
            if !self.pending.is_empty() {
                let n = self.pending.len();
                self.run_chunk(Some(n), &mut out);
            }
            let mut guard = 0;
            while self.out_total + (out.len() as u64) < expected && guard < 8 {
                self.run_chunk(Some(0), &mut out);
                guard += 1;
            }
            let room = expected.saturating_sub(self.out_total) as usize;
            out.truncate(room);
        }
        self.reset();
        out
    }

    fn reset(&mut self) {
        self.carry.clear();
        self.pending.clear();
        self.in_total = 0;
        self.out_total = 0;
        if let Some(r) = self.rs.as_mut() {
            r.reset();
            self.skip = r.output_delay();
        }
    }
}

/// Downmix (channel average) and resample a whole buffer to 16 kHz mono.
pub fn to_mono_16k(input: &[f32], channels: u16, rate: u32) -> Vec<f32> {
    let mut r = StreamResampler::new(channels, rate);
    let mut out = r.process(input);
    out.extend(r.flush());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn tone(freq: f32, rate: u32, secs: f32, channels: u16) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        let mut v = Vec::with_capacity(n * channels as usize);
        for i in 0..n {
            let s = 0.5 * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin();
            for _ in 0..channels {
                v.push(s);
            }
        }
        v
    }

    #[test]
    fn resample_48k_stereo_to_16k_mono_length() {
        let input = tone(440.0, 48_000, 1.0, 2);
        let out = to_mono_16k(&input, 2, 48_000);
        assert!((out.len() as i64 - 16_000).abs() <= 32, "len {}", out.len());
    }

    #[test]
    fn resample_preserves_440hz_tone() {
        let input = tone(440.0, 48_000, 1.0, 1);
        let out = to_mono_16k(&input, 1, 48_000);
        let crossings = out.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let freq = crossings as f32 / (out.len() as f32 / 16_000.0);
        assert!((freq - 440.0).abs() <= 5.0, "freq {freq}");
    }

    #[test]
    fn passthrough_when_16k_mono() {
        let input = tone(300.0, 16_000, 0.1, 1);
        assert_eq!(to_mono_16k(&input, 1, 16_000), input);
    }

    #[test]
    fn stream_resampler_matches_batch() {
        let input = tone(440.0, 48_000, 1.0, 1);
        let batch = to_mono_16k(&input, 1, 48_000);
        let mut r = StreamResampler::new(1, 48_000);
        let mut out = Vec::new();
        let cuts = [0usize, 1000, 5000, 5001, 20_000, 31_337, 40_000, input.len()];
        for w in cuts.windows(2) {
            out.extend(r.process(&input[w[0]..w[1]]));
        }
        out.extend(r.flush());
        assert_eq!(out.len(), batch.len());
        let mse: f32 =
            out.iter().zip(&batch).map(|(a, b)| (a - b) * (a - b)).sum::<f32>() / out.len() as f32;
        assert!(mse.sqrt() < 1e-3, "rms diff {}", mse.sqrt());
    }
}
