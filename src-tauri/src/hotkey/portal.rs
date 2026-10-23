//! Wayland backend. Compositors do not allow clients to observe global key
//! state; the only sanctioned route is the XDG GlobalShortcuts portal, which
//! only reports activation/deactivation of user-approved shortcuts. Support
//! for it is future work, so Part 1 reports the platform as unsupported and
//! the app surfaces that to the user.
use super::{Binding, HotkeyError, HotkeyEvent};
use std::sync::mpsc::Sender;

pub fn start(_binding: Binding, _tx: Sender<HotkeyEvent>) -> Result<(), HotkeyError> {
    Err(HotkeyError::Unsupported)
}
