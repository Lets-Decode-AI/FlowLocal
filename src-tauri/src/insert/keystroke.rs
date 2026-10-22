use anyhow::{Result, anyhow};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

pub trait Keystroke {
    fn paste(&self) -> Result<()>;
}

/// Synthetic paste via enigo: Cmd+V on macOS, Ctrl+V elsewhere. An `Enigo`
/// is created per call (cheap, avoids holding non-Send OS handles).
pub struct EnigoKeystroke;

#[cfg(target_os = "macos")]
const MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const MODIFIER: Key = Key::Control;

impl Keystroke for EnigoKeystroke {
    fn paste(&self) -> Result<()> {
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow!("enigo init: {e}"))?;
        enigo
            .key(MODIFIER, Direction::Press)
            .map_err(|e| anyhow!("modifier press: {e}"))?;
        let clicked = enigo.key(Key::Unicode('v'), Direction::Click);
        // Always release the modifier, even if the V click failed.
        let released = enigo.key(MODIFIER, Direction::Release);
        clicked.map_err(|e| anyhow!("v click: {e}"))?;
        released.map_err(|e| anyhow!("modifier release: {e}"))?;
        Ok(())
    }
}
