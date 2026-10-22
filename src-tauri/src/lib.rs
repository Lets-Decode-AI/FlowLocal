#![allow(dead_code)]

pub mod app_state;
pub mod audio;
pub mod commands;
pub mod dictation;
pub mod events;
pub mod hotkey;
pub mod insert;
pub mod models;
pub mod permissions;
mod postprocess;
pub mod stt;
pub mod store;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const FLOWBAR_W: f64 = 220.0;
const FLOWBAR_H: f64 = 56.0;
const FLOWBAR_BOTTOM_MARGIN: f64 = 48.0;

/// Show the main window, creating it lazily if it does not exist.
fn open_main_window(app: &AppHandle, route: Option<&str>) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("main") {
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    let mut b = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("FlowLocal")
        .inner_size(960.0, 640.0);
    if let Some(route) = route {
        // Hash route, applied before the page's own scripts run.
        b = b.initialization_script(format!("if (!location.hash) location.hash = '#{route}';"));
    }
    let w = b.build()?;
    // Closing the window destroys it; the app keeps running in the tray.
    let win = w.clone();
    w.on_window_event(move |e| {
        if let WindowEvent::CloseRequested { .. } = e {
            let _ = win.destroy();
        }
    });
    w.set_focus()?;
    Ok(())
}

/// Place the Flow Bar at the bottom-center of the primary monitor.
fn position_flowbar(app: &AppHandle) -> tauri::Result<()> {
    let Some(w) = app.get_webview_window("flowbar") else {
        return Ok(());
    };
    if let Some(m) = app.primary_monitor()? {
        let scale = m.scale_factor();
        let size = m.size();
        let pos = m.position();
        let x = pos.x as f64 + (size.width as f64 - FLOWBAR_W * scale) / 2.0;
        let y = pos.y as f64 + size.height as f64 - (FLOWBAR_H + FLOWBAR_BOTTOM_MARGIN) * scale;
        w.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))?;
    }
    Ok(())
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open FlowLocal", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause dictation", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &quit])?;
    if let Some(st) = app.try_state::<app_state::AppState>() {
        *st.pause_item.lock().unwrap_or_else(|p| p.into_inner()) = Some(pause.clone());
    }

    let mut builder = TrayIconBuilder::with_id("tray")
        .tooltip("FlowLocal")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Err(e) = open_main_window(app, None) {
                    eprintln!("failed to open main window: {e}");
                }
            }
            "pause" => {
                if let Some(st) = app.try_state::<app_state::AppState>() {
                    let paused = !st.paused.load(Ordering::SeqCst);
                    commands::apply_paused(&st, paused);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Opens the store, builds every subsystem and registers `AppState`.
fn init_backend(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use app_state::{AppState, SwitchableRecorder, SETTINGS_KEY};
    use hotkey::HotkeyService;

    let handle = app.handle().clone();
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let store = store::Store::open(&data_dir.join("flow.db"))?;
    let settings: app_state::Settings = store.get_setting(SETTINGS_KEY)?.unwrap_or_default();
    tracing::info!("store opened at {}", data_dir.display());

    // Emit bridge; also caches the latest dictation state for `get_state`.
    let state_cache = Arc::new(Mutex::new(json!({ "kind": "idle" })));
    let emit: Arc<dyn Fn(&str, Value) + Send + Sync> = {
        let h = handle.clone();
        let cache = state_cache.clone();
        Arc::new(move |name, value| {
            if name == events::STATE {
                *cache.lock().unwrap_or_else(|p| p.into_inner()) = value.clone();
            }
            let _ = h.emit(name, value);
        })
    };

    // Models. The keychain read happens off this thread (it can block on a prompt); the token is
    // shared by every clone of the client.
    let hf = models::hf::HfClient::new(None);
    {
        let hf = hf.clone();
        std::thread::spawn(move || hf.set_token(commands::stored_hf_token()));
    }
    let progress_handle = handle.clone();
    let models = models::Manager::new(
        data_dir.join("models"),
        store.clone(),
        models::catalog::Catalog::bundled(),
        hf.clone(),
        Arc::new(move |p| {
            let _ = progress_handle.emit(events::DL_PROGRESS, &p);
        }),
    );
    models.resume_incomplete_on_startup();

    // Speech worker.
    let stt = stt::SttWorker::spawn(Box::new(stt::SherpaEngine::new()), settings.idle_unload());
    if settings.preload {
        if let Some(m) = models.active() {
            let stt = stt.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = stt.preload(m).await {
                    tracing::warn!("model preload failed: {e}");
                }
            });
        }
    }
    tracing::info!("speech worker spawned");

    // Audio + hotkeys.
    let (level_tx, level_rx) = std::sync::mpsc::sync_channel::<f32>(64);
    let recorder = Arc::new(SwitchableRecorder::new(settings.mic.clone(), level_tx));
    let (hk_tx, hk_rx) = std::sync::mpsc::channel();
    let hotkey: Arc<Mutex<Option<HotkeyService>>> = Arc::default();
    let settings_arc = Arc::new(Mutex::new(settings));
    let hotkey_starting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    commands::spawn_hotkey_start(
        handle.clone(),
        hotkey.clone(),
        settings_arc.clone(),
        hotkey_starting.clone(),
        hk_tx.clone(),
        std::time::Duration::from_millis(2500), // let the webviews subscribe before the toast
    );
    app_state::reconcile_autostart(&store, &settings_arc, &app_state::PluginAutostart(handle.clone()));
    let settings = settings_arc.lock().unwrap_or_else(|p| p.into_inner()).clone();

    // Controller (needs a tokio runtime context).
    let store = Arc::new(store);
    let arm_slot = hotkey.clone();
    let controller = tauri::async_runtime::block_on(async {
        dictation::Controller::spawn(dictation::ControllerDeps {
            hotkeys: hk_rx,
            levels: Some(level_rx),
            recorder: recorder.clone(),
            stt: Arc::new(stt.clone()),
            models: Arc::new(models.clone()),
            inserter: Arc::new(insert::ClipboardInserter::system()),
            store: store.clone(),
            emit,
            arm_esc: Arc::new(move |armed| {
                if let Some(s) = arm_slot.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                    s.set_armed(armed);
                }
            }),
            app_name: Arc::new(insert::frontmost_app_name),
        })
    });
    tracing::info!("dictation controller started");

    let onboarded = settings.onboarded;
    let show_flowbar = settings.show_flowbar;
    app.manage(AppState {
        store,
        models,
        stt,
        recorder,
        hotkey,
        controller,
        settings: settings_arc,
        hf,
        hk_tx: Mutex::new(hk_tx),
        hotkey_starting,
        dictation_state: state_cache,
        paused: std::sync::atomic::AtomicBool::new(false),
        pause_item: Mutex::default(),
    });

    if !show_flowbar {
        commands::set_flowbar_visible(&handle, false);
    }
    if !onboarded {
        open_main_window(&handle, Some("/onboarding"))?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "flowlocal_lib=info".into())).try_init();
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::set_paused,
            commands::permissions_status,
            commands::open_permission_settings,
            commands::request_microphone,
            commands::catalog,
            commands::installed,
            commands::recommend,
            commands::hf_search,
            commands::hf_inspect,
            commands::model_install,
            commands::model_pause,
            commands::model_resume,
            commands::model_cancel,
            commands::model_delete,
            commands::model_set_active,
            commands::history_list,
            commands::history_delete,
            commands::settings_get,
            commands::settings_set,
            commands::hf_token_set,
            commands::dictionary_list,
            commands::dictionary_add,
            commands::dictionary_delete,
            commands::mic_list,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            init_backend(app)?;
            build_tray(app.handle())?;
            position_flowbar(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Keep running in the tray when all windows are closed.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert_eq!(2 + 2, 4);
    }
}
