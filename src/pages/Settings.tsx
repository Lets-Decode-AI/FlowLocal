import { useEffect, useRef, useState } from 'react';
import { api, type DictEntry, type Settings as S } from '../ipc';
import HotkeyInput from '../components/HotkeyInput';
import { useToast } from '../components/Toasts';
import Button from '../components/ui/Button';
import Group from '../components/ui/Group';
import IconButton from '../components/ui/IconButton';
import { keycapsFor } from '../components/ui/Keycap';
import Row from '../components/ui/Row';
import Segmented from '../components/ui/Segmented';
import Switch from '../components/ui/Switch';
import { errMsg } from '../lib/format';
import { useTheme, type ThemePref } from '../lib/theme';

const THEMES: { value: ThemePref; label: string }[] = [{ value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }];
const IDLE: [string, number | null][] = [['After 5 minutes', 5], ['After 10 minutes', 10], ['After 30 minutes', 30], ['Never', null]];

export default function Settings() {
  const toast = useToast();
  const [s, setS] = useState<S | null>(null);
  const [mics, setMics] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [changing, setChanging] = useState(false);

  const confirmed = useRef<S | null>(null); // last state the backend accepted
  const queue = useRef<Promise<void>>(Promise.resolve());
  const pending = useRef(0);

  useEffect(() => {
    api.settingsGet().then((v) => { confirmed.current = v; setS(v); }, (e) => setError(errMsg(e)));
    api.micList().then(setMics, () => {});
  }, []);

  // Saves run one at a time, each applied on top of the last accepted state, so a rejected
  // save can never wipe a later one.
  const save = (patch: Partial<S>) => {
    setS((x) => (x ? { ...x, ...patch } : x));
    setError(null);
    pending.current++;
    queue.current = queue.current.then(async () => {
      try {
        const base = confirmed.current;
        if (!base) return;
        const next = { ...base, ...patch };
        await api.settingsSet(next);
        confirmed.current = next;
      } catch (e) {
        setError(errMsg(e)); // the backend rejected it, for example an invalid shortcut
        toast(errMsg(e), 'error');
      } finally {
        if (--pending.current === 0) setS(confirmed.current);
      }
    });
  };

  if (!s) return <div className="page">{error && <div className="banner-error" role="alert">{error}</div>}</div>;

  return (
    <div className="page">
      <header className="page-head">
        <h1>Settings</h1>
      </header>
      {error && <div className="banner-error" role="alert">{error}</div>}

      <Group title="Dictation" footer="Hold the shortcut to talk, or double-press it to keep listening hands-free.">
        <Row
          label="Shortcut"
          accessory={
            <>
              <span role="group" aria-label="Current shortcut">{keycapsFor(s.hotkey)}</span>
              <Button aria-expanded={changing} onClick={() => setChanging((c) => !c)}>Change</Button>
            </>
          }
        />
        {changing && (
          <div className="ui-row ui-row-panel">
            <HotkeyInput value={s.hotkey} onChange={(hotkey) => save({ hotkey })} />
          </div>
        )}
        <Row
          label="Microphone"
          accessory={
            <select className="ui-select" aria-label="Microphone" value={s.mic ?? ''} onChange={(e) => save({ mic: e.target.value || null })}>
              <option value="">System default</option>
              {mics.map((m) => <option key={m} value={m}>{m}</option>)}
              {s.mic && !mics.includes(s.mic) && <option value={s.mic}>{s.mic} (not connected)</option>}
            </select>
          }
        />
        <Row
          label="Show Flow Bar" detail="The small pill at the bottom of the screen."
          accessory={<Switch label="Show Flow Bar" checked={s.showFlowbar} onChange={(showFlowbar) => save({ showFlowbar })} />}
        />
      </Group>

      <Group title="Model" footer="Unloading the model frees memory. The next dictation loads it again.">
        <Row
          label="Unload when idle"
          accessory={
            <select className="ui-select" aria-label="Unload when idle" value={String(s.idleUnloadMin)} onChange={(e) => save({ idleUnloadMin: e.target.value === 'null' ? null : Number(e.target.value) })}>
              {IDLE.map(([l, v]) => <option key={l} value={String(v)}>{l}</option>)}
            </select>
          }
        />
        <Row
          label="Load at startup" detail="Makes the first dictation faster and uses more memory."
          accessory={<Switch label="Load at startup" checked={s.preload} onChange={(preload) => save({ preload })} />}
        />
      </Group>

      <Group title="General">
        <Row label="Open at login" accessory={<Switch label="Open at login" checked={s.launchAtLogin} onChange={(launchAtLogin) => save({ launchAtLogin })} />} />
        <Appearance />
      </Group>

      <Dictionary />
      <HfToken />
    </div>
  );
}

function Appearance() {
  const { pref, setPref } = useTheme();
  return <Row label="Appearance" detail="System follows your computer's light or dark mode." accessory={<Segmented options={THEMES} value={pref} onChange={setPref} />} />;
}

function HfToken() {
  const toast = useToast();
  const [token, setToken] = useState('');
  const store = async (t: string | null) => {
    try {
      await api.hfTokenSet(t);
      setToken('');
      toast(t ? 'Token saved to your keychain' : 'Token removed');
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  return (
    <Group title="Hugging Face" footer="Only needed to download gated or private models. The token is stored in your system keychain.">
      <form className="ui-row ui-row-form" onSubmit={(e) => { e.preventDefault(); if (token.trim()) store(token.trim()); }}>
        <input className="ui-field" type="password" aria-label="Hugging Face token" placeholder="Access token" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
        <Button type="submit" disabled={!token.trim()}>Save</Button>
        <Button variant="plain" onClick={() => store(null)}>Remove</Button>
      </form>
    </Group>
  );
}

function Dictionary() {
  const toast = useToast();
  const [items, setItems] = useState<DictEntry[]>([]);
  const [term, setTerm] = useState('');
  const [rep, setRep] = useState('');
  const load = () => api.dictionaryList().then(setItems, (e) => toast(errMsg(e), 'error'));
  useEffect(() => { load(); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const add = async () => {
    if (!term.trim()) return;
    try {
      await api.dictionaryAdd(term.trim(), rep.trim() || undefined);
      setTerm('');
      setRep('');
      load();
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  const del = async (id: number) => {
    try {
      await api.dictionaryDelete(id);
      load();
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };

  return (
    <Group title="Personal dictionary" footer="Add names and jargon so they are spelled correctly. A replacement rewrites the term as you dictate.">
      {items.map((d) => (
        <Row
          key={d.id}
          label={d.term}
          detail={d.replacement ? `Replaced with ${d.replacement}` : undefined}
          accessory={<IconButton icon="trash" label={`Remove ${d.term}`} tone="destructive" onClick={() => del(d.id)} />}
        />
      ))}
      <form className="ui-row ui-row-form" onSubmit={(e) => { e.preventDefault(); add(); }}>
        <input className="ui-field" aria-label="Term" placeholder="Term" value={term} onChange={(e) => setTerm(e.target.value)} />
        <input className="ui-field" aria-label="Replacement" placeholder="Replacement (optional)" value={rep} onChange={(e) => setRep(e.target.value)} />
        <Button type="submit" disabled={!term.trim()}>Add</Button>
      </form>
    </Group>
  );
}
