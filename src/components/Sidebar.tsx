import { useEffect, useState } from 'react';
import { api, type DictationState } from '../ipc';
import { useEvent } from '../lib/hooks';
import Icon, { type IconName } from './ui/Icon';

// Part 3 adds a "Home" dashboard entry above History; until then History is the landing page.
const NAV: { label: string; route: string; icon: IconName }[] = [
  { label: 'History', route: '/history', icon: 'history' },
  { label: 'Models', route: '/models', icon: 'models' },
  { label: 'Settings', route: '/settings', icon: 'settings' },
];

function statusText(s: DictationState): string {
  switch (s.kind) {
    case 'recording': return 'Listening';
    case 'transcribing': return 'Transcribing';
    case 'inserting': return 'Transcribing';
    case 'error': return 'Error';
    default: return 'Ready';
  }
}

export default function Sidebar({ route }: { route: string }) {
  const [state, setState] = useState<DictationState>({ kind: 'idle' });
  useEffect(() => { api.getState().then(setState, () => {}); }, []);
  useEvent('state', setState);
  return (
    <aside className="sidebar">
      <div className="sidebar-drag" data-tauri-drag-region />
      <nav>
        {NAV.map((n) => (
          <a key={n.label} href={`#${n.route}`} className={`nav${route === n.route ? ' nav-on' : ''}`} aria-current={route === n.route ? 'page' : undefined}><Icon name={n.icon} />{n.label}</a>
        ))}
      </nav>
      <div className="side-status"><span className={`status-dot${state.kind !== 'idle' ? ` status-${state.kind}` : ''}`} />{statusText(state)}</div>
    </aside>
  );
}
