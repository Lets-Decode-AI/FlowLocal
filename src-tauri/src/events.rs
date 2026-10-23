//! Names of the events the backend emits to the UI.

pub const STATE: &str = "dictation://state";
pub const LEVEL: &str = "dictation://level";
pub const DL_PROGRESS: &str = "models://progress";
pub const TOAST: &str = "app://toast";
pub const TRANSCRIPT_ADDED: &str = "history://added";
/// Payload: the full `Settings` object, after `settings_set` applied it.
pub const SETTINGS_CHANGED: &str = "settings://changed";
/// The active model changed outside the Models page (tray menu); the page should re-fetch.
pub const MODELS_CHANGED: &str = "models://changed";
