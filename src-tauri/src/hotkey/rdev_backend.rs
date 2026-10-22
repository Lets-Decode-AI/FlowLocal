//! Windows / Linux-X11 backend built on `rdev::listen`.
//!
//! `rdev::listen` blocks forever and cannot be cancelled, so the listener
//! thread lives for the rest of the process; `stop()` sets a flag that makes
//! the callback ignore every event.
use super::{Core, HotkeyError, Key};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

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

pub fn start(core: Arc<Mutex<Core>>, stopped: Arc<AtomicBool>) -> Result<(), HotkeyError> {
    std::thread::Builder::new()
        .name("flowlocal-hotkey".into())
        .spawn(move || {
            let result = rdev::listen(move |event| {
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
            });
            if let Err(e) = result {
                tracing::error!("rdev listener exited: {e:?}");
            }
        })
        .map(|_| ())
        .map_err(|e| HotkeyError::Backend(e.to_string()))
}
