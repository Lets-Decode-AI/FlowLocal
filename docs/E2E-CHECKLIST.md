# Manual end-to-end checklist

Automated tests cannot cover permissions, global hotkeys, synthetic paste or real apps, so run this on a **release
build** before tagging a release. Use a fresh app data directory (macOS: move
`~/Library/Application Support/com.flowlocal.app` aside) and install the `.dmg` into `/Applications`.

Record the OS, version, date and results in the release PR.

## 1. First run

- [ ] App launches from `/Applications` and appears in the tray; no Dock-less crash within 20 seconds.
- [ ] Onboarding opens. Grant Microphone and Accessibility; the status indicators turn green without restarting.
- [ ] The recommended model (Parakeet TDT 0.6B v2 on an English machine) downloads with progress, speed and ETA; pause and resume work.
- [ ] After download the model can be set active and the try-it field accepts dictation.

## 2. Dictation in real apps

For each of **TextEdit, a Safari textarea, VS Code and Slack**:

- [ ] Hold the hotkey (`Fn` on macOS), say a sentence with natural punctuation, release: punctuated, capitalized text is inserted at the cursor.
- [ ] The Flow Bar shows recording, then transcribing, then done, and never steals focus.

## 3. Behaviors

- [ ] Clipboard restore: copy some text, dictate, then paste: the original text is back. Repeat with an image on the clipboard.
- [ ] `Esc` while recording cancels; nothing is inserted.
- [ ] Double-tap the hotkey: hands-free mode starts; press again to finish and insert.
- [ ] A tap shorter than 250 ms and a silent recording insert nothing and show no error.
- [ ] A dictation longer than 30 s (speak for about a minute with a few pauses) is inserted promptly after key-up.
- [ ] The dictation appears in History with the right app name and word count; copy and delete work.
- [ ] A personal dictionary replacement is applied.
- [ ] With no model installed (or Accessibility revoked), triggering the hotkey shows a clear Flow Bar error.

## 4. Performance

- [ ] A ~5 s utterance is inserted in under 500 ms after key-up on Apple Silicon (check the timing in the logs, or the numbers from the ignored integration test).
- [ ] After 10 minutes idle with the model unloaded, total memory of all FlowLocal processes (including the WebKit content process for the Flow Bar) is under 60 MB in Activity Monitor.
- [ ] The installer is under 25 MB.

## 5. Other platforms (best effort)

- [ ] Windows: install via the NSIS installer, hold `Ctrl+Win`, dictate into Notepad.
- [ ] Windows: releasing `Ctrl+Win` must not open the Start menu (the Win key is not masked in Part 1). Note what happens and whether the Start menu steals focus before the paste.
- [ ] Windows / X11: press `Win+L` (lock) or `Alt+Tab` while the chord is partly held, unlock/return, then press only `Ctrl`: no recording may start (stale-modifier check).
- [ ] Linux (X11): install the `.deb` or run the `.AppImage`, hold `Ctrl+Super`, dictate into a text editor.
- [ ] Linux (Wayland): confirm the clipboard fallback notice appears when paste is blocked.
