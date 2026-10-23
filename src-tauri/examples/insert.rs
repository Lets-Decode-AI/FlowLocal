//! Manual check: `cargo run --example insert -- "héllo 👋 世界"`, then focus a
//! text field (e.g. TextEdit) within 3 s. The prior clipboard should survive.
use flowlocal_lib::insert::{ClipboardInserter, Inserter, frontmost_app_name};

fn main() {
    let text = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "héllo 👋 世界".into());
    println!("focus a text field; inserting in 3 s...");
    std::thread::sleep(std::time::Duration::from_secs(3));
    println!("frontmost app: {:?}", frontmost_app_name());
    let outcome = ClipboardInserter::system().insert(&text);
    println!("outcome: {outcome:?}");
}
