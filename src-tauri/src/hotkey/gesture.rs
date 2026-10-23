//! Pure, time-injected hold / double-tap recogniser. It sees only the
//! chord's down/up transitions (see `ChordTracker`) and never reads a clock.
use super::HotkeyEvent;
use std::time::{Duration, Instant};

/// A press shorter than this counts as a "tap" for double-tap purposes.
pub const TAP_MAX: Duration = Duration::from_millis(250);
/// Max gap between the first release and the second press of a double-tap.
pub const DOUBLE_TAP_GAP: Duration = Duration::from_millis(300);

#[derive(Debug, Default)]
pub struct GestureDetector {
    down_at: Option<Instant>,
    /// Release time of the last short tap, if it can still start a double-tap.
    last_short_up: Option<Instant>,
    /// The current press was reported as `DoubleTap`; its release is silent.
    swallow_up: bool,
}

impl GestureDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chord transition (`down == true` for chord-down).
    pub fn on_key(&mut self, down: bool, at: Instant) -> Vec<HotkeyEvent> {
        if down {
            if self.down_at.is_some() {
                return vec![]; // auto-repeat
            }
            self.down_at = Some(at);
            let is_double = self
                .last_short_up
                .take()
                .is_some_and(|up| at.saturating_duration_since(up) <= DOUBLE_TAP_GAP);
            if is_double {
                self.swallow_up = true;
                vec![HotkeyEvent::DoubleTap]
            } else {
                vec![HotkeyEvent::Pressed]
            }
        } else {
            let Some(started) = self.down_at.take() else {
                return vec![];
            };
            if std::mem::take(&mut self.swallow_up) {
                return vec![];
            }
            if at.saturating_duration_since(started) < TAP_MAX {
                self.last_short_up = Some(at);
            }
            vec![HotkeyEvent::Released]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::HotkeyEvent::*;
    use std::time::{Duration, Instant};

    fn ms(base: Instant, n: u64) -> Instant {
        base + Duration::from_millis(n)
    }

    #[test]
    fn hold_emits_pressed_then_released() {
        let t = Instant::now();
        let mut g = GestureDetector::new();
        assert_eq!(g.on_key(true, ms(t, 0)), vec![Pressed]);
        assert_eq!(g.on_key(false, ms(t, 1000)), vec![Released]);
    }

    #[test]
    fn short_tap_emits_pressed_released() {
        let t = Instant::now();
        let mut g = GestureDetector::new();
        assert_eq!(g.on_key(true, ms(t, 0)), vec![Pressed]);
        assert_eq!(g.on_key(false, ms(t, 100)), vec![Released]);
    }

    #[test]
    fn double_tap_within_300ms_emits_doubletap() {
        let t = Instant::now();
        let mut g = GestureDetector::new();
        g.on_key(true, ms(t, 0));
        g.on_key(false, ms(t, 100));
        assert_eq!(g.on_key(true, ms(t, 350)), vec![DoubleTap]);
        assert!(g.on_key(false, ms(t, 450)).is_empty());
        // the gesture is consumed: a following press is a plain press
        assert_eq!(g.on_key(true, ms(t, 500)), vec![Pressed]);
    }

    #[test]
    fn slow_second_tap_is_new_press() {
        let t = Instant::now();
        let mut g = GestureDetector::new();
        g.on_key(true, ms(t, 0));
        g.on_key(false, ms(t, 100));
        assert_eq!(g.on_key(true, ms(t, 450)), vec![Pressed]);
        assert_eq!(g.on_key(false, ms(t, 900)), vec![Released]);
    }

    #[test]
    fn long_hold_then_quick_press_is_not_doubletap() {
        let t = Instant::now();
        let mut g = GestureDetector::new();
        g.on_key(true, ms(t, 0));
        g.on_key(false, ms(t, 600));
        assert_eq!(g.on_key(true, ms(t, 700)), vec![Pressed]);
    }

    #[test]
    fn repeated_down_and_stray_up_are_ignored() {
        let t = Instant::now();
        let mut g = GestureDetector::new();
        assert!(g.on_key(false, ms(t, 0)).is_empty());
        g.on_key(true, ms(t, 10));
        assert!(g.on_key(true, ms(t, 20)).is_empty());
    }
}
