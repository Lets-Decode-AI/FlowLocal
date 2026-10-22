use anyhow::Result;
use arboard::ImageData;

#[derive(Clone, Debug, Default)]
pub enum ClipboardSnapshot {
    #[default]
    Empty,
    Text(String),
    Image(ImageData<'static>),
}

pub trait Clipboard {
    fn snapshot(&mut self) -> ClipboardSnapshot;
    fn set_text(&mut self, s: &str) -> Result<()>;
    fn restore(&mut self, s: ClipboardSnapshot) -> Result<()>;
}

/// Real clipboard backed by `arboard`. A fresh `arboard::Clipboard` is opened
/// per call because it is not `Send` on every platform; this type stays a
/// zero-sized `Send + Sync` handle. (Linux/X11: arboard serves selection data
/// from a background thread, so content set here remains available after drop
/// only while the process runs — fine for a resident tray app.)
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        let Ok(mut cb) = arboard::Clipboard::new() else {
            return ClipboardSnapshot::Empty;
        };
        if let Ok(t) = cb.get_text() {
            return ClipboardSnapshot::Text(t);
        }
        if let Ok(img) = cb.get_image() {
            return ClipboardSnapshot::Image(img.to_owned_img());
        }
        ClipboardSnapshot::Empty
    }

    fn set_text(&mut self, s: &str) -> Result<()> {
        arboard::Clipboard::new()?.set_text(s.to_owned())?;
        Ok(())
    }

    fn restore(&mut self, s: ClipboardSnapshot) -> Result<()> {
        let mut cb = arboard::Clipboard::new()?;
        match s {
            ClipboardSnapshot::Empty => cb.clear()?,
            ClipboardSnapshot::Text(t) => cb.set_text(t)?,
            ClipboardSnapshot::Image(i) => cb.set_image(i)?,
        }
        Ok(())
    }
}
