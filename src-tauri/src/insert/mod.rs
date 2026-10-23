//! Text insertion at the user's cursor in any app: snapshot clipboard, set text,
//! synthetic paste, restore the original clipboard shortly after.

pub mod clipboard;
pub mod frontmost;
pub mod keystroke;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::postprocess::InsertContext;
use clipboard::{Clipboard, ClipboardSnapshot, SystemClipboard};
pub use keystroke::Runner;
use keystroke::{EnigoKeystroke, Keystroke, MainThreadKeystroke};

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
    app_name: Box<dyn Fn() -> Option<String> + Send + Sync>,
    clock: Box<dyn Fn() -> Instant + Send + Sync>,
    last: Mutex<Option<LastInsert>>,
}

/// How long after an insertion the cursor is assumed to still sit right behind it.
pub const CONTEXT_WINDOW: Duration = Duration::from_secs(60);

/// The last text we pasted: where (frontmost app), its final character and when.
struct LastInsert {
    app: Option<String>,
    ch: char,
    at: Instant,
}

/// The character before the cursor, if we pasted into the same app moments ago.
fn prev_char_after(last: Option<&LastInsert>, app: &Option<String>, now: Instant) -> Option<char> {
    last.filter(|l| l.app == *app && now.saturating_duration_since(l.at) <= CONTEXT_WINDOW)
        .map(|l| l.ch)
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
            app_name: Box::new(frontmost_app_name),
            clock: Box::new(Instant::now),
            last: Mutex::new(None),
        }
    }

    pub fn with_app_name(mut self, f: impl Fn() -> Option<String> + Send + Sync + 'static) -> Self {
        self.app_name = Box::new(f);
        self
    }

    pub fn with_clock(mut self, f: impl Fn() -> Instant + Send + Sync + 'static) -> Self {
        self.clock = Box::new(f);
        self
    }

    fn remember(&self, text: &str) {
        let last = text.chars().last().map(|ch| LastInsert {
            app: (self.app_name)(),
            ch,
            at: (self.clock)(),
        });
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = last;
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

impl ClipboardInserter<SystemClipboard, MainThreadKeystroke<EnigoKeystroke>> {
    /// `main_thread` runs the synthetic keystroke on the app's main thread.
    pub fn system(main_thread: Runner) -> Self {
        Self::new(
            SystemClipboard::new(),
            MainThreadKeystroke::new(EnigoKeystroke, main_thread),
        )
    }
}

impl<C: Clipboard + Send, K: Keystroke + Send + Sync> Inserter for ClipboardInserter<C, K> {
    fn insert(&self, text: &str) -> InsertOutcome {
        // Whatever happens, the cursor context is only trusted after a successful paste.
        self.last.lock().unwrap_or_else(|e| e.into_inner()).take();
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
        self.remember(text);
        std::thread::sleep(self.restore_delay);
        // Only restore if nothing else touched the clipboard since we set it
        // (compared with \r\n normalized: Windows may rewrite newlines).
        // An Empty snapshot means "empty OR unreadable" (files, HTML, RTF, ...): clearing the
        // clipboard then would destroy the user's data, so the restore is skipped instead.
        // Rich formatting of a text clipboard comes back as plain text.
        match cb.snapshot() {
            ClipboardSnapshot::Text(t) if norm_nl(&t) == norm_nl(text) => {
                if matches!(snapshot, ClipboardSnapshot::Empty) {
                    tracing::debug!("clipboard snapshot was empty/unknown; skipping restore");
                } else if let Err(e) = cb.restore(snapshot) {
                    tracing::warn!("clipboard restore failed: {e}");
                }
            }
            _ => tracing::debug!("clipboard changed during paste; skipping restore"),
        }
        InsertOutcome::Pasted
    }

    fn context(&self) -> InsertContext {
        // No accessibility read of the real caret context: assume the cursor still sits right
        // behind our last paste if it went into the same app within CONTEXT_WINDOW.
        let app = (self.app_name)();
        let last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        InsertContext {
            prev_char: prev_char_after(last.as_ref(), &app, (self.clock)()),
        }
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
    fn empty_snapshot_is_never_restored_so_the_clipboard_is_not_wiped() {
        // "Empty" also means "could not read it" (files, HTML, RTF...): clearing would destroy it.
        let (ins, st) = rig(ClipboardSnapshot::Empty, false, false);
        assert_eq!(ins.insert("x"), InsertOutcome::Pasted);
        let st = st.lock().unwrap();
        assert!(st.restored.is_empty(), "restore must be skipped");
        assert!(matches!(&st.content, ClipboardSnapshot::Text(t) if t == "x"));
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
        assert_send_sync::<ClipboardInserter<SystemClipboard, MainThreadKeystroke<EnigoKeystroke>>>(
        );
    }

    #[test]
    fn context_is_unknown_before_any_insert() {
        let (ins, _) = rig(ClipboardSnapshot::Empty, false, false);
        assert_eq!(ins.context().prev_char, None);
    }

    type CtxRig = (
        ClipboardInserter<FakeClipboard, FakeKeys>,
        Arc<Mutex<Option<String>>>,
        Arc<Mutex<std::time::Instant>>,
    );

    /// Rig with a controllable frontmost app and clock.
    fn ctx_rig() -> CtxRig {
        let (ins, _) = rig(ClipboardSnapshot::Text("old".into()), false, false);
        let app = Arc::new(Mutex::new(Some("Notes".to_string())));
        let clock = Arc::new(Mutex::new(std::time::Instant::now()));
        let (a, c) = (app.clone(), clock.clone());
        let ins = ins
            .with_app_name(move || a.lock().unwrap().clone())
            .with_clock(move || *c.lock().unwrap());
        (ins, app, clock)
    }

    #[test]
    fn context_remembers_last_char_in_same_app_within_60s() {
        let (ins, _app, clock) = ctx_rig();
        assert_eq!(ins.insert("Hello there."), InsertOutcome::Pasted);
        assert_eq!(ins.context().prev_char, Some('.'));
        *clock.lock().unwrap() += Duration::from_secs(59);
        assert_eq!(ins.context().prev_char, Some('.'));
        *clock.lock().unwrap() += Duration::from_secs(2);
        assert_eq!(ins.context().prev_char, None, "older than 60 s");
    }

    #[test]
    fn context_is_forgotten_when_the_frontmost_app_changes() {
        let (ins, app, _clock) = ctx_rig();
        ins.insert("Hello");
        *app.lock().unwrap() = Some("Mail".into());
        assert_eq!(ins.context().prev_char, None);
        *app.lock().unwrap() = Some("Notes".into());
        assert_eq!(ins.context().prev_char, Some('o'));
    }

    #[test]
    fn copied_only_does_not_count_as_inserted_text() {
        let (ins, _) = rig(ClipboardSnapshot::Text("old".into()), true, false);
        ins.insert("Hello.");
        assert_eq!(ins.context().prev_char, None);
    }
}
