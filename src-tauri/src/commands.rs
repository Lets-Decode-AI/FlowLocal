//! Tauri commands: the backend surface the React UI calls through `src/ipc.ts`.
//! Errors cross the boundary as strings.

use std::sync::atomic::Ordering;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager as _, State};

use crate::app_state::{os_locale, recommend_dto, AppState, InstalledModelDto, RecommendDto, Settings, SETTINGS_KEY};
use crate::events;
use crate::models::catalog::{Catalog, CatalogEntry};
use crate::models::hardware::Hardware;
use crate::models::hf::{HfClient, HfRepoSummary, Inspection};
use crate::permissions::{self, PermKind, PermissionsStatus};
use crate::store::{DictEntry, Transcript};

const HISTORY_PAGE_SIZE: u32 = 50;
const KEYRING_SERVICE: &str = "flowlocal";
const KEYRING_USER: &str = "hf_token";

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

pub fn stored_hf_token() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?.get_password().ok().filter(|t| !t.is_empty())
}

/// Pause/resume dictation and keep the tray item's label in sync.
pub fn apply_paused(st: &AppState, paused: bool) {
    st.controller.set_paused(paused);
    st.paused.store(paused, Ordering::SeqCst);
    if let Some(item) = st.pause_item.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = item.set_text(if paused { "Resume dictation" } else { "Pause dictation" });
    }
}

pub fn set_flowbar_visible(app: &AppHandle, visible: bool) {
    if let Some(w) = app.get_webview_window("flowbar") {
        let _ = if visible { w.show() } else { w.hide() };
    }
}

/// Persist `new` and apply every changed field live.
pub fn apply_settings(app: &AppHandle, st: &AppState, new: Settings) -> Result<(), String> {
    if new.hotkey.keys.is_empty() {
        return Err("hotkey must contain at least one key".into());
    }
    let old = st.settings.lock().unwrap_or_else(|p| p.into_inner()).clone();
    st.store.set_setting(SETTINGS_KEY, &new).map_err(err)?;

    if new.hotkey != old.hotkey {
        if let Some(h) = st.hotkey.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            h.set_binding(new.hotkey.clone());
        }
    }
    if new.mic != old.mic {
        st.recorder.set_mic(new.mic.clone());
    }
    if new.idle_unload_min != old.idle_unload_min {
        st.stt.set_idle_unload(new.idle_unload());
    }
    if new.launch_at_login != old.launch_at_login {
        use tauri_plugin_autostart::ManagerExt;
        let auto = app.autolaunch();
        let r = if new.launch_at_login { auto.enable() } else { auto.disable() };
        if let Err(e) = r {
            tracing::warn!("autostart change failed: {e}");
        }
    }
    if new.show_flowbar != old.show_flowbar {
        set_flowbar_visible(app, new.show_flowbar);
    }

    *st.settings.lock().unwrap_or_else(|p| p.into_inner()) = new.clone();
    let _ = app.emit(events::SETTINGS_CHANGED, &new);
    Ok(())
}

// ---- dictation -----------------------------------------------------------------------------

#[tauri::command]
pub fn get_state(st: State<'_, AppState>) -> Value {
    st.dictation_state.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[tauri::command]
pub fn set_paused(st: State<'_, AppState>, paused: bool) {
    apply_paused(&st, paused);
}

// ---- permissions ---------------------------------------------------------------------------

#[tauri::command]
pub fn permissions_status() -> PermissionsStatus {
    permissions::status()
}

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
pub fn recommend() -> RecommendDto {
    recommend_dto(&Hardware::detect(), &os_locale())
}

#[tauri::command]
pub async fn hf_search(q: String) -> Result<Vec<HfRepoSummary>, String> {
    HfClient::new(stored_hf_token()).search(&q).await.map_err(err)
}

#[tauri::command]
pub async fn hf_inspect(repo: String) -> Result<Inspection, String> {
    HfClient::new(stored_hf_token()).inspect(&repo).await.map_err(err)
}

/// Install a catalog model by `id`, or a custom (HF-inspected) `entry`.
#[tauri::command]
pub fn model_install(st: State<'_, AppState>, id: Option<String>, entry: Option<CatalogEntry>) -> Result<(), String> {
    let entry = match (entry, id) {
        (Some(e), _) => e,
        (None, Some(id)) => Catalog::bundled().get(&id).cloned().ok_or_else(|| format!("unknown model {id}"))?,
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
pub async fn model_delete(st: State<'_, AppState>, id: String) -> Result<(), String> {
    st.models.delete(&id).await.map_err(err)
}

#[tauri::command]
pub fn model_set_active(st: State<'_, AppState>, id: String) -> Result<(), String> {
    st.models.set_active(&id).map_err(err)
}

// ---- history / dictionary ------------------------------------------------------------------

#[tauri::command]
pub fn history_list(st: State<'_, AppState>, page: u32, query: Option<String>) -> Result<Vec<Transcript>, String> {
    st.store.list_transcripts(page, HISTORY_PAGE_SIZE, query.as_deref()).map_err(err)
}

#[tauri::command]
pub fn history_delete(st: State<'_, AppState>, id: i64) -> Result<(), String> {
    st.store.delete_transcript(id).map_err(err)
}

#[tauri::command]
pub fn dictionary_list(st: State<'_, AppState>) -> Result<Vec<DictEntry>, String> {
    st.store.dictionary_list().map_err(err)
}

#[tauri::command]
pub fn dictionary_add(st: State<'_, AppState>, term: String, replacement: Option<String>) -> Result<i64, String> {
    st.store.dictionary_add(&term, replacement.as_deref()).map_err(err)
}

#[tauri::command]
pub fn dictionary_delete(st: State<'_, AppState>, id: i64) -> Result<(), String> {
    st.store.dictionary_delete(id).map_err(err)
}

// ---- settings ------------------------------------------------------------------------------

#[tauri::command]
pub fn settings_get(st: State<'_, AppState>) -> Settings {
    st.settings.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[tauri::command]
pub fn settings_set(app: AppHandle, st: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    apply_settings(&app, &st, settings)
}

/// `None` removes the stored token. Takes effect for searches immediately and for downloads
/// after the next app start.
#[tauri::command]
pub fn hf_token_set(token: Option<String>) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(err)?;
    match token.filter(|t| !t.is_empty()) {
        Some(t) => entry.set_password(&t).map_err(err),
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(err(e)),
        },
    }
}

#[tauri::command]
pub fn mic_list() -> Vec<String> {
    crate::audio::list_devices()
}
