//! Text insertion at the user's cursor in any app: snapshot clipboard, set text,
//! synthetic paste, restore the original clipboard shortly after.

pub mod clipboard;
pub mod frontmost;
pub mod keystroke;

use std::sync::Mutex;
use std::time::Duration;

use crate::postprocess::InsertContext;
use clipboard::{Clipboard, ClipboardSnapshot, SystemClipboard};
use keystroke::{EnigoKeystroke, Keystroke};

pub use frontmost::{detect_wayland, frontmost_app_name, is_wayland, secure_input_active};

/// Delay between the synthetic paste and restoring the previous clipboard.
pub const RESTORE_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq)]
pub enum InsertOutcome {
    Pasted,
    /// Text is on the clipboard but was not pasted; the string says why.
    CopiedOnly(String),
}

pub trait Inserter: Send + Sync {
    /// Blocking: sleeps the restore delay on the calling thread.
    fn insert(&self, text: &str) -> InsertOutcome;
    fn context(&self) -> InsertContext;
}

/// Clipboard-based inserter. The clipboard sits behind a `Mutex` so `insert`
/// takes `&self` and concurrent inserts are serialized.
pub struct ClipboardInserter<C: Clipboard, K: Keystroke> {
    clipboard: Mutex<C>,
    keystroke: K,
    pub restore_delay: Duration,
    secure_input: Box<dyn Fn() -> bool + Send + Sync>,
}

fn norm_nl(s: &str) -> String {
    s.replace("\r\n", "\n")
}

impl<C: Clipboard, K: Keystroke> ClipboardInserter<C, K> {
    pub fn new(clipboard: C, keystroke: K) -> Self {
        Self {
            clipboard: Mutex::new(clipboard),
            keystroke,
            restore_delay: RESTORE_DELAY,
            secure_input: Box::new(secure_input_active),
        }
    }

    pub fn with_restore_delay(mut self, d: Duration) -> Self {
        self.restore_delay = d;
        self
    }

    pub fn with_secure_input(mut self, f: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.secure_input = Box::new(f);
        self
    }
}

impl ClipboardInserter<SystemClipboard, EnigoKeystroke> {
    pub fn system() -> Self {
        Self::new(SystemClipboard::new(), EnigoKeystroke)
    }
}

impl<C: Clipboard + Send, K: Keystroke + Send + Sync> Inserter for ClipboardInserter<C, K> {
    fn insert(&self, text: &str) -> InsertOutcome {
        let mut cb = self.clipboard.lock().unwrap_or_else(|e| e.into_inner());
        let snapshot = cb.snapshot();
        if let Err(e) = cb.set_text(text) {
            return InsertOutcome::CopiedOnly(format!("clipboard unavailable: {e}"));
        }
        if (self.secure_input)() {
            return InsertOutcome::CopiedOnly("secure input is active".into());
        }
        if let Err(e) = self.keystroke.paste() {
            // Intentional: the dictated text stays on the clipboard so the
            // user can paste it manually.
            return InsertOutcome::CopiedOnly(format!("paste failed: {e}"));
        }
        std::thread::sleep(self.restore_delay);
        // Only restore if nothing else touched the clipboard since we set it
        // (compared with \r\n normalized: Windows may rewrite newlines).
        // Restoring Empty clears the clipboard; file/HTML/RTF contents are not
        // preserved in Part 1 (only text or image).
        match cb.snapshot() {
            ClipboardSnapshot::Text(t) if norm_nl(&t) == norm_nl(text) => {
                if let Err(e) = cb.restore(snapshot) {
                    tracing::warn!("clipboard restore failed: {e}");
                }
            }
            _ => tracing::debug!("clipboard changed during paste; skipping restore"),
        }
        InsertOutcome::Pasted
    }

    fn context(&self) -> InsertContext {
        // Best-effort accessibility read of the previous character is deferred.
        InsertContext { prev_char: None }
    }
}

#[cfg(test)]
mod tests {
    use super::clipboard::{Clipboard, ClipboardSnapshot};
    use super::keystroke::Keystroke;
    use super::*;
    use anyhow::{Result, anyhow};
    use arboard::ImageData;
    use std::borrow::Cow;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct State {
        content: ClipboardSnapshot,
        restored: Vec<ClipboardSnapshot>,
        pasted: usize,
        /// Simulates the user copying something during the restore delay.
        change_on_paste: Option<String>,
    }

    #[derive(Clone, Default)]
    struct FakeClipboard(Arc<Mutex<State>>);

    impl Clipboard for FakeClipboard {
        fn snapshot(&mut self) -> ClipboardSnapshot {
            self.0.lock().unwrap().content.clone()
        }
        fn set_text(&mut self, s: &str) -> Result<()> {
            self.0.lock().unwrap().content = ClipboardSnapshot::Text(s.to_string());
            Ok(())
        }
        fn restore(&mut self, s: ClipboardSnapshot) -> Result<()> {
            let mut st = self.0.lock().unwrap();
            st.restored.push(s.clone());
            st.content = s;
            Ok(())
        }
    }

    struct FakeKeys {
        state: Arc<Mutex<State>>,
        fail: bool,
    }

    impl Keystroke for FakeKeys {
        fn paste(&self) -> Result<()> {
            if self.fail {
                return Err(anyhow!("no accessibility"));
            }
            let mut st = self.state.lock().unwrap();
            st.pasted += 1;
            if let Some(t) = st.change_on_paste.take() {
                st.content = ClipboardSnapshot::Text(t);
            }
            Ok(())
        }
    }

    fn rig(
        initial: ClipboardSnapshot,
        fail: bool,
        secure: bool,
    ) -> (
        ClipboardInserter<FakeClipboard, FakeKeys>,
        Arc<Mutex<State>>,
    ) {
        let cb = FakeClipboard::default();
        cb.0.lock().unwrap().content = initial;
        let state = cb.0.clone();
        let keys = FakeKeys {
            state: state.clone(),
            fail,
        };
        let ins = ClipboardInserter::new(cb, keys)
            .with_restore_delay(Duration::ZERO)
            .with_secure_input(move || secure);
        (ins, state)
    }

    fn img() -> ImageData<'static> {
        ImageData {
            width: 1,
            height: 1,
            bytes: Cow::Owned(vec![1, 2, 3, 4]),
        }
    }

    #[test]
    fn pastes_then_restores_text_clipboard() {
        let (ins, st) = rig(ClipboardSnapshot::Text("old".into()), false, false);
        assert_eq!(ins.insert("héllo"), InsertOutcome::Pasted);
        let st = st.lock().unwrap();
        assert_eq!(st.pasted, 1);
        assert_eq!(st.restored.len(), 1);
        assert!(matches!(&st.content, ClipboardSnapshot::Text(t) if t == "old"));
    }

    #[test]
    fn restores_image_clipboard() {
        let (ins, st) = rig(ClipboardSnapshot::Image(img()), false, false);
        assert_eq!(ins.insert("x"), InsertOutcome::Pasted);
        let st = st.lock().unwrap();
        match &st.restored[..] {
            [ClipboardSnapshot::Image(i)] => {
                assert_eq!(
                    (i.width, i.height, i.bytes.to_vec()),
                    (1, 1, vec![1, 2, 3, 4])
                )
            }
            other => panic!("unexpected restore: {} entries", other.len()),
        }
    }

    #[test]
    fn empty_clipboard_restored_as_empty() {
        let (ins, st) = rig(ClipboardSnapshot::Empty, false, false);
        assert_eq!(ins.insert("x"), InsertOutcome::Pasted);
        let st = st.lock().unwrap();
        assert!(matches!(&st.restored[..], [ClipboardSnapshot::Empty]));
        assert!(matches!(st.content, ClipboardSnapshot::Empty));
    }

    #[test]
    fn paste_error_returns_copied_only_and_leaves_text() {
        let (ins, st) = rig(ClipboardSnapshot::Text("old".into()), true, false);
        assert!(matches!(ins.insert("new"), InsertOutcome::CopiedOnly(_)));
        let st = st.lock().unwrap();
        assert!(st.restored.is_empty());
        assert!(matches!(&st.content, ClipboardSnapshot::Text(t) if t == "new"));
    }

    #[test]
    fn secure_input_returns_copied_only() {
        let (ins, st) = rig(ClipboardSnapshot::Text("old".into()), false, true);
        assert!(matches!(ins.insert("new"), InsertOutcome::CopiedOnly(_)));
        let st = st.lock().unwrap();
        assert_eq!(st.pasted, 0);
        assert!(st.restored.is_empty());
        assert!(matches!(&st.content, ClipboardSnapshot::Text(t) if t == "new"));
    }

    #[test]
    fn does_not_clobber_clipboard_changed_by_user() {
        let (ins, st) = rig(ClipboardSnapshot::Text("old".into()), false, false);
        st.lock().unwrap().change_on_paste = Some("user copied".into());
        assert_eq!(ins.insert("new"), InsertOutcome::Pasted);
        let st = st.lock().unwrap();
        assert!(st.restored.is_empty());
        assert!(matches!(&st.content, ClipboardSnapshot::Text(t) if t == "user copied"));
    }

    #[test]
    fn newline_normalized_clipboard_still_restores() {
        let (ins, st) = rig(ClipboardSnapshot::Text("old".into()), false, false);
        st.lock().unwrap().change_on_paste = Some("a\r\nb".into());
        ins.insert("a\nb");
        assert_eq!(st.lock().unwrap().restored.len(), 1);
    }

    #[test]
    fn system_inserter_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ClipboardInserter<SystemClipboard, EnigoKeystroke>>();
    }

    #[test]
    fn context_is_unknown_in_part1() {
        let (ins, _) = rig(ClipboardSnapshot::Empty, false, false);
        assert_eq!(ins.context().prev_char, None);
    }
}
