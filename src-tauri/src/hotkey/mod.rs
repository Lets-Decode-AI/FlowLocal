//! Global push-to-talk hotkeys: raw key transitions from a platform backend
//! -> chord tracking -> `GestureDetector` -> `HotkeyEvent` on an mpsc channel.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};
use std::time::Instant;

pub mod gesture;
#[cfg(target_os = "macos")]
pub mod macos_fn;
pub mod portal;
#[cfg(not(target_os = "macos"))]
pub mod rdev_backend;

use gesture::GestureDetector;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
    DoubleTap,
    Cancel,
}

#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    /// The macOS event tap could not be created (Accessibility / Input
    /// Monitoring not granted). Callers may retry with `fallback_binding()`.
    #[error("keyboard event tap unavailable (grant Accessibility permission)")]
    TapUnavailable,
    #[error("global hotkeys are not supported on this platform/session")]
    Unsupported,
    #[error("keyboard listener failed to start: {0}")]
    ListenFailed(String),
    #[error("hotkey backend error: {0}")]
    Backend(String),
}

/// Keys that can take part in a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    Fn,
    ControlLeft,
    ControlRight,
    MetaLeft,
    MetaRight,
    AltLeft,
    AltRight,
    ShiftLeft,
    ShiftRight,
    Space,
    Escape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub keys: Vec<Key>,
}

/// Hold `Fn` on macOS, `Ctrl+Super` elsewhere.
pub fn default_binding() -> Binding {
    #[cfg(target_os = "macos")]
    let keys = vec![Key::Fn];
    #[cfg(not(target_os = "macos"))]
    let keys = vec![Key::ControlLeft, Key::MetaLeft];
    Binding { keys }
}

/// Binding to use when the default `Fn` key cannot be observed (macOS event
/// tap unavailable). Applying it is the settings layer's decision.
pub fn fallback_binding() -> Binding {
    Binding { keys: vec![Key::AltRight] }
}

/// Tracks which keys are down and reports chord transitions.
pub struct ChordTracker {
    binding: Binding,
    down: HashSet<Key>,
    active: bool,
}

impl ChordTracker {
    pub fn new(binding: Binding) -> Self {
        Self { binding, down: HashSet::new(), active: false }
    }

    /// `Some(true)` when the chord becomes fully held, `Some(false)` when it
    /// stops being held, `None` otherwise (including key repeat).
    pub fn on_key(&mut self, key: Key, down: bool) -> Option<bool> {
        if down {
            self.down.insert(key);
        } else {
            self.down.remove(&key);
        }
        let now = !self.binding.keys.is_empty() && self.binding.keys.iter().all(|k| self.down.contains(k));
        (now != std::mem::replace(&mut self.active, now)).then_some(now)
    }
}

/// State shared between the service handle and the backend thread.
pub struct Core {
    chord: ChordTracker,
    gesture: GestureDetector,
    pub armed: bool,
    esc_down: bool,
    tx: Sender<HotkeyEvent>,
}

impl Core {
    pub fn new(binding: Binding, tx: Sender<HotkeyEvent>) -> Self {
        Self { chord: ChordTracker::new(binding), gesture: GestureDetector::new(), armed: false, esc_down: false, tx }
    }

    fn emit(&self, events: impl IntoIterator<Item = HotkeyEvent>) {
        for e in events {
            // Unbounded channel: never blocks. A dropped receiver is fine.
            let _ = self.tx.send(e);
        }
    }

    pub fn handle(&mut self, key: Key, down: bool) {
        if key == Key::Escape && !self.chord.binding.keys.contains(&Key::Escape) {
            let first_down = down && !std::mem::replace(&mut self.esc_down, true);
            if !down {
                self.esc_down = false;
            }
            if first_down && self.armed {
                self.emit([HotkeyEvent::Cancel]);
            }
            return;
        }
        if let Some(chord_down) = self.chord.on_key(key, down) {
            let events = self.gesture.on_key(chord_down, Instant::now());
            self.emit(events);
        }
    }

    /// Forget all held keys, releasing the chord if it was held.
    pub fn reset(&mut self) {
        self.chord.down.clear();
        if std::mem::take(&mut self.chord.active) {
            let events = self.gesture.on_key(false, Instant::now());
            self.emit(events);
        }
    }

    pub fn set_binding(&mut self, binding: Binding) {
        self.reset();
        self.chord = ChordTracker::new(binding);
    }
}

fn lock(core: &Mutex<Core>) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct HotkeyService {
    core: Arc<Mutex<Core>>,
    stopped: Arc<AtomicBool>,
}

impl HotkeyService {
    pub fn start(binding: Binding, tx: Sender<HotkeyEvent>) -> Result<HotkeyService, HotkeyError> {
        let core = Arc::new(Mutex::new(Core::new(binding.clone(), tx.clone())));
        let stopped = Arc::new(AtomicBool::new(false));
        Self::start_backend(binding, tx, core.clone(), stopped.clone())?;
        Ok(HotkeyService { core, stopped })
    }

    #[cfg(target_os = "macos")]
    fn start_backend(
        _binding: Binding,
        _tx: Sender<HotkeyEvent>,
        core: Arc<Mutex<Core>>,
        stopped: Arc<AtomicBool>,
    ) -> Result<(), HotkeyError> {
        macos_fn::start(core, stopped)
    }

    #[cfg(not(target_os = "macos"))]
    fn start_backend(
        binding: Binding,
        tx: Sender<HotkeyEvent>,
        core: Arc<Mutex<Core>>,
        stopped: Arc<AtomicBool>,
    ) -> Result<(), HotkeyError> {
        if crate::insert::is_wayland() {
            return portal::start(binding, tx);
        }
        rdev_backend::start(core, stopped)
    }

    pub fn set_binding(&self, binding: Binding) {
        lock(&self.core).set_binding(binding);
    }

    /// Esc emits `Cancel` only while armed (set by the dictation controller
    /// for the duration of a recording).
    pub fn set_armed(&self, armed: bool) {
        lock(&self.core).armed = armed;
    }

    /// Stop delivering events. Backend threads exit on their next wake-up
    /// (macOS) or stay parked inside `rdev::listen` ignoring events (which
    /// cannot be cancelled).
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_requires_all_keys() {
        let mut c = ChordTracker::new(Binding {
            keys: vec![Key::ControlLeft, Key::MetaLeft],
        });
        assert_eq!(c.on_key(Key::ControlLeft, true), None);
        assert_eq!(c.on_key(Key::MetaLeft, true), Some(true));
        assert_eq!(c.on_key(Key::MetaLeft, true), None); // key repeat
        assert_eq!(c.on_key(Key::ControlLeft, false), Some(false));
        assert_eq!(c.on_key(Key::MetaLeft, false), None);
    }

    #[test]
    fn binding_serde_roundtrip() {
        let b = Binding {
            keys: vec![Key::ControlLeft, Key::MetaLeft],
        };
        let s = serde_json::to_string(&b).unwrap();
        assert_eq!(s, r#"{"keys":["ControlLeft","MetaLeft"]}"#);
        assert_eq!(serde_json::from_str::<Binding>(&s).unwrap(), b);
        assert_eq!(
            serde_json::from_str::<Binding>(r#"{"keys":["Fn"]}"#).unwrap().keys,
            vec![Key::Fn]
        );
    }

    #[test]
    fn escape_cancels_only_when_armed() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut core = Core::new(Binding { keys: vec![Key::Fn] }, tx);
        core.handle(Key::Escape, true);
        assert!(rx.try_recv().is_err());
        core.handle(Key::Escape, false);
        core.armed = true;
        core.handle(Key::Escape, true);
        assert_eq!(rx.try_recv().unwrap(), HotkeyEvent::Cancel);
    }

    #[test]
    fn set_binding_mid_hold_releases() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut core = Core::new(Binding { keys: vec![Key::Fn] }, tx);
        core.handle(Key::Fn, true);
        assert_eq!(rx.try_recv().unwrap(), HotkeyEvent::Pressed);
        core.set_binding(Binding { keys: vec![Key::AltRight] });
        assert_eq!(rx.try_recv().unwrap(), HotkeyEvent::Released);
    }

    #[test]
    fn held_esc_cancels_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut core = Core::new(Binding { keys: vec![Key::Fn] }, tx);
        core.armed = true;
        core.handle(Key::Escape, true);
        core.handle(Key::Escape, true);
        core.handle(Key::Escape, true);
        assert_eq!(rx.try_recv().unwrap(), HotkeyEvent::Cancel);
        assert!(rx.try_recv().is_err());
        core.handle(Key::Escape, false);
        core.handle(Key::Escape, true);
        assert_eq!(rx.try_recv().unwrap(), HotkeyEvent::Cancel);
    }

    #[test]
    fn wayland_with_xwayland_display_is_wayland() {
        // XWayland sessions also set DISPLAY; only the Wayland vars matter.
        assert!(crate::insert::detect_wayland(Some("wayland-0"), None));
        assert!(crate::insert::detect_wayland(None, Some("wayland")));
        assert!(!crate::insert::detect_wayland(None, Some("x11")));
        assert!(!crate::insert::detect_wayland(None, None));
    }
}
