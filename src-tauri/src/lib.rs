#![allow(dead_code)]

mod app_state;
pub mod audio;
mod commands;
mod dictation;
mod events;
mod hotkey;
mod insert;
pub mod models;
mod permissions;
mod postprocess;
pub mod stt;
pub mod store;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const FLOWBAR_W: f64 = 220.0;
const FLOWBAR_H: f64 = 56.0;
const FLOWBAR_BOTTOM_MARGIN: f64 = 48.0;

/// Show the main window, creating it lazily if it does not exist.
fn open_main_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("main") {
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    let w = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("FlowLocal")
        .inner_size(960.0, 640.0)
        .build()?;
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

    let mut builder = TrayIconBuilder::with_id("tray")
        .tooltip("FlowLocal")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Err(e) = open_main_window(app) {
                    eprintln!("failed to open main window: {e}");
                }
            }
            "pause" => { /* wired up in a later task */ }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
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
