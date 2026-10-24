//! Logging to stderr and to a file in the app's log folder. Release builds on Windows have no
//! console, so the file is the only log a user can attach to a bug report.

use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::sync::Mutex;

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub const LOG_FILE: &str = "uttr.log";
/// The previous launch's log, kept so a crash report can include the run that crashed.
pub const OLD_LOG_FILE: &str = "uttr.old.log";

/// Installs the global subscriber (stderr, plus `dir/uttr.log` when `dir` is given) and a panic
/// hook that records panics in the log. Safe to call more than once; later calls do nothing.
pub fn init(dir: Option<&Path>) {
    let file = dir.and_then(|d| match open_log(d) {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("could not open the log file in {}: {e}", d.display());
            None
        }
    });
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("uttr_lib=info"));
    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(io::stderr))
        .with(file.map(|f| fmt::layer().with_ansi(false).with_writer(Mutex::new(f))))
        .try_init()
        .is_ok();
    if installed {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!("panic: {info}");
            previous(info);
        }));
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            "uttr starting"
        );
    }
}

/// Creates `dir`, moves an existing log to [`OLD_LOG_FILE`] and opens a fresh one.
fn open_log(dir: &Path) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    let path = dir.join(LOG_FILE);
    if path.exists() {
        // Best effort: on failure the old log is simply overwritten.
        let _ = fs::rename(&path, dir.join(OLD_LOG_FILE));
    }
    File::create(path)
}

/// Opens `dir` in the system file manager.
pub fn reveal(dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let program = "xdg-open";
    // Waited on so no zombie is left (the launcher returns at once). Its exit status is ignored:
    // explorer.exe reports failure even when the folder opened.
    std::process::Command::new(program)
        .arg(dir)
        .status()
        .map(drop)
        .map_err(|e| format!("could not open {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_log_keeps_the_previous_run() {
        let dir = std::env::temp_dir().join(format!("uttr-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        io::Write::write_all(&mut open_log(&dir).unwrap(), b"first run").unwrap();
        io::Write::write_all(&mut open_log(&dir).unwrap(), b"second run").unwrap();

        assert_eq!(
            fs::read_to_string(dir.join(LOG_FILE)).unwrap(),
            "second run"
        );
        assert_eq!(
            fs::read_to_string(dir.join(OLD_LOG_FILE)).unwrap(),
            "first run"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
