//! Microphone capture: cpal input stream -> worker thread (downmix, resample to 16 kHz
//! mono, 20 ms framing, levels, pre-roll) -> consumer channel.

pub mod resample;
pub mod vad;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use crossbeam_channel::{bounded, select, Receiver};

use resample::StreamResampler;

pub const SAMPLE_RATE: u32 = 16_000;
pub const FRAME_MS: u32 = 20;
/// Samples per 20 ms frame at 16 kHz (320).
pub const FRAME_LEN: usize = (SAMPLE_RATE * FRAME_MS / 1000) as usize;

/// Audio kept from before `start()` while the stream is warm (300 ms).
const PREROLL_SAMPLES: usize = (SAMPLE_RATE as usize) * 3 / 10;
/// The stream stays open this long after the last recording stops.
const WARM_FOR: Duration = Duration::from_secs(60);
/// One level event per ~33 ms of audio.
const LEVEL_SAMPLES: usize = (SAMPLE_RATE as usize) * 33 / 1000;
const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RecorderError {
    #[error("no input device available")]
    NoDevice,
    #[error("microphone permission denied")]
    PermissionDenied,
    #[error("input device was lost")]
    DeviceLost,
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingStats {
    /// Duration of audio delivered to the frame consumer, including pre-roll.
    pub duration_ms: u64,
}

/// Map a cpal error to a [`RecorderError`].
pub fn map_stream_error(e: &cpal::Error) -> RecorderError {
    use cpal::ErrorKind as K;
    match e.kind() {
        K::DeviceNotAvailable | K::DeviceChanged | K::StreamInvalidated => RecorderError::DeviceLost,
        K::PermissionDenied => RecorderError::PermissionDenied,
        _ => RecorderError::Other(e.to_string()),
    }
}

/// Errors reported after the stream started that mean it can no longer deliver audio.
fn is_fatal(e: &cpal::Error) -> bool {
    matches!(map_stream_error(e), RecorderError::DeviceLost | RecorderError::PermissionDenied)
}

/// Names of the available input devices.
pub fn list_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|it| {
            it.filter_map(|d| d.description().ok().map(|d| d.name().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn find_device(name: Option<&str>) -> Result<cpal::Device, RecorderError> {
    let host = cpal::default_host();
    match name {
        None => host.default_input_device().ok_or(RecorderError::NoDevice),
        Some(n) => host
            .input_devices()
            .map_err(|e| map_stream_error(&e))?
            .find(|d| d.description().is_ok_and(|d| d.name() == n))
            .ok_or(RecorderError::NoDevice),
    }
}

enum Cmd {
    Start { frame_tx: Sender<Vec<f32>>, ack: Sender<()> },
    Stop { reply: Sender<Result<RecordingStats, RecorderError>> },
    Cancel,
    Shutdown,
}

struct Handle {
    cmd_tx: crossbeam_channel::Sender<Cmd>,
    closed: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Handle {
    fn is_open(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Cmd::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub struct Recorder {
    device_name: Option<String>,
    level_tx: SyncSender<f32>,
    inner: Mutex<Option<Handle>>,
}

impl Recorder {
    /// Validates the device selection; the stream itself is opened by `start()`.
    pub fn new(device_name: Option<String>, level_tx: SyncSender<f32>) -> Result<Recorder, RecorderError> {
        find_device(device_name.as_deref())?;
        Ok(Recorder { device_name, level_tx, inner: Mutex::new(None) })
    }

    fn open(&self) -> Result<Handle, RecorderError> {
        let (cmd_tx, cmd_rx) = bounded(16);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let ctx = WorkerCtx {
            device_name: self.device_name.clone(),
            level_tx: self.level_tx.clone(),
            cmd_rx,
            closed: closed.clone(),
        };
        let join = std::thread::Builder::new()
            .name("flowlocal-audio".into())
            .spawn(move || run_worker(ctx, ready_tx))
            .map_err(|e| RecorderError::Other(e.to_string()))?;
        let handle = Handle { cmd_tx, closed, join: Some(join) };
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(RecorderError::Other("timed out opening input stream".into())),
        }
    }

    fn try_start(h: &Handle, frame_tx: &Sender<Vec<f32>>) -> bool {
        if !h.is_open() {
            return false;
        }
        let (ack, ack_rx) = std::sync::mpsc::channel();
        h.cmd_tx.send(Cmd::Start { frame_tx: frame_tx.clone(), ack }).is_ok()
            && ack_rx.recv_timeout(Duration::from_secs(1)).is_ok()
    }

    /// Begin streaming 16 kHz mono ~20 ms frames to `frame_tx` (pre-roll first when warm).
    pub fn start(&self, frame_tx: Sender<Vec<f32>>) -> Result<(), RecorderError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.as_ref().is_some_and(|h| Self::try_start(h, &frame_tx)) {
            return Ok(());
        }
        *inner = None; // stream closed or lost: reopen cold
        let handle = self.open()?;
        if !Self::try_start(&handle, &frame_tx) {
            return Err(RecorderError::Other("audio worker did not start".into()));
        }
        *inner = Some(handle);
        Ok(())
    }

    /// Flush remaining audio, drop the frame sender, and report what was delivered.
    pub fn stop(&self) -> Result<RecordingStats, RecorderError> {
        let mut inner = self.inner.lock().unwrap();
        let Some(h) = inner.as_ref() else {
            return Err(RecorderError::Other("not recording".into()));
        };
        let (reply, reply_rx) = std::sync::mpsc::channel();
        let res = if h.cmd_tx.send(Cmd::Stop { reply }).is_ok() {
            reply_rx
                .recv_timeout(Duration::from_secs(3))
                .unwrap_or(Err(RecorderError::DeviceLost))
        } else {
            Err(RecorderError::DeviceLost)
        };
        if res.is_err() {
            *inner = None;
        }
        res
    }

    /// Stop without flushing; the consumer sees the channel close.
    pub fn cancel(&self) {
        if let Some(h) = self.inner.lock().unwrap().as_ref() {
            let _ = h.cmd_tx.send(Cmd::Cancel);
        }
    }
}

struct WorkerCtx {
    device_name: Option<String>,
    level_tx: SyncSender<f32>,
    cmd_rx: Receiver<Cmd>,
    closed: Arc<AtomicBool>,
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    data_tx: crossbeam_channel::Sender<Vec<f32>>,
    pool_rx: Receiver<Vec<f32>>,
    lost: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream::<T, _, _>(
        config,
        // Real-time callback: convert into a recycled buffer and hand off; never block.
        move |data: &[T], _| {
            let mut buf = pool_rx.try_recv().unwrap_or_default();
            buf.clear();
            buf.extend(data.iter().map(|&s| f32::from_sample_(s)));
            let _ = data_tx.try_send(buf);
        },
        move |e| {
            if is_fatal(&e) {
                lost.store(true, Ordering::Release);
            }
        },
        None,
    )
}

fn open_stream(
    device_name: Option<&str>,
    data_tx: crossbeam_channel::Sender<Vec<f32>>,
    pool_rx: Receiver<Vec<f32>>,
    lost: Arc<AtomicBool>,
) -> Result<(cpal::Stream, u16, u32), RecorderError> {
    let device = find_device(device_name)?;
    let supported = device.default_input_config().map_err(|e| map_stream_error(&e))?;
    let (channels, rate, format) = (supported.channels(), supported.sample_rate(), supported.sample_format());
    let config: cpal::StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => build_stream::<f32>(&device, config, data_tx, pool_rx, lost),
        SampleFormat::I16 => build_stream::<i16>(&device, config, data_tx, pool_rx, lost),
        SampleFormat::U16 => build_stream::<u16>(&device, config, data_tx, pool_rx, lost),
        SampleFormat::I32 => build_stream::<i32>(&device, config, data_tx, pool_rx, lost),
        SampleFormat::I8 => build_stream::<i8>(&device, config, data_tx, pool_rx, lost),
        SampleFormat::U8 => build_stream::<u8>(&device, config, data_tx, pool_rx, lost),
        SampleFormat::F64 => build_stream::<f64>(&device, config, data_tx, pool_rx, lost),
        other => return Err(RecorderError::Other(format!("unsupported sample format {other}"))),
    }
    .map_err(|e| map_stream_error(&e))?;
    stream.play().map_err(|e| map_stream_error(&e))?;
    Ok((stream, channels, rate))
}

/// Everything the worker owns while the stream is open.
struct Pipeline {
    resampler: StreamResampler,
    ring: VecDeque<f32>,
    framer: Vec<f32>,
    session: Option<Sender<Vec<f32>>>,
    emitted: u64,
    level_sum: f32,
    level_n: usize,
    level_tx: SyncSender<f32>,
}

impl Pipeline {
    fn push(&mut self, samples: &[f32]) {
        self.ring.extend(samples.iter().copied());
        let excess = self.ring.len().saturating_sub(PREROLL_SAMPLES);
        self.ring.drain(..excess);
        if self.session.is_none() {
            return;
        }
        for &s in samples {
            self.level_sum += s * s;
            self.level_n += 1;
            if self.level_n == LEVEL_SAMPLES {
                let _ = self.level_tx.try_send((self.level_sum / LEVEL_SAMPLES as f32).sqrt());
                self.level_sum = 0.0;
                self.level_n = 0;
            }
        }
        self.framer.extend_from_slice(samples);
        self.emit_frames();
    }

    fn emit_frames(&mut self) {
        let Some(tx) = self.session.as_ref() else { return };
        let whole = self.framer.len() / FRAME_LEN * FRAME_LEN;
        for frame in self.framer[..whole].chunks_exact(FRAME_LEN) {
            if tx.send(frame.to_vec()).is_err() {
                break; // consumer gone
            }
        }
        self.emitted += whole as u64;
        self.framer.drain(..whole);
    }

    fn start(&mut self, tx: Sender<Vec<f32>>, warm: bool) {
        self.framer.clear();
        self.emitted = 0;
        self.level_sum = 0.0;
        self.level_n = 0;
        self.session = Some(tx);
        if warm {
            self.framer.extend(self.ring.iter().copied());
            self.emit_frames();
        }
    }

    /// End the session. With `flush`, deliver the resampler tail and partial frame.
    fn finish(&mut self, flush: bool) -> RecordingStats {
        let tail = self.resampler.flush();
        if flush {
            self.push(&tail);
            if !self.framer.is_empty() {
                if let Some(tx) = self.session.as_ref() {
                    let _ = tx.send(self.framer.clone());
                }
                self.emitted += self.framer.len() as u64;
            }
        }
        self.framer.clear();
        self.session = None; // drops the sender: the consumer sees the end
        RecordingStats { duration_ms: self.emitted * 1000 / SAMPLE_RATE as u64 }
    }
}

fn run_worker(ctx: WorkerCtx, ready: std::sync::mpsc::Sender<Result<(), RecorderError>>) {
    let WorkerCtx { device_name, level_tx, cmd_rx, closed } = ctx;
    let (data_tx, data_rx) = bounded::<Vec<f32>>(256);
    let (pool_tx, pool_rx) = bounded::<Vec<f32>>(64);
    let lost = Arc::new(AtomicBool::new(false));
    let (stream, channels, rate) =
        match open_stream(device_name.as_deref(), data_tx, pool_rx, lost.clone()) {
            Ok(v) => v,
            Err(e) => {
                let _ = ready.send(Err(e));
                closed.store(true, Ordering::Release);
                return;
            }
        };
    let _ = ready.send(Ok(()));

    let mut p = Pipeline {
        resampler: StreamResampler::new(channels, rate),
        ring: VecDeque::new(),
        framer: Vec::new(),
        session: None,
        emitted: 0,
        level_sum: 0.0,
        level_n: 0,
        level_tx,
    };
    let mut idle_since: Option<Instant> = None; // Some = warm, waiting for next dictation
    let mut failed = false;

    let consume = |p: &mut Pipeline, buf: Vec<f32>| {
        let out = p.resampler.process(&buf);
        p.push(&out);
        let _ = pool_tx.try_send(buf);
    };

    loop {
        select! {
            recv(data_rx) -> m => if let Ok(buf) = m { consume(&mut p, buf) },
            recv(cmd_rx) -> c => match c {
                Ok(Cmd::Start { frame_tx, ack }) => {
                    p.start(frame_tx, idle_since.is_some());
                    idle_since = None;
                    let _ = ack.send(());
                }
                Ok(Cmd::Stop { reply }) => {
                    if p.session.is_none() && !failed {
                        let _ = reply.send(Err(RecorderError::Other("not recording".into())));
                        continue;
                    }
                    while let Ok(buf) = data_rx.try_recv() { consume(&mut p, buf) }
                    let stats = p.finish(true);
                    if failed || lost.load(Ordering::Acquire) {
                        let _ = reply.send(Err(RecorderError::DeviceLost));
                        break;
                    }
                    idle_since = Some(Instant::now());
                    let _ = reply.send(Ok(stats));
                }
                Ok(Cmd::Cancel) => {
                    if p.session.is_some() {
                        p.finish(false);
                        idle_since = Some(Instant::now());
                    }
                }
                Ok(Cmd::Shutdown) | Err(_) => break,
            },
            default(POLL) => {}
        }
        if lost.load(Ordering::Acquire) && !failed {
            if p.session.is_some() {
                // Deliver what we have; `stop()` will report DeviceLost.
                p.finish(true);
                failed = true;
            } else {
                break;
            }
        }
        if idle_since.is_some_and(|t| t.elapsed() >= WARM_FOR) {
            break;
        }
    }
    drop(stream);
    closed.store(true, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_error_maps_to_device_lost() {
        let e: cpal::Error = cpal::ErrorKind::DeviceNotAvailable.into();
        assert_eq!(map_stream_error(&e), RecorderError::DeviceLost);
    }
}
