use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Result, anyhow};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

pub trait Keystroke {
    fn paste(&self) -> Result<()>;
}

/// Synthetic paste via enigo: Cmd+V on macOS, Ctrl+V elsewhere. An `Enigo`
/// is created per call (cheap, avoids holding non-Send OS handles).
pub struct EnigoKeystroke;

#[cfg(target_os = "macos")]
const MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const MODIFIER: Key = Key::Control;

impl Keystroke for EnigoKeystroke {
    fn paste(&self) -> Result<()> {
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow!("enigo init: {e}"))?;
        enigo
            .key(MODIFIER, Direction::Press)
            .map_err(|e| anyhow!("modifier press: {e}"))?;
        let clicked = enigo.key(Key::Unicode('v'), Direction::Click);
        // Always release the modifier, even if the V click failed.
        let released = enigo.key(MODIFIER, Direction::Release);
        clicked.map_err(|e| anyhow!("v click: {e}"))?;
        released.map_err(|e| anyhow!("modifier release: {e}"))?;
        Ok(())
    }
}

pub type Job = Box<dyn FnOnce() + Send>;
/// Schedules a job on the app's main thread (Tauri's `run_on_main_thread`).
pub type Runner = Arc<dyn Fn(Job) + Send + Sync>;

/// How long the caller waits for the main thread to perform the paste.
const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(2);

/// Runs the inner keystroke on the main thread and waits for it.
///
/// On macOS, enigo resolves the "v" key through the Text Input Sources API,
/// which asserts that it runs on the main dispatch queue; calling it from a
/// worker thread aborts the process. Must not be called from the main thread
/// itself, or it waits on a job that can never run.
pub struct MainThreadKeystroke<K> {
    inner: Arc<K>,
    runner: Runner,
}

impl<K: Keystroke + Send + Sync + 'static> MainThreadKeystroke<K> {
    pub fn new(inner: K, runner: Runner) -> Self {
        Self {
            inner: Arc::new(inner),
            runner,
        }
    }
}

impl<K: Keystroke + Send + Sync + 'static> Keystroke for MainThreadKeystroke<K> {
    fn paste(&self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let inner = self.inner.clone();
        (self.runner)(Box::new(move || {
            let _ = tx.send(inner.paste());
        }));
        rx.recv_timeout(MAIN_THREAD_TIMEOUT)
            .map_err(|_| anyhow!("the paste did not run on the main thread"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread::{self, ThreadId};

    /// Records the thread the inner keystroke ran on.
    struct Probe(Arc<Mutex<Option<ThreadId>>>);
    impl Keystroke for Probe {
        fn paste(&self) -> Result<()> {
            *self.0.lock().unwrap() = Some(thread::current().id());
            Ok(())
        }
    }

    #[test]
    fn paste_runs_on_the_runner_thread_not_the_caller() {
        // Stand-in for the main thread: a dedicated thread that runs jobs.
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let main = thread::spawn(move || {
            let id = thread::current().id();
            if let Ok(job) = rx.recv() {
                job();
            }
            id
        });
        let runner: Runner = Arc::new(move |job| {
            let _ = tx.send(job);
        });
        let seen = Arc::new(Mutex::new(None));
        let ks = MainThreadKeystroke::new(Probe(seen.clone()), runner);

        ks.paste().unwrap();

        let main_id = main.join().unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(main_id));
        assert_ne!(main_id, thread::current().id());
    }

    #[test]
    fn paste_errors_if_the_runner_drops_the_job() {
        let runner: Runner = Arc::new(|_job| {});
        let ks = MainThreadKeystroke::new(Probe(Arc::new(Mutex::new(None))), runner);
        assert!(ks.paste().is_err());
    }
}
