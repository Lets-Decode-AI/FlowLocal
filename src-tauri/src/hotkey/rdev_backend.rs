//! Windows / Linux-X11 backend built on `rdev::listen`.
//!
//! `rdev::listen` blocks forever and cannot be cancelled, so the listener
//! thread lives for the rest of the process; `stop()` sets a flag that makes
//! the callback ignore every event.
use super::{Core, HotkeyError, Key};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

fn map(key: rdev::Key) -> Option<Key> {
    use rdev::Key as R;
    Some(match key {
        R::ControlLeft => Key::ControlLeft,
        R::ControlRight => Key::ControlRight,
        R::MetaLeft => Key::MetaLeft,
        R::MetaRight => Key::MetaRight,
        R::Alt => Key::AltLeft,
        R::AltGr => Key::AltRight,
        R::ShiftLeft => Key::ShiftLeft,
        R::ShiftRight => Key::ShiftRight,
        R::Space => Key::Space,
        R::Escape => Key::Escape,
        _ => return None,
    })
}

/// How long `start` waits for an immediate listen failure before assuming
/// the listener is up (`rdev::listen` blocks forever on success).
const READY_WAIT: Duration = Duration::from_millis(300);

pub fn start(core: Arc<Mutex<Core>>, stopped: Arc<AtomicBool>) -> Result<(), HotkeyError> {
    // Discard modifiers whose release was missed (Win+L, Alt+Tab...) before reporting a chord.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    super::lock(&core).set_verifier(Box::new(super::physical::is_down));
    start_with(
        move || {
            rdev::listen(move |event| {
                if stopped.load(Ordering::Relaxed) {
                    return;
                }
                let (key, down) = match event.event_type {
                    rdev::EventType::KeyPress(k) => (k, true),
                    rdev::EventType::KeyRelease(k) => (k, false),
                    _ => return,
                };
                if let Some(key) = map(key) {
                    super::lock(&core).handle(key, down);
                }
            })
            .map_err(|e| format!("{e:?}"))
        },
        READY_WAIT,
    )
}

/// Runs `listen` on a thread. Best effort: a failure within `wait` is
/// returned; a listener that is still blocking is assumed healthy.
fn start_with<F>(listen: F, wait: Duration) -> Result<(), HotkeyError>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    let (err_tx, err_rx) = mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("flowlocal-hotkey".into())
        .spawn(move || {
            if let Err(msg) = listen() {
                tracing::error!("rdev listener exited: {msg}");
                let _ = err_tx.send(msg);
            }
        })
        .map_err(|e| HotkeyError::Backend(e.to_string()))?;
    match err_rx.recv_timeout(wait) {
        Ok(msg) => Err(HotkeyError::ListenFailed(msg)),
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn start_reports_immediate_listen_failure() {
        let r = start_with(|| Err("no display".to_string()), Duration::from_millis(300));
        assert!(matches!(r, Err(HotkeyError::ListenFailed(m)) if m == "no display"));
    }

    #[test]
    fn start_ok_when_listener_blocks() {
        let r = start_with(
            || {
                std::thread::sleep(Duration::from_secs(2));
                Ok(())
            },
            Duration::from_millis(50),
        );
        assert!(r.is_ok());
    }
}
