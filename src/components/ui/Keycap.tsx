import type { ReactNode } from 'react';
import type { Binding, Key } from '../../ipc';

export type Platform = 'mac' | 'other';

export const currentPlatform = (): Platform =>
  typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform) ? 'mac' : 'other';

export function Keycap({ children }: { children: ReactNode }) {
  return <kbd className="ui-keycap">{children}</kbd>;
}

const GLYPHS: Record<Platform, Record<Key, string>> = {
  mac: {
    Fn: 'fn', ControlLeft: '⌃', ControlRight: '⌃', AltLeft: '⌥', AltRight: '⌥',
    ShiftLeft: '⇧', ShiftRight: '⇧', MetaLeft: '⌘', MetaRight: '⌘', Space: 'Space', Escape: 'Esc',
  },
  other: {
    Fn: 'Fn', ControlLeft: 'Ctrl', ControlRight: 'Ctrl', AltLeft: 'Alt', AltRight: 'Alt',
    ShiftLeft: 'Shift', ShiftRight: 'Shift', MetaLeft: 'Win', MetaRight: 'Win', Space: 'Space', Escape: 'Esc',
  },
};

/** Render a hotkey binding as a row of keycaps. */
export function keycapsFor(binding: Binding, platform: Platform = currentPlatform()): ReactNode {
  return (
    <span className="ui-keycaps">
      {binding.keys.map((k) => <Keycap key={k}>{GLYPHS[platform][k]}</Keycap>)}
    </span>
  );
}

export default Keycap;
