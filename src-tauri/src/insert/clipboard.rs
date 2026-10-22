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

type Job = Box<dyn FnOnce(&mut Option<arboard::Clipboard>) + Send>;

/// Real clipboard backed by `arboard`. All operations run on ONE dedicated
/// thread that owns a single long-lived `arboard::Clipboard`. This matters on
/// X11: dropping the last handle destroys the selection owner, so the text we
/// set would vanish before the target app pastes it. The thread also sidesteps
/// `arboard::Clipboard` being `!Send` on some platforms, keeping this type
/// `Send + Sync`. The thread exits when the `SystemClipboard` is dropped.
pub struct SystemClipboard {
    tx: std::sync::mpsc::Sender<Job>,
}

impl SystemClipboard {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("clipboard".into())
            .spawn(move || {
                let mut cb: Option<arboard::Clipboard> = None;
                while let Ok(job) = rx.recv() {
                    if cb.is_none() {
                        cb = arboard::Clipboard::new().ok();
                    }
                    job(&mut cb);
                }
            })
            .expect("spawn clipboard thread");
        Self { tx }
    }

    fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut arboard::Clipboard) -> R + Send + 'static,
    ) -> Result<R> {
        let (rtx, rrx) = std::sync::mpsc::channel();
        let job: Job = Box::new(move |cb| {
            let _ = rtx.send(cb.as_mut().map(f));
        });
        self.tx
            .send(job)
            .map_err(|_| anyhow::anyhow!("clipboard thread gone"))?;
        rrx.recv()
            .map_err(|_| anyhow::anyhow!("clipboard thread gone"))?
            .ok_or_else(|| anyhow::anyhow!("clipboard unavailable"))
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard for SystemClipboard {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        self.run(|cb| {
            if let Ok(t) = cb.get_text() {
                return ClipboardSnapshot::Text(t);
            }
            if let Ok(img) = cb.get_image() {
                return ClipboardSnapshot::Image(img.to_owned_img());
            }
            ClipboardSnapshot::Empty
        })
        .unwrap_or_default()
    }

    fn set_text(&mut self, s: &str) -> Result<()> {
        let s = s.to_owned();
        self.run(move |cb| cb.set_text(s))??;
        Ok(())
    }

    fn restore(&mut self, s: ClipboardSnapshot) -> Result<()> {
        // Empty clears the clipboard. Only text/image are preserved; file,
        // HTML and RTF contents are not (Part 1).
        self.run(move |cb| match s {
            ClipboardSnapshot::Empty => cb.clear(),
            ClipboardSnapshot::Text(t) => cb.set_text(t),
            ClipboardSnapshot::Image(i) => cb.set_image(i),
        })??;
        Ok(())
    }
}
