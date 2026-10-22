import { useEffect, useState } from 'react';
import type { Binding, Key } from '../ipc';

const isMac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform);

const CODE_TO_KEY: Record<string, Key> = {
  ControlLeft: 'ControlLeft', ControlRight: 'ControlRight', MetaLeft: 'MetaLeft', MetaRight: 'MetaRight',
  AltLeft: 'AltLeft', AltRight: 'AltRight', ShiftLeft: 'ShiftLeft', ShiftRight: 'ShiftRight', Space: 'Space', Fn: 'Fn',
};

const KEY_LABEL: Record<Key, string> = {
  Fn: 'Fn', ControlLeft: 'Left Ctrl', ControlRight: 'Right Ctrl',
  MetaLeft: isMac ? 'Left ⌘' : 'Left Win', MetaRight: isMac ? 'Right ⌘' : 'Right Win',
  AltLeft: isMac ? 'Left ⌥' : 'Left Alt', AltRight: isMac ? 'Right ⌥' : 'Right Alt',
  ShiftLeft: 'Left Shift', ShiftRight: 'Right Shift', Space: 'Space', Escape: 'Esc',
};

export const bindingLabel = (b: Binding) => b.keys.map((k) => KEY_LABEL[k]).join(' + ');

const PRESETS: { label: string; keys: Key[] }[] = [
  { label: 'Fn', keys: ['Fn'] },
  { label: 'Right ⌥', keys: ['AltRight'] },
  { label: 'Ctrl+Win/⌘', keys: ['ControlLeft', 'MetaLeft'] },
];

const same = (a: Binding, b: Key[]) => a.keys.length === b.length && b.every((k) => a.keys.includes(k));

export default function HotkeyInput({ value, onChange }: { value: Binding; onChange: (b: Binding) => void }) {
  const [recording, setRecording] = useState(false);

  useEffect(() => {
    if (!recording) return;
    const held = new Set<Key>();
    const chord: Key[] = [];
    const down = (e: KeyboardEvent) => {
      if (e.code === 'Escape' || e.key === 'Escape') {
        e.preventDefault();
        setRecording(false); // Escape cancels and is never a binding
        return;
      }
      const k = CODE_TO_KEY[e.code] ?? (e.key === 'Fn' ? 'Fn' : undefined);
      if (!k) return;
      e.preventDefault();
      held.add(k);
      if (!chord.includes(k)) chord.push(k);
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === 'Escape') return;
      if (chord.length) {
        e.preventDefault();
        setRecording(false);
        onChange({ keys: [...chord] });
      }
    };
    window.addEventListener('keydown', down, true);
    window.addEventListener('keyup', up, true);
    return () => {
      window.removeEventListener('keydown', down, true);
      window.removeEventListener('keyup', up, true);
    };
  }, [recording, onChange]);

  return (
    <div className="hotkey">
      <div className="hotkey-current" aria-live="polite">
        {recording ? 'Press the keys… (Esc to cancel)' : bindingLabel(value)}
      </div>
      <div className="hotkey-actions">
        {PRESETS.map((p) => (
          <button key={p.label} type="button" className={`chip${same(value, p.keys) ? ' chip-on' : ''}`} onClick={() => { setRecording(false); onChange({ keys: p.keys }); }}>
            {p.label}
          </button>
        ))}
        <button type="button" className="chip" onClick={() => setRecording((r) => !r)}>
          {recording ? 'Cancel' : 'Record…'}
        </button>
      </div>
    </div>
  );
}
