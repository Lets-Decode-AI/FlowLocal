//! The dictation state machine: pure (no I/O, time is injected), driven by [`Input`]s and
//! answering with the next [`State`] plus the [`Effect`]s the controller must perform.

use std::time::Duration;

use serde::ser::{Serialize, SerializeMap, Serializer};
use tokio::time::Instant;

use crate::audio::RecorderError;
use crate::hotkey::HotkeyEvent;
use crate::insert::InsertOutcome;
use crate::stt::SttError;

/// A press shorter than this is a tap, not a dictation.
pub const MIN_HOLD: Duration = Duration::from_millis(250);
/// Hands-free recordings stop by themselves after this long.
pub const HANDS_FREE_MAX: Duration = Duration::from_secs(6 * 60);
/// How long the error state is shown.
pub const ERROR_SHOWN_FOR: Duration = Duration::from_secs(2);

/// Shown (state + toast) when the OS denies microphone access.
pub const MIC_DENIED: &str = "Microphone access denied";

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Idle,
    Recording { started: Instant, hands_free: bool },
    Transcribing,
    Inserting,
    Error { message: String, since: Instant },
}

/// UI shape: `{"kind": "idle"|"recording"|"transcribing"|"inserting"|"error", "handsFree"?, "message"?}`.
impl Serialize for State {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        match self {
            State::Idle => m.serialize_entry("kind", "idle")?,
            State::Recording { hands_free, .. } => {
                m.serialize_entry("kind", "recording")?;
                m.serialize_entry("handsFree", hands_free)?;
            }
            State::Transcribing => m.serialize_entry("kind", "transcribing")?,
            State::Inserting => m.serialize_entry("kind", "inserting")?,
            State::Error { message, .. } => {
                m.serialize_entry("kind", "error")?;
                m.serialize_entry("message", message)?;
            }
        }
        m.end()
    }
}

#[derive(Debug, Clone)]
pub enum Input {
    Hotkey(HotkeyEvent),
    RecorderFailed(RecorderError),
    /// There is no active speech model, so recording cannot start.
    NoModel,
    /// The OS denies microphone access, so recording cannot start.
    MicDenied,
    /// The oldest in-flight utterance finished transcribing (results arrive in utterance order).
    Transcribed(Result<String, SttError>),
    /// The oldest in-flight utterance contained no speech.
    NoSpeech,
    Inserted(InsertOutcome),
    /// The oldest in-flight utterance failed outside the speech engine (recorder stop, insert).
    Failed(String),
    Tick,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    StartRecording,
    StopRecording,
    CancelRecording,
    /// Finalize the stopped recording (tail chunk + join of chunk texts) in the background.
    Transcribe,
    Insert(String),
    Persist,
    /// Surface an error without changing the foreground state (a recording is in progress).
    ShowError(String),
    ArmEsc(bool),
    Emit(State),
}

fn recording(now: Instant, hands_free: bool) -> State {
    State::Recording {
        started: now,
        hands_free,
    }
}

fn error(message: String, now: Instant) -> State {
    State::Error {
        message,
        since: now,
    }
}

fn stop_effects() -> Vec<Effect> {
    vec![
        Effect::StopRecording,
        Effect::ArmEsc(false),
        Effect::Transcribe,
    ]
}

/// Appends `Emit(new)` when the state changed.
fn finish(old: &State, new: State, mut fx: Vec<Effect>) -> (State, Vec<Effect>) {
    if *old != new {
        fx.push(Effect::Emit(new.clone()));
    }
    (new, fx)
}

pub fn step(state: State, input: Input, now: Instant) -> (State, Vec<Effect>) {
    let old = state.clone();
    match (state, input) {
        (
            State::Recording {
                hands_free: true, ..
            },
            Input::Hotkey(HotkeyEvent::Pressed),
        ) => finish(&old, State::Transcribing, stop_effects()),
        (State::Recording { .. }, Input::Hotkey(HotkeyEvent::Pressed)) => (old, vec![]),
        (_, Input::Hotkey(HotkeyEvent::Pressed)) => finish(
            &old,
            recording(now, false),
            vec![Effect::StartRecording, Effect::ArmEsc(true)],
        ),

        (State::Recording { started, .. }, Input::Hotkey(HotkeyEvent::DoubleTap)) => {
            finish(&old, recording(started, true), vec![])
        }
        (_, Input::Hotkey(HotkeyEvent::DoubleTap)) => finish(
            &old,
            recording(now, true),
            vec![Effect::StartRecording, Effect::ArmEsc(true)],
        ),

        (
            State::Recording {
                started,
                hands_free: false,
            },
            Input::Hotkey(HotkeyEvent::Released),
        ) => {
            if now.saturating_duration_since(started) < MIN_HOLD {
                finish(
                    &old,
                    State::Idle,
                    vec![Effect::CancelRecording, Effect::ArmEsc(false)],
                )
            } else {
                finish(&old, State::Transcribing, stop_effects())
            }
        }

        (State::Recording { .. }, Input::Hotkey(HotkeyEvent::Cancel)) => finish(
            &old,
            State::Idle,
            vec![Effect::CancelRecording, Effect::ArmEsc(false)],
        ),

        (
            State::Recording {
                started,
                hands_free: true,
            },
            Input::Tick,
        ) if now.saturating_duration_since(started) >= HANDS_FREE_MAX => {
            finish(&old, State::Transcribing, stop_effects())
        }
        (State::Error { since, .. }, Input::Tick)
            if now.saturating_duration_since(since) >= ERROR_SHOWN_FOR =>
        {
            finish(&old, State::Idle, vec![])
        }

        (State::Recording { .. }, Input::RecorderFailed(e)) => finish(
            &old,
            error(e.to_string(), now),
            vec![Effect::CancelRecording, Effect::ArmEsc(false)],
        ),
        (_, Input::RecorderFailed(e)) => finish(&old, error(e.to_string(), now), vec![]),
        (State::Recording { .. }, Input::NoModel) => finish(
            &old,
            error("No model installed".into(), now),
            vec![Effect::CancelRecording, Effect::ArmEsc(false)],
        ),
        (_, Input::NoModel) => finish(&old, error("No model installed".into(), now), vec![]),

        (State::Recording { .. }, Input::MicDenied) => finish(
            &old,
            error(MIC_DENIED.into(), now),
            vec![Effect::CancelRecording, Effect::ArmEsc(false)],
        ),
        (_, Input::MicDenied) => finish(&old, error(MIC_DENIED.into(), now), vec![]),

        (State::Recording { .. }, Input::Transcribed(Ok(text))) if !text.trim().is_empty() => {
            (old, vec![Effect::Insert(text)])
        }
        (_, Input::Transcribed(Ok(text))) if !text.trim().is_empty() => {
            finish(&old, State::Inserting, vec![Effect::Insert(text)])
        }
        (State::Recording { .. }, Input::Transcribed(Ok(_))) => (old, vec![]),
        (_, Input::Transcribed(Ok(_))) => finish(&old, State::Idle, vec![]),
        (State::Recording { .. }, Input::Transcribed(Err(e))) => {
            (old, vec![Effect::ShowError(e.to_string())])
        }
        (_, Input::Transcribed(Err(e))) => finish(&old, error(e.to_string(), now), vec![]),

        (State::Recording { .. }, Input::Failed(msg)) => (old, vec![Effect::ShowError(msg)]),
        (_, Input::Failed(msg)) => finish(&old, error(msg, now), vec![]),

        (State::Transcribing, Input::NoSpeech) => finish(&old, State::Idle, vec![]),
        (State::Inserting, Input::Inserted(_)) => finish(&old, State::Idle, vec![Effect::Persist]),
        (_, Input::Inserted(_)) => (old, vec![Effect::Persist]),

        _ => (old, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::HotkeyEvent as H;
    use crate::insert::InsertOutcome;
    use crate::stt::SttError;
    use std::time::Duration;
    use tokio::time::Instant;

    fn rec(t: Instant, hands_free: bool) -> State {
        State::Recording {
            started: t,
            hands_free,
        }
    }
    fn has(effects: &[Effect], e: &Effect) -> bool {
        effects.contains(e)
    }

    #[test]
    fn pressed_in_idle_starts_recording_and_arms_esc() {
        let t = Instant::now();
        let (s, fx) = step(State::Idle, Input::Hotkey(H::Pressed), t);
        assert_eq!(s, rec(t, false));
        assert_eq!(
            fx,
            vec![
                Effect::StartRecording,
                Effect::ArmEsc(true),
                Effect::Emit(s.clone())
            ]
        );
    }

    #[test]
    fn pressed_while_transcribing_or_inserting_starts_new_recording() {
        let t = Instant::now();
        for from in [State::Transcribing, State::Inserting] {
            let (s, fx) = step(from, Input::Hotkey(H::Pressed), t);
            assert_eq!(s, rec(t, false));
            assert!(has(&fx, &Effect::StartRecording));
        }
    }

    #[test]
    fn short_release_cancels() {
        let t = Instant::now();
        let (s, fx) = step(
            rec(t, false),
            Input::Hotkey(H::Released),
            t + Duration::from_millis(249),
        );
        assert_eq!(s, State::Idle);
        assert!(has(&fx, &Effect::CancelRecording) && has(&fx, &Effect::ArmEsc(false)));
        assert!(!has(&fx, &Effect::Transcribe));
    }

    #[test]
    fn long_release_stops_and_transcribes() {
        let t = Instant::now();
        let (s, fx) = step(
            rec(t, false),
            Input::Hotkey(H::Released),
            t + Duration::from_millis(250),
        );
        assert_eq!(s, State::Transcribing);
        assert_eq!(
            fx,
            vec![
                Effect::StopRecording,
                Effect::ArmEsc(false),
                Effect::Transcribe,
                Effect::Emit(State::Transcribing)
            ]
        );
    }

    #[test]
    fn double_tap_starts_hands_free_and_press_stops_it() {
        let t = Instant::now();
        let (s, _) = step(State::Idle, Input::Hotkey(H::DoubleTap), t);
        assert_eq!(s, rec(t, true));
        // release of a hands-free recording is ignored
        let (s2, fx) = step(
            s.clone(),
            Input::Hotkey(H::Released),
            t + Duration::from_secs(5),
        );
        assert_eq!((s2.clone(), fx.len()), (s, 0));
        let (s3, fx) = step(s2, Input::Hotkey(H::Pressed), t + Duration::from_secs(6));
        assert_eq!(s3, State::Transcribing);
        assert!(has(&fx, &Effect::StopRecording) && has(&fx, &Effect::Transcribe));
    }

    #[test]
    fn tick_stops_hands_free_at_six_minutes_only() {
        let t = Instant::now();
        let hf = rec(t, true);
        let (s, fx) = step(hf.clone(), Input::Tick, t + Duration::from_secs(359));
        assert_eq!((s, fx.len()), (hf.clone(), 0));
        let (s, fx) = step(hf, Input::Tick, t + Duration::from_secs(360));
        assert_eq!(s, State::Transcribing);
        assert!(has(&fx, &Effect::StopRecording) && has(&fx, &Effect::Transcribe));
        // a held (non hands-free) recording never auto-stops
        let held = rec(t, false);
        let (s, fx) = step(held.clone(), Input::Tick, t + Duration::from_secs(900));
        assert_eq!((s, fx.len()), (held, 0));
    }

    #[test]
    fn cancel_in_recording_goes_idle() {
        let t = Instant::now();
        let (s, fx) = step(rec(t, false), Input::Hotkey(H::Cancel), t);
        assert_eq!(s, State::Idle);
        assert!(has(&fx, &Effect::CancelRecording) && has(&fx, &Effect::ArmEsc(false)));
    }

    #[test]
    fn error_expires_after_two_seconds_on_tick() {
        let t = Instant::now();
        let (s, _) = step(State::Idle, Input::NoModel, t);
        assert_eq!(
            s,
            State::Error {
                message: "No model installed".into(),
                since: t
            }
        );
        let (s, _) = step(s, Input::Tick, t + Duration::from_millis(1999));
        assert!(matches!(s, State::Error { .. }));
        let (s, fx) = step(s, Input::Tick, t + Duration::from_secs(2));
        assert_eq!(s, State::Idle);
        assert_eq!(fx, vec![Effect::Emit(State::Idle)]);
    }

    #[test]
    fn mic_denied_goes_to_error_without_recording() {
        let t = Instant::now();
        let (s, fx) = step(State::Idle, Input::MicDenied, t);
        assert_eq!(
            s,
            State::Error {
                message: "Microphone access denied".into(),
                since: t
            }
        );
        assert!(!has(&fx, &Effect::StartRecording));
    }

    #[test]
    fn recorder_failure_goes_to_error() {
        let t = Instant::now();
        let (s, fx) = step(
            rec(t, false),
            Input::RecorderFailed(crate::audio::RecorderError::DeviceLost),
            t,
        );
        assert_eq!(
            s,
            State::Error {
                message: "input device was lost".into(),
                since: t
            }
        );
        assert!(has(&fx, &Effect::CancelRecording) && has(&fx, &Effect::ArmEsc(false)));
    }

    #[test]
    fn transcribed_ok_inserts_then_inserted_persists_and_idles() {
        let t = Instant::now();
        let (s, fx) = step(State::Transcribing, Input::Transcribed(Ok("hi".into())), t);
        assert_eq!(s, State::Inserting);
        assert!(has(&fx, &Effect::Insert("hi".into())));
        let (s, fx) = step(s, Input::Inserted(InsertOutcome::Pasted), t);
        assert_eq!(s, State::Idle);
        assert!(has(&fx, &Effect::Persist));
    }

    #[test]
    fn transcribed_err_goes_to_error_but_not_over_a_recording() {
        let t = Instant::now();
        let e = SttError::EngineFailed("boom".into());
        let (s, _) = step(State::Transcribing, Input::Transcribed(Err(e.clone())), t);
        assert!(matches!(s, State::Error { .. }));
        let r = rec(t, false);
        let (s, fx) = step(r.clone(), Input::Transcribed(Err(e)), t);
        assert_eq!(s, r);
        assert!(fx.iter().any(|f| matches!(f, Effect::ShowError(_))));
    }

    #[test]
    fn results_do_not_disturb_a_new_recording() {
        let t = Instant::now();
        let r = rec(t, false);
        let (s, fx) = step(r.clone(), Input::Transcribed(Ok("a".into())), t);
        assert_eq!(s, r);
        assert!(has(&fx, &Effect::Insert("a".into())));
        let (s, fx) = step(r.clone(), Input::Inserted(InsertOutcome::Pasted), t);
        assert_eq!(s, r);
        assert!(has(&fx, &Effect::Persist));
    }

    #[test]
    fn state_serializes_for_the_ui() {
        let t = Instant::now();
        let j = |s: State| serde_json::to_value(s).unwrap();
        assert_eq!(j(State::Idle), serde_json::json!({"kind": "idle"}));
        assert_eq!(
            j(rec(t, true)),
            serde_json::json!({"kind": "recording", "handsFree": true})
        );
        assert_eq!(
            j(State::Transcribing),
            serde_json::json!({"kind": "transcribing"})
        );
        assert_eq!(
            j(State::Inserting),
            serde_json::json!({"kind": "inserting"})
        );
        assert_eq!(
            j(State::Error {
                message: "x".into(),
                since: t
            }),
            serde_json::json!({"kind": "error", "message": "x"})
        );
    }
}
