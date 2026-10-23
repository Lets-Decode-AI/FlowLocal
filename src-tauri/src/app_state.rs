//! Shared application state, persisted settings and the DTOs the UI consumes.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audio::{Recorder, RecorderError, RecordingStats};
use crate::dictation::{ControllerHandle, RecorderPort};
use crate::hotkey::{Binding, HotkeyService, default_binding};
use crate::models::Manager;
use crate::models::catalog::CatalogEntry;
use crate::models::hardware::{Hardware, recommend};
use crate::store::Store;
use crate::stt::SttHandle;

/// Key of the single JSON settings document in the store's settings table.
pub const SETTINGS_KEY: &str = "settings";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hotkey: Binding,
    /// Input device name; `None` = system default.
    pub mic: Option<String>,
    pub show_flowbar: bool,
    /// Minutes of inactivity before the model is unloaded; `None` = never.
    pub idle_unload_min: Option<u32>,
    pub preload: bool,
    pub launch_at_login: bool,
    pub onboarded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey: default_binding(),
            mic: None,
            show_flowbar: true,
            idle_unload_min: Some(10),
            preload: false,
            launch_at_login: false,
            onboarded: false,
        }
    }
}

impl Settings {
    pub fn idle_unload(&self) -> Option<std::time::Duration> {
        self.idle_unload_min
            .map(|m| std::time::Duration::from_secs(u64::from(m) * 60))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelDto {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub disk_bytes: u64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendDto {
    pub id: String,
    pub hardware: Hardware,
}

pub fn recommend_dto(hw: &Hardware, locale: &str) -> RecommendDto {
    RecommendDto {
        id: recommend(hw, locale).to_string(),
        hardware: hw.clone(),
    }
}

/// OS locale (e.g. `en-US`), falling back to `$LANG`, then `en-US`.
pub fn os_locale() -> String {
    sys_locale::get_locale()
        .or_else(|| {
            std::env::var("LANG")
                .ok()
                .map(|l| l.split('.').next().unwrap_or("").replace('_', "-"))
        })
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "en-US".into())
}

/// A recorder whose input device can be changed at any time: the new device takes effect at the
/// next recording start (the cpal stream is only opened by `start`, so nothing live is disturbed).
pub struct SwitchableRecorder {
    level_tx: SyncSender<f32>,
    current: Mutex<Option<Arc<Recorder>>>,
    wanted: Mutex<Option<String>>,
    dirty: AtomicBool,
}

impl SwitchableRecorder {
    pub fn new(mic: Option<String>, level_tx: SyncSender<f32>) -> Self {
        Self {
            level_tx,
            current: Mutex::new(None),
            wanted: Mutex::new(mic),
            dirty: AtomicBool::new(true),
        }
    }

    pub fn set_mic(&self, mic: Option<String>) {
        *self.wanted.lock().unwrap_or_else(|p| p.into_inner()) = mic;
        self.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn active(&self) -> Option<Arc<Recorder>> {
        self.current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn recorder(&self) -> Result<Arc<Recorder>, RecorderError> {
        let mut cur = self.current.lock().unwrap_or_else(|p| p.into_inner());
        if self.dirty.swap(false, std::sync::atomic::Ordering::SeqCst) || cur.is_none() {
            let wanted = self
                .wanted
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let rec = match Recorder::new(wanted.clone(), self.level_tx.clone()) {
                Ok(r) => r,
                // The chosen device vanished: fall back to the system default.
                Err(_) if wanted.is_some() => Recorder::new(None, self.level_tx.clone())?,
                Err(e) => return Err(e),
            };
            *cur = Some(Arc::new(rec));
        }
        Ok(cur.clone().expect("recorder just created"))
    }
}

impl RecorderPort for SwitchableRecorder {
    fn start(&self, frame_tx: std::sync::mpsc::Sender<Vec<f32>>) -> Result<(), RecorderError> {
        self.recorder()?.start(frame_tx)
    }
    fn stop(&self) -> Result<RecordingStats, RecorderError> {
        // Never rebuild here: the recording in progress belongs to the current recorder.
        match self.active() {
            Some(r) => r.stop(),
            None => Err(RecorderError::Other("not recording".into())),
        }
    }
    fn cancel(&self) {
        if let Some(r) = self.active() {
            r.cancel();
        }
    }
}

/// Launch-at-login registration (the Tauri autostart plugin in the app, a fake in tests).
pub trait AutostartPort: Send + Sync {
    fn is_enabled(&self) -> Result<bool, String>;
    fn set(&self, on: bool) -> Result<(), String>;
}

pub struct PluginAutostart(pub tauri::AppHandle);

impl AutostartPort for PluginAutostart {
    fn is_enabled(&self) -> Result<bool, String> {
        use tauri_plugin_autostart::ManagerExt;
        self.0.autolaunch().is_enabled().map_err(|e| e.to_string())
    }
    fn set(&self, on: bool) -> Result<(), String> {
        use tauri_plugin_autostart::ManagerExt;
        let a = self.0.autolaunch();
        (if on { a.enable() } else { a.disable() }).map_err(|e| e.to_string())
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn validate_settings(s: &Settings) -> Result<(), String> {
    if s.hotkey.keys.is_empty() {
        return Err("Choose at least one key for the shortcut.".into());
    }
    if s.hotkey.keys.contains(&crate::hotkey::Key::Escape) {
        return Err("Esc cannot be part of the shortcut because it cancels dictation.".into());
    }
    Ok(())
}

/// Validates `new`, applies the fallible side effect (autostart) first, and only then persists
/// and publishes it. On error nothing is changed. Returns the previous settings.
pub fn commit_settings(
    store: &Store,
    current: &Mutex<Settings>,
    auto: &dyn AutostartPort,
    new: Settings,
) -> Result<Settings, String> {
    validate_settings(&new)?;
    let old = lock(current).clone();
    if new.launch_at_login != old.launch_at_login {
        auto.set(new.launch_at_login)
            .map_err(|e| format!("could not change launch at login: {e}"))?;
    }
    if let Err(e) = store.set_setting(SETTINGS_KEY, &new) {
        if new.launch_at_login != old.launch_at_login {
            let _ = auto.set(old.launch_at_login); // best-effort rollback
        }
        return Err(format!("{e:#}"));
    }
    *lock(current) = new;
    Ok(old)
}

/// At startup the OS registration wins over the stored flag.
pub fn reconcile_autostart(store: &Store, current: &Mutex<Settings>, auto: &dyn AutostartPort) {
    let Ok(os) = auto.is_enabled() else { return };
    let mut cur = lock(current);
    if cur.launch_at_login != os {
        cur.launch_at_login = os;
        let _ = store.set_setting(SETTINGS_KEY, &*cur);
    }
}

/// A hotkey service that started with `started_with` must switch to the current binding if the
/// user changed it in the meantime.
pub fn binding_to_reapply(started_with: &Binding, current: &Mutex<Settings>) -> Option<Binding> {
    let cur = lock(current).hotkey.clone();
    (cur != *started_with).then_some(cur)
}

pub struct AppState {
    pub store: Arc<Store>,
    pub models: Manager,
    pub stt: SttHandle,
    pub recorder: Arc<SwitchableRecorder>,
    pub hotkey: Arc<Mutex<Option<HotkeyService>>>,
    pub controller: ControllerHandle,
    pub settings: Arc<Mutex<Settings>>,
    pub hf: crate::models::hf::HfClient,
    pub hk_tx: Mutex<std::sync::mpsc::Sender<crate::hotkey::HotkeyEvent>>,
    pub hotkey_starting: Arc<AtomicBool>,
    /// A hotkey restart was already triggered for the current Accessibility grant.
    pub hotkey_grant_retried: AtomicBool,
    /// Last `dictation://state` payload (what `get_state` returns).
    pub dictation_state: Arc<Mutex<Value>>,
    pub paused: AtomicBool,
    pub pause_item: Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::default_binding;

    #[test]
    fn settings_default_values() {
        let s = Settings::default();
        assert_eq!(s.hotkey, default_binding());
        assert!(s.show_flowbar);
        assert_eq!(s.idle_unload_min, Some(10));
        assert!(!s.preload);
        assert!(!s.launch_at_login);
        assert!(!s.onboarded);
        assert_eq!(s.mic, None);
    }

    #[test]
    fn settings_serialize_camel_case() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert!(v.get("showFlowbar").is_some() && v.get("idleUnloadMin").is_some());
        assert!(v.get("launchAtLogin").is_some());
    }

    #[test]
    fn settings_tolerate_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"onboarded":true}"#).unwrap();
        assert!(s.onboarded && s.show_flowbar);
    }

    #[test]
    fn recommend_for_this_mac_is_parakeet_v2() {
        let hw = crate::models::hardware::Hardware {
            total_ram_gb: 16.0,
            cores: 8,
            arch: "aarch64".into(),
            apple_silicon: true,
            gpu_vendor: Some("Apple".into()),
        };
        let r = recommend_dto(&hw, "en-US");
        assert_eq!(r.id, "parakeet-v2");
        assert_eq!(r.hardware.cores, 8);
    }
}

#[cfg(test)]
mod settings_apply_tests {
    use super::*;
    use crate::hotkey::Key;

    struct FakeAuto {
        enabled: Mutex<bool>,
        fail: bool,
    }
    impl AutostartPort for FakeAuto {
        fn is_enabled(&self) -> Result<bool, String> {
            Ok(*self.enabled.lock().unwrap())
        }
        fn set(&self, on: bool) -> Result<(), String> {
            if self.fail {
                return Err("denied".into());
            }
            *self.enabled.lock().unwrap() = on;
            Ok(())
        }
    }

    fn fixture(fail: bool) -> (Store, Mutex<Settings>, FakeAuto) {
        (
            Store::open_in_memory().unwrap(),
            Mutex::new(Settings::default()),
            FakeAuto {
                enabled: Mutex::new(false),
                fail,
            },
        )
    }

    #[test]
    fn autostart_failure_returns_err_and_keeps_old_settings() {
        let (store, cur, auto) = fixture(true);
        let new = Settings {
            launch_at_login: true,
            show_flowbar: false,
            ..Settings::default()
        };
        let r = commit_settings(&store, &cur, &auto, new);
        assert!(r.is_err());
        assert_eq!(*cur.lock().unwrap(), Settings::default());
        assert_eq!(store.get_setting::<Settings>(SETTINGS_KEY).unwrap(), None);
    }

    #[test]
    fn autostart_success_persists_and_returns_old() {
        let (store, cur, auto) = fixture(false);
        let new = Settings {
            launch_at_login: true,
            ..Settings::default()
        };
        let old = commit_settings(&store, &cur, &auto, new.clone()).unwrap();
        assert_eq!(old, Settings::default());
        assert!(*auto.enabled.lock().unwrap());
        assert_eq!(*cur.lock().unwrap(), new);
        assert_eq!(
            store.get_setting::<Settings>(SETTINGS_KEY).unwrap(),
            Some(new)
        );
    }

    #[test]
    fn escape_binding_rejected() {
        let (store, cur, auto) = fixture(false);
        let new = Settings {
            hotkey: Binding {
                keys: vec![Key::ControlLeft, Key::Escape],
            },
            ..Settings::default()
        };
        assert!(commit_settings(&store, &cur, &auto, new).is_err());
        assert_eq!(*cur.lock().unwrap(), Settings::default());
    }

    #[test]
    fn startup_reconcile_trusts_os_state() {
        let (store, cur, auto) = fixture(false);
        cur.lock().unwrap().launch_at_login = true; // stored ON, OS says off
        reconcile_autostart(&store, &cur, &auto);
        assert!(!cur.lock().unwrap().launch_at_login);
        assert!(
            !store
                .get_setting::<Settings>(SETTINGS_KEY)
                .unwrap()
                .unwrap()
                .launch_at_login
        );
    }

    #[test]
    fn binding_change_before_service_start_is_applied() {
        let started_with = Binding {
            keys: vec![Key::Fn],
        };
        let cur = Mutex::new(Settings {
            hotkey: Binding {
                keys: vec![Key::AltRight],
            },
            ..Settings::default()
        });
        assert_eq!(
            binding_to_reapply(&started_with, &cur),
            Some(Binding {
                keys: vec![Key::AltRight]
            })
        );
        assert_eq!(
            binding_to_reapply(
                &Binding {
                    keys: vec![Key::AltRight]
                },
                &cur
            ),
            None
        );
    }
}
