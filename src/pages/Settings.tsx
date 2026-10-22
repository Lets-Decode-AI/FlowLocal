import { useEffect, useState, type ReactNode } from 'react';
import { api, type DictEntry, type Settings as S } from '../ipc';
import HotkeyInput from '../components/HotkeyInput';
import Toggle from '../components/Toggle';
import { useToast } from '../components/Toasts';
import { errMsg } from '../lib/format';

const IDLE: [string, number | null][] = [['5 minutes', 5], ['10 minutes', 10], ['30 minutes', 30], ['Never', null]];

function Row({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <div className="setting">
      <div>
        <div className="setting-title">{title}</div>
        {hint && <div className="row-sub">{hint}</div>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}

export default function Settings() {
  const toast = useToast();
  const [s, setS] = useState<S | null>(null);
  const [mics, setMics] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.settingsGet().then(setS, (e) => setError(errMsg(e)));
    api.micList().then(setMics, () => {});
  }, []);

  const save = async (patch: Partial<S>) => {
    if (!s) return;
    const next = { ...s, ...patch };
    const prev = s;
    setS(next);
    setError(null);
    try {
      await api.settingsSet(next);
    } catch (e) {
      setS(prev); // backend rejected it (invalid binding, autostart failure, …)
      setError(errMsg(e));
      toast(errMsg(e), 'error');
    }
  };

  if (!s) return <div className="page">{error && <div className="row-error">{error}</div>}</div>;

  return (
    <div className="page">
      <header className="page-head">
        <h1>Settings</h1>
      </header>
      {error && <div className="banner-error" role="alert">{error}</div>}
      <section className="card">
        <Row title="Dictation hotkey" hint="Hold to talk, double-tap to go hands-free.">
          <HotkeyInput value={s.hotkey} onChange={(hotkey) => save({ hotkey })} />
        </Row>
        <Row title="Microphone">
          <select className="input" aria-label="Microphone" value={s.mic ?? ''} onChange={(e) => save({ mic: e.target.value || null })}>
            <option value="">System default</option>
            {mics.map((m) => <option key={m} value={m}>{m}</option>)}
            {s.mic && !mics.includes(s.mic) && <option value={s.mic}>{s.mic} (unavailable)</option>}
          </select>
        </Row>
        <Row title="Show Flow Bar" hint="The small pill at the bottom of your screen.">
          <Toggle label="Show Flow Bar" checked={s.showFlowbar} onChange={(showFlowbar) => save({ showFlowbar })} />
        </Row>
        <Row title="Unload model when idle" hint="Frees memory; the next dictation reloads it.">
          <select className="input" aria-label="Unload model when idle" value={String(s.idleUnloadMin)} onChange={(e) => save({ idleUnloadMin: e.target.value === 'null' ? null : Number(e.target.value) })}>
            {IDLE.map(([l, v]) => <option key={l} value={String(v)}>{l}</option>)}
          </select>
        </Row>
        <Row title="Preload model at startup" hint="Faster first dictation, uses more memory.">
          <Toggle label="Preload model at startup" checked={s.preload} onChange={(preload) => save({ preload })} />
        </Row>
        <Row title="Launch at login">
          <Toggle label="Launch at login" checked={s.launchAtLogin} onChange={(launchAtLogin) => save({ launchAtLogin })} />
        </Row>
      </section>
      <HfToken />
      <Dictionary />
    </div>
  );
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
    <section className="card">
      <Row title="Hugging Face token" hint="Optional. Needed only for gated models. Stored in your OS keychain.">
        <div className="inline">
          <input className="input" type="password" aria-label="Hugging Face token" placeholder="hf_…" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
          <button className="btn" disabled={!token.trim()} onClick={() => store(token.trim())}>Save</button>
          <button className="btn btn-ghost" onClick={() => store(null)}>Remove</button>
        </div>
      </Row>
    </section>
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
    <section className="card">
      <div className="setting-title">Personal dictionary</div>
      <div className="row-sub">Names and jargon to spell correctly. Add a replacement to rewrite a term automatically.</div>
      <form className="inline dict-add" onSubmit={(e) => { e.preventDefault(); add(); }}>
        <input className="input" aria-label="Term" placeholder="Term" value={term} onChange={(e) => setTerm(e.target.value)} />
        <input className="input" aria-label="Replacement" placeholder="Replacement (optional)" value={rep} onChange={(e) => setRep(e.target.value)} />
        <button className="btn" type="submit" disabled={!term.trim()}>Add</button>
      </form>
      {items.length === 0 ? (
        <div className="row-sub dict-empty">No custom terms yet.</div>
      ) : (
        <ul className="dict">
          {items.map((d) => (
            <li key={d.id}>
              <span>{d.term}{d.replacement ? <span className="muted"> → {d.replacement}</span> : null}</span>
              <button className="btn btn-ghost" aria-label={`Remove ${d.term}`} onClick={() => del(d.id)}>Remove</button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
