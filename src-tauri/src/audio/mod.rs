//! Microphone capture: cpal input stream -> worker thread (downmix, resample to 16 kHz
//! mono, 20 ms framing, levels, pre-roll) -> consumer channel.

pub mod resample;
pub mod vad;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use crossbeam_channel::{Receiver, bounded, select};

use resample::StreamResampler;

pub const SAMPLE_RATE: u32 = 16_000;
pub const FRAME_MS: u32 = 20;
/// Samples per 20 ms frame at 16 kHz (320).
pub const FRAME_LEN: usize = (SAMPLE_RATE * FRAME_MS / 1000) as usize;

/// Audio kept from before `start()` while the stream is warm (300 ms).
const PREROLL_SAMPLES: usize = (SAMPLE_RATE as usize) * 3 / 10;
/// The stream stays open this long after the last recording stops.
const WARM_FOR: Duration = Duration::from_secs(15);
/// One level event per ~33 ms of audio.
const LEVEL_SAMPLES: usize = (SAMPLE_RATE as usize) * 33 / 1000;
const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RecorderError {
    #[error("No microphone found.")]
    NoDevice,
    #[error("Microphone access is turned off.")]
    PermissionDenied,
    #[error("The microphone was disconnected.")]
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
        K::DeviceNotAvailable | K::DeviceChanged | K::StreamInvalidated => {
            RecorderError::DeviceLost
        }
        K::PermissionDenied => RecorderError::PermissionDenied,
        _ => RecorderError::Other(e.to_string()),
    }
}

/// Errors reported after the stream started that mean it can no longer deliver audio.
fn is_fatal(e: &cpal::Error) -> bool {
    matches!(
        map_stream_error(e),
        RecorderError::DeviceLost | RecorderError::PermissionDenied
    )
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
    Start {
        frame_tx: Sender<Vec<f32>>,
        ack: Sender<()>,
    },
    Stop {
        reply: Sender<Result<RecordingStats, RecorderError>>,
    },
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
    pub fn new(
        device_name: Option<String>,
        level_tx: SyncSender<f32>,
    ) -> Result<Recorder, RecorderError> {
        find_device(device_name.as_deref())?;
        Ok(Recorder {
            device_name,
            level_tx,
            inner: Mutex::new(None),
        })
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
        let handle = Handle {
            cmd_tx,
            closed,
            join: Some(join),
        };
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(RecorderError::Other(
                "timed out opening input stream".into(),
            )),
        }
    }

    fn try_start(h: &Handle, frame_tx: &Sender<Vec<f32>>) -> bool {
        if !h.is_open() {
            return false;
        }
        let (ack, ack_rx) = std::sync::mpsc::channel();
        h.cmd_tx
            .send(Cmd::Start {
                frame_tx: frame_tx.clone(),
                ack,
            })
            .is_ok()
            && ack_rx.recv_timeout(Duration::from_secs(1)).is_ok()
    }

    /// Begin streaming 16 kHz mono ~20 ms frames to `frame_tx` (pre-roll first when warm).
    pub fn start(&self, frame_tx: Sender<Vec<f32>>) -> Result<(), RecorderError> {
        let mut inner = self.inner.lock().unwrap();
        if inner
            .as_ref()
            .is_some_and(|h| Self::try_start(h, &frame_tx))
        {
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
    dropped: Arc<AtomicU64>,
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
            if data_tx.try_send(buf).is_err() {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        },
        move |e| {
            if is_fatal(&e) {
                tracing::error!("input stream failed: {e}");
                lost.store(true, Ordering::Release);
            } else {
                tracing::warn!("non-fatal input stream error: {e}");
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
    dropped: Arc<AtomicU64>,
) -> Result<(cpal::Stream, u16, u32), RecorderError> {
    let device = find_device(device_name)?;
    let supported = device
        .default_input_config()
        .map_err(|e| map_stream_error(&e))?;
    let (channels, rate, format) = (
        supported.channels(),
        supported.sample_rate(),
        supported.sample_format(),
    );
    let config: cpal::StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => build_stream::<f32>(&device, config, data_tx, pool_rx, lost, dropped),
        SampleFormat::I16 => build_stream::<i16>(&device, config, data_tx, pool_rx, lost, dropped),
        SampleFormat::U16 => build_stream::<u16>(&device, config, data_tx, pool_rx, lost, dropped),
        SampleFormat::I32 => build_stream::<i32>(&device, config, data_tx, pool_rx, lost, dropped),
        SampleFormat::I8 => build_stream::<i8>(&device, config, data_tx, pool_rx, lost, dropped),
        SampleFormat::U8 => build_stream::<u8>(&device, config, data_tx, pool_rx, lost, dropped),
        SampleFormat::F64 => build_stream::<f64>(&device, config, data_tx, pool_rx, lost, dropped),
        other => {
            return Err(RecorderError::Other(format!(
                "unsupported sample format {other}"
            )));
        }
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
        if self.session.is_none() {
            // Idle but warm: keep only the most recent 300 ms for the next start().
            self.ring.extend(samples.iter().copied());
            let excess = self.ring.len().saturating_sub(PREROLL_SAMPLES);
            self.ring.drain(..excess);
            return;
        }
        for &s in samples {
            self.level_sum += s * s;
            self.level_n += 1;
            if self.level_n == LEVEL_SAMPLES {
                let _ = self
                    .level_tx
                    .try_send((self.level_sum / LEVEL_SAMPLES as f32).sqrt());
                self.level_sum = 0.0;
                self.level_n = 0;
            }
        }
        self.framer.extend_from_slice(samples);
        self.emit_frames();
    }

    fn emit_frames(&mut self) {
        let Some(tx) = self.session.as_ref() else {
            return;
        };
        let whole = self.framer.len() / FRAME_LEN * FRAME_LEN;
        for frame in self.framer[..whole].as_chunks::<FRAME_LEN>().0 {
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
        self.ring.clear(); // never re-delivered to a later session
    }

    /// End the session. With `flush`, deliver the resampler tail and partial frame.
    fn finish(&mut self, flush: bool) -> Result<RecordingStats, RecorderError> {
        let tail = if flush {
            self.resampler.flush()
        } else {
            self.resampler.flush().map(|_| Vec::new())
        };
        if flush && let Ok(tail) = &tail {
            self.push(tail);
            if !self.framer.is_empty() {
                if let Some(tx) = self.session.as_ref() {
                    let _ = tx.send(self.framer.clone());
                }
                self.emitted += self.framer.len() as u64;
            }
        }
        // Clean up before reporting a resampler error, so the consumer always sees the end.
        self.framer.clear();
        self.ring.clear();
        self.session = None; // drops the sender: the consumer sees the end
        tail?;
        Ok(RecordingStats {
            duration_ms: self.emitted * 1000 / SAMPLE_RATE as u64,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Exit,
}

/// Hardware-independent worker state machine.
struct Engine {
    p: Pipeline,
    /// Some = warm, waiting for the next dictation.
    idle_since: Option<Instant>,
    /// The stream died (or processing failed) while a session was open; the worker only
    /// lives on to answer `stop()` with `DeviceLost`.
    failed: bool,
    lost: Arc<AtomicBool>,
}

impl Engine {
    fn new(
        channels: u16,
        rate: u32,
        level_tx: SyncSender<f32>,
        lost: Arc<AtomicBool>,
    ) -> Result<Self, RecorderError> {
        Ok(Self {
            p: Pipeline {
                resampler: StreamResampler::new(channels, rate)?,
                ring: VecDeque::new(),
                framer: Vec::new(),
                session: None,
                emitted: 0,
                level_sum: 0.0,
                level_n: 0,
                level_tx,
            },
            idle_since: None,
            failed: false,
            lost,
        })
    }

    /// Process one raw interleaved buffer; a resampler error marks the stream lost.
    fn feed(&mut self, buf: &[f32]) {
        match self.p.resampler.process(buf) {
            Ok(out) => self.p.push(&out),
            Err(e) => {
                tracing::error!("audio processing failed: {e}");
                self.lost.store(true, Ordering::Release);
            }
        }
    }

    fn on_cmd(&mut self, cmd: Cmd, drain: &mut dyn FnMut() -> Option<Vec<f32>>) -> Flow {
        match cmd {
            Cmd::Start { frame_tx, ack } => {
                if self.failed {
                    return Flow::Exit; // ack dropped: the recorder reopens a fresh stream
                }
                self.p.start(frame_tx, self.idle_since.is_some());
                self.idle_since = None;
                let _ = ack.send(());
                Flow::Continue
            }
            Cmd::Stop { reply } => {
                if self.p.session.is_none() && !self.failed {
                    let _ = reply.send(Err(RecorderError::Other("not recording".into())));
                    return Flow::Continue;
                }
                while let Some(buf) = drain() {
                    self.feed(&buf);
                }
                let stats = self.p.finish(true);
                if self.failed || self.lost.load(Ordering::Acquire) {
                    let _ = reply.send(Err(RecorderError::DeviceLost));
                    return Flow::Exit;
                }
                self.idle_since = Some(Instant::now());
                let _ = reply.send(stats);
                Flow::Continue
            }
            Cmd::Cancel => {
                if self.failed {
                    return Flow::Exit;
                }
                if self.p.session.is_some() {
                    let _ = self.p.finish(false);
                    self.idle_since = Some(Instant::now());
                }
                Flow::Continue
            }
            Cmd::Shutdown => Flow::Exit,
        }
    }

    /// Check for stream loss and warm-timer expiry.
    fn poll(&mut self) -> Flow {
        if self.lost.load(Ordering::Acquire) && !self.failed {
            if self.p.session.is_some() {
                // Deliver what we have; `stop()` will report DeviceLost.
                let _ = self.p.finish(true);
                self.failed = true;
            } else {
                return Flow::Exit;
            }
        }
        if self.idle_since.is_some_and(|t| t.elapsed() >= WARM_FOR) {
            return Flow::Exit;
        }
        Flow::Continue
    }
}

fn run_worker(ctx: WorkerCtx, ready: std::sync::mpsc::Sender<Result<(), RecorderError>>) {
    let WorkerCtx {
        device_name,
        level_tx,
        cmd_rx,
        closed,
    } = ctx;
    let (data_tx, data_rx) = bounded::<Vec<f32>>(256);
    let (pool_tx, pool_rx) = bounded::<Vec<f32>>(64);
    let lost = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));
    let (stream, channels, rate) = match open_stream(
        device_name.as_deref(),
        data_tx,
        pool_rx,
        lost.clone(),
        dropped.clone(),
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            closed.store(true, Ordering::Release);
            return;
        }
    };
    let mut engine = match Engine::new(channels, rate, level_tx, lost) {
        Ok(e) => e,
        Err(e) => {
            let _ = ready.send(Err(e));
            closed.store(true, Ordering::Release);
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut reported_drops = 0;
    loop {
        select! {
            recv(data_rx) -> m => if let Ok(buf) = m {
                engine.feed(&buf);
                let _ = pool_tx.try_send(buf);
            },
            recv(cmd_rx) -> c => {
                let flow = match c {
                    Ok(cmd) => engine.on_cmd(cmd, &mut || data_rx.try_recv().ok()),
                    Err(_) => Flow::Exit,
                };
                if flow == Flow::Exit {
                    break;
                }
            },
            default(POLL) => {}
        }
        let drops = dropped.load(Ordering::Relaxed);
        if drops != reported_drops {
            tracing::warn!(
                "dropped {} audio callback buffers (worker too slow)",
                drops - reported_drops
            );
            reported_drops = drops;
        }
        if engine.poll() == Flow::Exit {
            break;
        }
    }
    drop(stream);
    closed.store(true, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{TryRecvError, channel, sync_channel};

    #[test]
    fn stream_error_maps_to_device_lost() {
        let e: cpal::Error = cpal::ErrorKind::DeviceNotAvailable.into();
        assert_eq!(map_stream_error(&e), RecorderError::DeviceLost);
    }

    #[test]
    fn device_lost_then_cancel_then_start_reopens_or_errors() {
        let lost = Arc::new(AtomicBool::new(false));
        let (ltx, _lrx) = sync_channel(8);
        let mut e = Engine::new(1, 16_000, ltx, lost.clone()).unwrap();
        let (ftx, frx) = channel();
        let (ack, ack_rx) = channel();
        assert_eq!(
            e.on_cmd(Cmd::Start { frame_tx: ftx, ack }, &mut || None),
            Flow::Continue
        );
        ack_rx.recv().unwrap();
        e.feed(&vec![0.1; 1600]);
        lost.store(true, Ordering::Release);
        assert_eq!(e.poll(), Flow::Continue);
        assert!(frx.try_iter().count() > 0);
        assert_eq!(frx.try_recv().unwrap_err(), TryRecvError::Disconnected);
        // cancel after loss must close the worker, not linger
        assert_eq!(e.on_cmd(Cmd::Cancel, &mut || None), Flow::Exit);

        // a start on a failed worker must not ack (Recorder then reopens a fresh stream)
        let (ftx2, _frx2) = channel();
        let (ack2, ack2_rx) = channel();
        assert_eq!(
            e.on_cmd(
                Cmd::Start {
                    frame_tx: ftx2,
                    ack: ack2
                },
                &mut || None
            ),
            Flow::Exit
        );
        assert!(ack2_rx.recv().is_err());
    }

    #[test]
    fn stop_after_loss_reports_device_lost() {
        let lost = Arc::new(AtomicBool::new(false));
        let (ltx, _lrx) = sync_channel(8);
        let mut e = Engine::new(1, 16_000, ltx, lost.clone()).unwrap();
        let (ftx, _frx) = channel();
        let (ack, _ack_rx) = channel();
        e.on_cmd(Cmd::Start { frame_tx: ftx, ack }, &mut || None);
        lost.store(true, Ordering::Release);
        e.poll();
        let (reply, reply_rx) = channel();
        assert_eq!(e.on_cmd(Cmd::Stop { reply }, &mut || None), Flow::Exit);
        assert_eq!(
            reply_rx.recv().unwrap().unwrap_err(),
            RecorderError::DeviceLost
        );
    }

    #[test]
    fn preroll_not_redelivered_across_sessions() {
        let lost = Arc::new(AtomicBool::new(false));
        let (ltx, _lrx) = sync_channel(8);
        let mut e = Engine::new(1, 16_000, ltx, lost).unwrap();
        let (ftx, _frx) = channel();
        let (ack, _a) = channel();
        e.on_cmd(Cmd::Start { frame_tx: ftx, ack }, &mut || None);
        e.feed(&vec![0.1; 16_000]);
        let (reply, _r) = channel();
        e.on_cmd(Cmd::Stop { reply }, &mut || None);
        let (ftx, frx) = channel();
        let (ack, _a) = channel();
        e.on_cmd(Cmd::Start { frame_tx: ftx, ack }, &mut || None);
        assert_eq!(
            frx.try_iter().count(),
            0,
            "warm start with nothing since stop must send no pre-roll"
        );
    }
}
