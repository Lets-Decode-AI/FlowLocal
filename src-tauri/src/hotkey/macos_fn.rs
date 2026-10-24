//! macOS backend: an active CGEventTap on a dedicated thread with its own run
//! loop. The callback returns `Keep` for every event, so events are never
//! swallowed or modified (Esc still reaches the focused app). An active tap
//! is gated by the Accessibility permission only; without it tap creation
//! fails and we report `TapUnavailable`.
use super::{Core, HotkeyError, Key};
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

const FLAG_SECONDARY_FN: u64 = 0x0080_0000;
const KC_ESCAPE: i64 = 53;
const KC_SPACE: i64 = 49;

/// Device-dependent modifier bits (IOLLEvent.h), which distinguish left from
/// right: (keycode, flag bit, key).
const MODIFIERS: [(i64, u64, Key); 8] = [
    (59, 0x0000_0001, Key::ControlLeft),
    (62, 0x0000_2000, Key::ControlRight),
    (56, 0x0000_0002, Key::ShiftLeft),
    (60, 0x0000_0004, Key::ShiftRight),
    (55, 0x0000_0008, Key::MetaLeft),
    (54, 0x0000_0010, Key::MetaRight),
    (58, 0x0000_0020, Key::AltLeft),
    (61, 0x0000_0040, Key::AltRight),
];

/// Translate a flagsChanged event into (key, down).
#[cfg(test)]
fn from_flags_changed(keycode: i64, flags: u64) -> Option<(Key, bool)> {
    if keycode == 63 {
        return Some((Key::Fn, flags & FLAG_SECONDARY_FN != 0));
    }
    MODIFIERS
        .iter()
        .find(|(kc, _, _)| *kc == keycode)
        .map(|(_, bit, key)| (*key, flags & bit != 0))
}

/// Down-state of every modifier (and Fn) according to `flags`. Applied on
/// each flagsChanged so a missed key-up cannot leave a modifier stuck.
fn modifier_states(flags: u64) -> Vec<(Key, bool)> {
    let mut v: Vec<_> = MODIFIERS
        .iter()
        .map(|(_, bit, key)| (*key, flags & bit != 0))
        .collect();
    v.push((Key::Fn, flags & FLAG_SECONDARY_FN != 0));
    v
}

fn from_keycode(keycode: i64) -> Option<Key> {
    match keycode {
        KC_ESCAPE => Some(Key::Escape),
        KC_SPACE => Some(Key::Space),
        _ => None,
    }
}

pub fn start(core: Arc<Mutex<Core>>, stopped: Arc<AtomicBool>) -> Result<(), HotkeyError> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), HotkeyError>>();
    std::thread::Builder::new()
        .name("uttr-hotkey".into())
        .spawn(move || run(core, stopped, ready_tx))
        .map_err(|e| HotkeyError::Backend(e.to_string()))?;
    ready_rx
        .recv()
        .unwrap_or_else(|_| Err(HotkeyError::Backend("hotkey thread died".into())))
}

fn run(
    core: Arc<Mutex<Core>>,
    stopped: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), HotkeyError>>,
) {
    let cb_core = core.clone();
    let cb_stopped = stopped.clone();
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![
            CGEventType::FlagsChanged,
            CGEventType::KeyDown,
            CGEventType::KeyUp,
        ],
        move |_proxy, etype, event| {
            if cb_stopped.load(Ordering::Relaxed) {
                return CallbackResult::Keep;
            }
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            let mapped = match etype {
                CGEventType::FlagsChanged => {
                    let mut core = super::lock(&cb_core);
                    for (key, down) in modifier_states(event.get_flags().bits()) {
                        core.handle(key, down);
                    }
                    None
                }
                CGEventType::KeyDown => from_keycode(keycode).map(|k| (k, true)),
                CGEventType::KeyUp => from_keycode(keycode).map(|k| (k, false)),
                // The system disabled the tap; key-ups may have been missed.
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
                    super::lock(&cb_core).reset();
                    None
                }
                _ => None,
            };
            if let Some((key, down)) = mapped {
                super::lock(&cb_core).handle(key, down);
            }
            CallbackResult::Keep
        },
    );
    let Ok(tap) = tap else {
        let _ = ready.send(Err(HotkeyError::TapUnavailable));
        return;
    };
    let Ok(source) = tap.mach_port().create_runloop_source(0) else {
        let _ = ready.send(Err(HotkeyError::TapUnavailable));
        return;
    };
    CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    let _ = ready.send(Ok(()));

    while !stopped.load(Ordering::Relaxed) {
        CFRunLoop::run_in_mode(
            unsafe { kCFRunLoopDefaultMode },
            Duration::from_secs(5),
            false,
        );
        // Cheap and idempotent; recovers a tap the system disabled.
        tap.enable();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fn_flag_maps_to_fn_key() {
        assert_eq!(
            from_flags_changed(63, FLAG_SECONDARY_FN),
            Some((Key::Fn, true))
        );
        assert_eq!(from_flags_changed(63, 0), Some((Key::Fn, false)));
    }

    #[test]
    fn left_right_modifiers_are_distinct() {
        assert_eq!(from_flags_changed(61, 0x40), Some((Key::AltRight, true)));
        assert_eq!(from_flags_changed(58, 0x40), Some((Key::AltLeft, false)));
        assert_eq!(from_flags_changed(0, 0x40), None);
    }

    #[test]
    fn flags_resync_releases_missed_modifier() {
        let states = modifier_states(0x40);
        assert!(states.contains(&(Key::AltRight, true)));
        assert!(states.contains(&(Key::ControlLeft, false)));
        assert!(states.contains(&(Key::Fn, false)));
        assert_eq!(states.len(), 9);
    }
}
