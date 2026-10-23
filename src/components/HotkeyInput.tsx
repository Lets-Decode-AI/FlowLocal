import { useEffect, useRef, useState } from 'react';
import type { Binding, Key } from '../ipc';
import Button from './ui/Button';

const CODE_TO_KEY: Record<string, Key> = {
  ControlLeft: 'ControlLeft', ControlRight: 'ControlRight', MetaLeft: 'MetaLeft', MetaRight: 'MetaRight',
  AltLeft: 'AltLeft', AltRight: 'AltRight', ShiftLeft: 'ShiftLeft', ShiftRight: 'ShiftRight', Space: 'Space', Fn: 'Fn',
};

const PRESETS: { label: string; keys: Key[] }[] = [
  { label: 'Fn', keys: ['Fn'] },
  { label: 'Right ⌥', keys: ['AltRight'] },
  { label: 'Ctrl+Win/⌘', keys: ['ControlLeft', 'MetaLeft'] },
];

const same = (a: Binding, b: Key[]) => a.keys.length === b.length && b.every((k) => a.keys.includes(k));

export default function HotkeyInput({ value, onChange }: { value: Binding; onChange: (b: Binding) => void }) {
  const [recording, setRecording] = useState(false);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

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
        onChangeRef.current({ keys: [...chord] });
      }
    };
    window.addEventListener('keydown', down, true);
    window.addEventListener('keyup', up, true);
    return () => {
      window.removeEventListener('keydown', down, true);
      window.removeEventListener('keyup', up, true);
    };
  }, [recording]);

  return (
    <div className="hotkey">
      <div className="hotkey-choices" role="group" aria-label="Shortcut choices">
        {PRESETS.map((p) => (
          <Button key={p.label} aria-pressed={same(value, p.keys)} onClick={() => { setRecording(false); onChange({ keys: p.keys }); }}>
            {p.label}
          </Button>
        ))}
        <Button onClick={() => setRecording((r) => !r)}>{recording ? 'Cancel' : 'Record shortcut'}</Button>
      </div>
      <div className="hotkey-status muted" aria-live="polite">{recording ? 'Press the keys. Press Esc to cancel.' : ''}</div>
    </div>
  );
}
