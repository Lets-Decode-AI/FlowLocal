//! Tauri commands: the backend surface the React UI calls through `src/ipc.ts`.
//! Errors cross the boundary as strings.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager as _, State};

use crate::app_state::{
    AppState, InstalledModelDto, PluginAutostart, RecommendDto, Settings, binding_to_reapply,
    commit_settings, os_locale, recommend_dto,
};
use crate::events;
use crate::hotkey::{HotkeyError, HotkeyEvent, HotkeyService, default_binding, fallback_binding};
use crate::models::catalog::{Catalog, CatalogEntry};
use crate::models::hardware::Hardware;
use crate::models::hf::{HfRepoSummary, Inspection};
use crate::permissions::{self, PermKind, PermissionsStatus};
use crate::store::{DictEntry, Transcript};

const HISTORY_PAGE_SIZE: u32 = 50;
const KEYRING_SERVICE: &str = "flowlocal";
const KEYRING_USER: &str = "hf_token";

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

pub fn stored_hf_token() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .ok()?
        .get_password()
        .ok()
        .filter(|t| !t.is_empty())
}

/// Pause/resume dictation and keep the tray item's label in sync.
pub fn apply_paused(st: &AppState, paused: bool) {
    st.controller.set_paused(paused);
    st.paused.store(paused, Ordering::SeqCst);
    if let Some(item) = st
        .pause_item
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        let _ = item.set_text(if paused {
            "Resume dictation"
        } else {
            "Pause dictation"
        });
    }
}

pub fn set_flowbar_visible(app: &AppHandle, visible: bool) {
    if let Some(w) = app.get_webview_window("flowbar") {
        let _ = if visible { w.show() } else { w.hide() };
    }
}

pub fn toast(app: &AppHandle, level: &str, message: &str) {
    let _ = app.emit(
        events::TOAST,
        serde_json::json!({ "level": level, "message": message }),
    );
}

/// Starts the hotkey service on its own thread (the macOS tap can block ~300 ms) unless one is
/// running or being started. The thread installs the service and then re-reads the CURRENT
/// binding, so a change made while it was starting is not lost.
pub fn spawn_hotkey_start(
    app: AppHandle,
    slot: Arc<Mutex<Option<HotkeyService>>>,
    settings: Arc<Mutex<Settings>>,
    starting: Arc<AtomicBool>,
    tx: Sender<HotkeyEvent>,
    toast_delay: Duration,
) {
    if starting.swap(true, Ordering::SeqCst) {
        return;
    }
    if slot.lock().unwrap_or_else(|p| p.into_inner()).is_some() {
        starting.store(false, Ordering::SeqCst);
        return;
    }
    let spawned = std::thread::Builder::new().name("flowlocal-hotkey-start".into()).spawn({
        let starting = starting.clone();
        move || {
            let requested = settings.lock().unwrap_or_else(|p| p.into_inner()).hotkey.clone();
            let mut notice: Option<(&str, String)> = None;
            let started = match HotkeyService::start(requested.clone(), tx.clone()) {
                Ok(s) => Some(s),
                Err(HotkeyError::TapUnavailable) if cfg!(target_os = "macos") && requested == default_binding() => {
                    match HotkeyService::start(fallback_binding(), tx.clone()) {
                        Ok(s) => {
                            notice = Some((
                                "warning",
                                "Fn key unavailable \u{2014} using Right \u{2325}. Grant Accessibility to use Fn.".into(),
                            ));
                            Some(s)
                        }
                        Err(e) => {
                            notice = Some(("error", format!("Global hotkey unavailable: {e}")));
                            None
                        }
                    }
                }
                Err(HotkeyError::Unsupported) => {
                    notice = Some(("warning", "Global hotkey unavailable on Wayland".into()));
                    None
                }
                Err(e) => {
                    notice = Some(("error", format!("Global hotkey unavailable: {e}")));
                    None
                }
            };
            if let Some(svc) = started {
                tracing::info!("hotkey service started");
                let mut g = slot.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(b) = binding_to_reapply(&requested, &settings) {
                    svc.set_binding(b);
                }
                *g = Some(svc);
            } else {
                tracing::warn!("hotkey service not started");
            }
            starting.store(false, Ordering::SeqCst);
            if let Some((level, msg)) = notice {
                std::thread::sleep(toast_delay);
                toast(&app, level, &msg);
            }
        }
    });
    if spawned.is_err() {
        starting.store(false, Ordering::SeqCst);
    }
}

/// Persist `new` and apply every changed field live. Autostart (the fallible part) is applied
/// before anything is saved, so a failure leaves the stored settings untouched.
pub fn apply_settings(app: &AppHandle, st: &AppState, new: Settings) -> Result<(), String> {
    let old = commit_settings(
        &st.store,
        &st.settings,
        &PluginAutostart(app.clone()),
        new.clone(),
    )?;

    if new.hotkey != old.hotkey {
        let g = st.hotkey.lock().unwrap_or_else(|p| p.into_inner());
        match g.as_ref() {
            Some(h) => h.set_binding(new.hotkey.clone()),
            None => {
                drop(g);
                // No service yet (failed earlier, e.g. Accessibility was missing): try again.
                let tx = st.hk_tx.lock().unwrap_or_else(|p| p.into_inner()).clone();
                spawn_hotkey_start(
                    app.clone(),
                    st.hotkey.clone(),
                    st.settings.clone(),
                    st.hotkey_starting.clone(),
                    tx,
                    Duration::ZERO,
                );
            }
        }
    }
    if new.mic != old.mic {
        st.recorder.set_mic(new.mic.clone());
    }
    if new.idle_unload_min != old.idle_unload_min {
        st.stt.set_idle_unload(new.idle_unload());
    }
    if new.show_flowbar != old.show_flowbar {
        set_flowbar_visible(app, new.show_flowbar);
    }
    let _ = app.emit(events::SETTINGS_CHANGED, &new);
    Ok(())
}

// ---- dictation -----------------------------------------------------------------------------

#[tauri::command]
pub fn get_state(st: State<'_, AppState>) -> Value {
    st.dictation_state
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

#[tauri::command]
pub fn set_paused(st: State<'_, AppState>, paused: bool) {
    apply_paused(&st, paused);
}

// ---- permissions ---------------------------------------------------------------------------

#[tauri::command(async)]
pub fn permissions_status() -> PermissionsStatus {
    permissions::status()
}

#[tauri::command(async)]
pub fn open_permission_settings(kind: PermKind) -> Result<(), String> {
    permissions::open_settings(kind)
}

#[tauri::command]
pub fn request_microphone() {
    permissions::request_microphone();
}

// ---- models --------------------------------------------------------------------------------

#[tauri::command]
pub fn catalog() -> Vec<CatalogEntry> {
    Catalog::bundled().entries().to_vec()
}

#[tauri::command(async)]
pub fn installed(st: State<'_, AppState>) -> Vec<InstalledModelDto> {
    let active = st.models.active().map(|m| m.entry.id);
    st.models
        .installed()
        .into_iter()
        .map(|m| InstalledModelDto {
            disk_bytes: st.models.disk_usage(&m.entry.id),
            active: active.as_deref() == Some(m.entry.id.as_str()),
            entry: m.entry,
        })
        .collect()
}

#[tauri::command(async)]
pub fn recommend() -> RecommendDto {
    recommend_dto(&Hardware::detect(), &os_locale())
}

#[tauri::command]
pub async fn hf_search(st: State<'_, AppState>, q: String) -> Result<Vec<HfRepoSummary>, String> {
    st.hf.clone().search(&q).await.map_err(err)
}

#[tauri::command]
pub async fn hf_inspect(st: State<'_, AppState>, repo: String) -> Result<Inspection, String> {
    st.hf.clone().inspect(&repo).await.map_err(err)
}

/// Install a catalog model by `id`, or a custom (HF-inspected) `entry`.
#[tauri::command]
pub fn model_install(
    st: State<'_, AppState>,
    id: Option<String>,
    entry: Option<CatalogEntry>,
) -> Result<(), String> {
    let entry = match (entry, id) {
        (Some(e), _) => e,
        (None, Some(id)) => Catalog::bundled()
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("unknown model {id}"))?,
        (None, None) => return Err("model_install needs an id or an entry".into()),
    };
    st.models.install(entry);
    Ok(())
}

#[tauri::command]
pub async fn model_pause(st: State<'_, AppState>, id: String) -> Result<(), String> {
    st.models.pause(&id).await;
    Ok(())
}

#[tauri::command]
pub fn model_resume(st: State<'_, AppState>, id: String) {
    st.models.resume(&id);
}

#[tauri::command]
pub async fn model_cancel(st: State<'_, AppState>, id: String) -> Result<(), String> {
    st.models.cancel(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn model_delete(
    app: AppHandle,
    st: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let r = st.models.delete(&id).await.map_err(err);
    crate::tray::refresh_models_menu(&app);
    r
}

#[tauri::command]
pub fn model_set_active(app: AppHandle, st: State<'_, AppState>, id: String) -> Result<(), String> {
    let r = st.models.set_active(&id).map_err(err);
    crate::tray::refresh_models_menu(&app);
    r
}

// ---- history / dictionary ------------------------------------------------------------------

#[tauri::command(async)]
pub fn history_list(
    st: State<'_, AppState>,
    page: u32,
    query: Option<String>,
) -> Result<Vec<Transcript>, String> {
    st.store
        .list_transcripts(page, HISTORY_PAGE_SIZE, query.as_deref())
        .map_err(err)
}

#[tauri::command(async)]
pub fn history_delete(st: State<'_, AppState>, id: i64) -> Result<(), String> {
    st.store.delete_transcript(id).map_err(err)
}

#[tauri::command(async)]
pub fn dictionary_list(st: State<'_, AppState>) -> Result<Vec<DictEntry>, String> {
    st.store.dictionary_list().map_err(err)
}

#[tauri::command(async)]
pub fn dictionary_add(
    st: State<'_, AppState>,
    term: String,
    replacement: Option<String>,
) -> Result<i64, String> {
    st.store
        .dictionary_add(&term, replacement.as_deref())
        .map_err(err)
}

#[tauri::command(async)]
pub fn dictionary_delete(st: State<'_, AppState>, id: i64) -> Result<(), String> {
    st.store.dictionary_delete(id).map_err(err)
}

// ---- settings ------------------------------------------------------------------------------

#[tauri::command]
pub fn settings_get(st: State<'_, AppState>) -> Settings {
    st.settings
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

#[tauri::command]
pub fn settings_set(
    app: AppHandle,
    st: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    apply_settings(&app, &st, settings)
}

/// `None` removes the stored token. Takes effect immediately (the client is shared with the
/// model manager). Runs off the main thread: keychain access can block on a prompt.
#[tauri::command(async)]
pub fn hf_token_set(st: State<'_, AppState>, token: Option<String>) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(err)?;
    let token = token.filter(|t| !t.is_empty());
    match &token {
        Some(t) => entry.set_password(t).map_err(err)?,
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(err(e)),
        },
    }
    st.hf.set_token(token);
    Ok(())
}

#[tauri::command(async)]
pub fn mic_list() -> Vec<String> {
    crate::audio::list_devices()
}
