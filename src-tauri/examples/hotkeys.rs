//! Manual check: `cargo run --example hotkeys`, then hold / double-tap the
//! default hotkey (Fn on macOS) and press Esc. Needs Accessibility granted to
//! the terminal; without it this prints a clean TapUnavailable error.
use flowlocal_lib::hotkey::{HotkeyService, default_binding};

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();
    let binding = default_binding();
    println!("binding: {binding:?}");
    let svc = match HotkeyService::start(binding, tx) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hotkey service failed to start: {e}");
            std::process::exit(1);
        }
    };
    svc.set_armed(true);
    println!("listening (Esc = Cancel); Ctrl-C to quit");
    for ev in rx {
        println!("{ev:?}");
    }
}
