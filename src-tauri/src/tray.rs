//! Tray "Active model" radio submenu: one check item per installed model, rebuilt whenever the
//! installed set or the active model changes.

use tauri::menu::{CheckMenuItem, Submenu};
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;

pub const MODEL_ITEM_PREFIX: &str = "model:";

/// Managed state holding the submenu so it can be rebuilt later.
pub struct ModelsMenu(pub Submenu<tauri::Wry>);

/// `(id, name, active)` for every installed model.
fn entries(st: &AppState) -> Vec<(String, String, bool)> {
    let active = st.models.active().map(|m| m.entry.id);
    st.models
        .installed()
        .into_iter()
        .map(|m| (m.entry.id.clone(), m.entry.name.clone(), active.as_deref() == Some(m.entry.id.as_str())))
        .collect()
}

pub fn menu_item_id(model_id: &str) -> String {
    format!("{MODEL_ITEM_PREFIX}{model_id}")
}

pub fn model_id_from_item(item_id: &str) -> Option<&str> {
    item_id.strip_prefix(MODEL_ITEM_PREFIX)
}

fn rebuild(app: &AppHandle) -> tauri::Result<()> {
    let (Some(menu), Some(st)) = (app.try_state::<ModelsMenu>(), app.try_state::<AppState>()) else {
        return Ok(());
    };
    let sub = &menu.0;
    for item in sub.items()? {
        sub.remove(&item)?;
    }
    let entries = entries(&st);
    if entries.is_empty() {
        let none = tauri::menu::MenuItem::with_id(app, "model:none", "No models installed", false, None::<&str>)?;
        sub.append(&none)?;
    }
    for (id, name, active) in entries {
        let item = CheckMenuItem::with_id(app, menu_item_id(&id), name, true, active, None::<&str>)?;
        sub.append(&item)?;
    }
    Ok(())
}

/// Rebuild the submenu on the main thread (menu mutation is not thread-safe on every platform).
pub fn refresh_models_menu(app: &AppHandle) {
    let h = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = rebuild(&h) {
            tracing::warn!("tray model menu refresh failed: {e}");
        }
    });
}

/// Tray click on a model: same path as the `model_set_active` command.
pub fn handle_model_click(app: &AppHandle, model_id: &str) {
    if let Some(st) = app.try_state::<AppState>() {
        if let Err(e) = st.models.set_active(model_id) {
            tracing::warn!("tray set_active failed: {e:#}");
        }
    }
    refresh_models_menu(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_ids_round_trip() {
        let id = menu_item_id("hf:owner/name");
        assert_eq!(model_id_from_item(&id), Some("hf:owner/name"));
        assert_eq!(model_id_from_item("open"), None);
    }
}
