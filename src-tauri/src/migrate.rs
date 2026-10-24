//! One-time move of app data from the app's old name (FlowLocal, bundle id
//! `com.flowlocal.app`) to the current data folder, so upgrading keeps the
//! downloaded models, history and settings.

use std::io;
use std::path::Path;

/// Data folder name used by releases before the rename.
pub const LEGACY_DIR_NAME: &str = "com.flowlocal.app";

/// Moves `<parent of data_dir>/com.flowlocal.app` to `data_dir` when the new
/// folder does not exist yet. Returns whether anything was moved.
pub fn move_legacy_data(data_dir: &Path) -> io::Result<bool> {
    let Some(parent) = data_dir.parent() else {
        return Ok(false);
    };
    let legacy = parent.join(LEGACY_DIR_NAME);
    if data_dir.exists() || !legacy.is_dir() {
        return Ok(false);
    }
    std::fs::rename(&legacy, data_dir)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_legacy_folder_when_new_one_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(LEGACY_DIR_NAME);
        std::fs::create_dir_all(legacy.join("models")).unwrap();
        std::fs::write(legacy.join("flow.db"), b"db").unwrap();
        let new = root.path().join("ai.letsdecode.uttr");

        assert!(move_legacy_data(&new).unwrap());
        assert_eq!(std::fs::read(new.join("flow.db")).unwrap(), b"db");
        assert!(new.join("models").is_dir());
        assert!(!legacy.exists());
    }

    #[test]
    fn leaves_existing_new_folder_alone() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(LEGACY_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        let new = root.path().join("ai.letsdecode.uttr");
        std::fs::create_dir_all(&new).unwrap();

        assert!(!move_legacy_data(&new).unwrap());
        assert!(legacy.exists());
    }

    #[test]
    fn does_nothing_without_legacy_data() {
        let root = tempfile::tempdir().unwrap();
        let new = root.path().join("ai.letsdecode.uttr");
        assert!(!move_legacy_data(&new).unwrap());
        assert!(!new.exists());
    }
}
