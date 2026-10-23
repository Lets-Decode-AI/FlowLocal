# UI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make FlowLocal look and feel like a first-party macOS app (System Settings / Notes / Voice Memos), with a clearer 5-step onboarding and plain, human UI copy with no em dashes.

**Architecture:** One design-system stylesheet (`src/styles/`) with tokens and primitives; small shared React primitives in `src/components/ui/`; each page restyled on top. Window chrome via Tauri config (overlay title bar, macOS sidebar vibrancy). Behavior, IPC, and existing tests stay intact.

**Tech Stack:** React 18 + TypeScript + plain CSS (Tailwind may remain for utilities but primitives use CSS variables), Tauri 2 window config, Vitest + Testing Library.

**Spec:** this file (approved in chat on 2026-10-23). Product spec for behavior: `docs/superpowers/specs/2026-10-22-part1-core-dictation-design.md`.

## Global Constraints

- Font stack: `-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif`. Remove the bundled Inter dependency and import.
- Base text 13px / line-height 1.4 (macOS body); headings: page title 22px semibold, section header 11px uppercase-free semibold muted (macOS uses sentence-case 13px semibold for group headers; use that).
- Colors (light / dark), Apple system values: accent `#007AFF` / `#0A84FF`; green `#34C759` / `#30D158`; red `#FF3B30` / `#FF453A`; orange `#FF9500` / `#FF9F0A`; window bg `#F5F5F7` / `#1E1E1E`; grouped surface `#FFFFFF` / `#2A2A2C`; label `#1D1D1F` / `#F5F5F7`; secondary label `rgba(60,60,67,0.6)` / `rgba(235,235,245,0.6)`; separator `rgba(60,60,67,0.18)` / `rgba(84,84,88,0.6)`; fill `rgba(120,120,128,0.12)` / `rgba(120,120,128,0.24)`.
- Radii: grouped list 10px, buttons 6px, pills 999px. Inset separators start at the row's text column.
- Motion: 150–220 ms ease-out, opacity/transform only; all disabled under `prefers-reduced-motion`.
- Copy rules: no em dashes (`—`) or en dashes used as punctuation anywhere in UI strings, Rust user-facing strings, README, CONTRIBUTING, SECURITY, SUPPORT, CODE_OF_CONDUCT, docs/*.md, .github templates. Sentence case. Short, specific, plain verbs. No filler ("seamlessly", "effortlessly", "powerful", "simply", "just"), no exclamation marks, no emoji in UI copy. `docs/superpowers/**` is excluded (historical record).
- Accessibility: every control keyboard reachable with visible focus ring (accent, 3px soft ring like macOS); labels on icon-only buttons; color is never the only signal (icons/text alongside).
- Do not change IPC, Rust behavior, or command names. Existing tests must keep passing; update selectors/text in tests only where copy changed.

---

### Task R1: Design system, primitives, window chrome, sidebar

**Files:** Create `src/styles/tokens.css`, `src/styles/base.css`, `src/components/ui/{Group.tsx,Row.tsx,Switch.tsx,Segmented.tsx,Button.tsx,Keycap.tsx,Icon.tsx}`; Modify `src/main.css` (replace), `src/main.tsx`, `src/components/{Sidebar,Toggle,Toasts,ProgressBar}.tsx`, `src-tauri/tauri.conf.json` (main window config lives in Rust — modify where the main window is built in `src-tauri/src/lib.rs`), `package.json` (drop Inter).

**Interfaces (Produces):**
- `<Group title? footer?>` — inset grouped list container. `<Row label detail? icon? accessory? onClick?>` — one row; inset separator between rows.
- `<Switch checked onChange label>` (macOS switch, role="switch"), `<Segmented options value onChange>` (role="tablist"), `<Button variant="primary"|"secondary"|"plain"|"destructive" size="regular"|"large">`, `<Keycap>{'fn'}</Keycap>` and `keycapsFor(binding) -> ReactNode` rendering ⌃ ⌥ ⇧ ⌘ fn glyphs (Windows/Linux: Ctrl, Alt, Shift, Win), `<Icon name>` with inline 1.5px-stroke SVGs: home, history, models (cube), settings (gear), mic, keyboard, check, warning, download, pause, play, xmark, trash, copy, search, sparkle-free.
- Main window: macOS `titleBarStyle: Overlay`, `hiddenTitle: true`, traffic lights inset (`trafficLightPosition` ≈ {x:16,y:20} if supported), sidebar uses `effects: ["sidebar"]` / window vibrancy on macOS with translucent CSS background; on Windows/Linux an opaque `--sidebar-bg`. Content area has a 52px draggable header region (`data-tauri-drag-region`). Min window size 760×520, default 900×620.
- Sidebar: icon + label rows (History, Models, Settings), selected row uses accent-tinted fill like Finder; status footer shows a dot + "Ready" / "Listening" / "Transcribing" / "Error".

- [ ] Tests: `switch_toggles_and_has_role`, `segmented_selects_option`, `keycaps_render_mac_glyphs` (mock platform), `row_renders_detail_and_accessory`. RED → implement → GREEN.
- [ ] Replace old classes (`.btn`, `.card`, `.tabs`, `.toggle`, `.chip`) usage in shared components; pages keep working (they are restyled in R3).
- [ ] `npx vitest run`, `npx tsc --noEmit`, `cargo test`, `npm run tauri dev` ~60 s clean. Commit `feat(ui): macOS-style design system, primitives and window chrome`.

### Task R2: Onboarding redesign

**Files:** Rewrite `src/pages/Onboarding.tsx` (split steps into `src/pages/onboarding/*.tsx`), tests in `src/test/onboarding.test.tsx`.

Steps (progress dots at top, Back/Continue footer, one idea per screen, centered 440px column, large 64px app glyph on Welcome):
1. **Welcome** — title "FlowLocal", one line: "Dictate into any app. Speech is turned into text on this Mac and never leaves it." (Windows/Linux: "on this computer"). Button "Continue".
2. **Your shortcut** — animated keycap of the current binding with caption "Hold to talk. Release to insert the text." Second line: "Double-press to keep listening hands-free. Press Esc to cancel." macOS only: a Globe-key note row with button "Open Keyboard Settings" (opens `x-apple.systempreferences:com.apple.Keyboard-Settings.extension`) explaining "Set Press 🌐 key to Do Nothing so macOS doesn't open the emoji picker." (the 🌐 glyph is allowed here as it names the key). Link "Change shortcut" opens the HotkeyInput inline.
3. **Permissions** — two Group rows (Microphone, Accessibility) each with icon, one-line reason, and accessory: green check "Allowed" or Button "Allow…" (opens prompt/settings). Live polling as today. Continue disabled until both allowed (or not required); footer note "You can change these later in System Settings."
4. **Speech model** — recommended model card: name, why ("Recommended for English on Apple silicon" etc., from recommend hardware), size, languages; primary "Download" → progress bar with "X MB of Y MB · about N min left", Pause/Resume/Cancel; "Choose a different model" disclosure lists catalog rows. Already-installed recommended model shows "Ready" and Continue.
5. **Try it** — prompt "Hold <keycap> and say something." textarea that receives the paste; live listening indicator mirroring dictation state; on `history://added` show a success state: check icon, "You're set up.", the transcript text and its word count + WPM, button "Start using FlowLocal" → sets onboarded and goes to #/history.

- [ ] Keep existing behaviors/tests (paused download controls, permission polling cleanup, modelSetActive on continue) — port tests, add `onboarding_has_five_steps_and_back_navigation`, `shortcut_step_shows_current_binding_keycaps`, `try_it_shows_transcript_and_finish`.
- [ ] Commit `feat(ui): five-step onboarding`.

### Task R3: Restyle pages, Flow Bar, and rewrite all copy

**Files:** `src/pages/{History,Models,Settings}.tsx`, `src/components/{ModelRow,HotkeyInput}.tsx`, `src/flowbar/FlowBar.tsx`, `src/index.css` (flowbar styles), Rust user-facing strings (`grep -rn '"' src-tauri/src` for toast/error/tray text), README.md, CONTRIBUTING.md, SECURITY.md, SUPPORT.md, CODE_OF_CONDUCT.md, docs/*.md (not docs/superpowers), .github/**/*.md|yml.

- History: page title + search field in the header area (macOS search field style with magnifier icon), list grouped by day ("Today", "Yesterday", date) in Groups; each row: transcript text (2-line clamp, expand on click), secondary line "App · 10:42 · 18 words"; hover reveals Copy and Delete plain icon buttons; empty state with mic icon "No dictations yet" + "Hold <keycap> in any app to start."
- Models: Segmented control (Installed / Library / Hugging Face); rows in Groups with name, detail "English · 661 MB", accessory Download / progress / "In use" check / menu (Use, Delete). Search field for Hugging Face; Incompatible shows reason in secondary text.
- Settings: System-Settings layout — Groups: "Dictation" (Shortcut row with keycaps + Change, Microphone popup, Show Flow Bar switch), "Model" (Unload when idle popup, Load at startup switch), "General" (Open at login switch), "Personal dictionary" (list + add row), "Hugging Face" (token secure field with footer explaining when it's needed). Footers explain settings in one sentence.
- Flow Bar: dark HUD pill (`rgba(28,28,30,0.82)` with 0.5px `rgba(255,255,255,0.12)` border, subtle shadow), white waveform bars with rounded caps, listening red dot for hands-free, transcribing = three-dot pulse, done = green check, message pill for toasts/errors with icon. Same sizes/behavior as now.
- Copy pass: rewrite every user-visible string (TSX, Rust toasts/errors/tray menu labels, README/docs) following the copy rules; remove all em dashes; keep facts accurate. Produce `docs/COPY.md`? No — not needed.
- [ ] Update tests whose text changed; add `history_groups_by_day`, `no_em_dashes_in_ui_strings` (a test that scans `src/**/*.tsx` and `src-tauri/src/**/*.rs` string literals and docs for `—`).
- [ ] Commits: `feat(ui): restyle history, models and settings`, `feat(ui): HUD-style Flow Bar`, `docs: plain-language copy pass, no em dashes`.
