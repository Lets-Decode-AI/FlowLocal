import { useEffect, useRef, useState } from 'react';
import { api, type CatalogEntry, type DownloadProgress, type Perm, type PermissionsStatus, type Settings } from '../ipc';
import ProgressBar from '../components/ProgressBar';
import { bindingLabel } from '../components/HotkeyInput';
import { useToast } from '../components/Toasts';
import { errMsg, fmtBytes, fmtEta } from '../lib/format';
import { navigate, useEvent } from '../lib/hooks';

const STEPS = ['Welcome', 'Permissions', 'Model', 'Try it'];
const ok = (p: Perm) => p === 'granted' || p === 'notRequired';

export default function Onboarding() {
  const [step, setStep] = useState(0);
  return (
    <div className="onb">
      <div className="onb-card">
        <div className="onb-dots" aria-label={`Step ${step + 1} of ${STEPS.length}: ${STEPS[step]}`}>
          {STEPS.map((s, i) => <span key={s} className={`dot${i <= step ? ' dot-on' : ''}`} />)}
        </div>
        {step === 0 && <Welcome onNext={() => setStep(1)} />}
        {step === 1 && <Permissions onNext={() => setStep(2)} />}
        {step === 2 && <ModelStep onNext={() => setStep(3)} />}
        {step === 3 && <TryIt />}
      </div>
    </div>
  );
}

function Welcome({ onNext }: { onNext: () => void }) {
  return (
    <>
      <h1>Welcome to FlowLocal</h1>
      <p className="muted">Talk instead of type, in any app. Everything is transcribed on this computer — your voice never leaves it.</p>
      <div className="onb-actions"><button className="btn btn-primary" onClick={onNext}>Get started</button></div>
    </>
  );
}

function PermRow({ title, why, status, onOpen, onRequest }: { title: string; why: string; status: Perm; onOpen: () => void; onRequest?: () => void }) {
  return (
    <div className="row">
      <div className="row-main">
        <div className="row-title">{title}</div>
        <div className="row-sub">{why}</div>
      </div>
      <div className="row-actions">
        {status === 'notRequired' ? <span className="muted">Not required</span> : ok(status) ? <span className="badge badge-ok">Granted</span> : (
          <>
            <span className="muted">Not granted</span>
            {onRequest && <button className="btn" onClick={onRequest}>Allow</button>}
            <button className="btn" onClick={onOpen}>Open Settings</button>
          </>
        )}
      </div>
    </div>
  );
}

function Permissions({ onNext }: { onNext: () => void }) {
  const toast = useToast();
  const [p, setP] = useState<PermissionsStatus | null>(null);
  useEffect(() => {
    let live = true;
    const poll = () => api.permissionsStatus().then((s) => live && setP(s), () => {});
    poll();
    const t = setInterval(poll, 1000);
    return () => { live = false; clearInterval(t); };
  }, []);
  const attempt = (f: () => Promise<void>) => f().catch((e) => toast(errMsg(e), 'error'));
  const ready = !!p && ok(p.microphone) && ok(p.accessibility);
  return (
    <>
      <h1>Permissions</h1>
      <p className="muted">FlowLocal needs the microphone to hear you, and accessibility access to type text into other apps.</p>
      <div className="list">
        <PermRow title="Microphone" why="Record your voice while you hold the hotkey." status={p?.microphone ?? 'unknown'} onRequest={() => attempt(api.requestMicrophone)} onOpen={() => attempt(() => api.openPermissionSettings('microphone'))} />
        <PermRow title="Accessibility" why="Paste the transcript into the app you are using." status={p?.accessibility ?? 'unknown'} onOpen={() => attempt(() => api.openPermissionSettings('accessibility'))} />
      </div>
      <div className="onb-actions">
        <button className="btn btn-ghost" onClick={onNext}>Skip for now</button>
        <button className="btn btn-primary" disabled={!ready} onClick={onNext}>Continue</button>
      </div>
    </>
  );
}

function ModelStep({ onNext }: { onNext: () => void }) {
  const toast = useToast();
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [installedIds, setInstalledIds] = useState<string[]>([]);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [picking, setPicking] = useState(false);
  const chosenRef = useRef<string | null>(null);
  chosenRef.current = chosen;

  useEffect(() => {
    Promise.all([api.catalog(), api.recommend(), api.installed()]).then(
      ([c, r, i]) => {
        setCatalog(c);
        setChosen(r.id);
        setInstalledIds(i.map((m) => m.id));
      },
      (e) => toast(errMsg(e), 'error'),
    );
  }, [toast]);

  useEvent('progress', (p) => {
    if (p.modelId !== chosenRef.current) return;
    setProgress(p);
    if (p.status === 'Installed') {
      setInstalledIds((x) => (x.includes(p.modelId) ? x : [...x, p.modelId]));
      api.modelSetActive(p.modelId).catch((e) => toast(errMsg(e), 'error'));
    }
  });

  const entry = catalog.find((c) => c.id === chosen);
  const isInstalled = !!chosen && installedIds.includes(chosen);
  const st = progress?.status;
  const failed = typeof st === 'object' && st !== null ? st.Failed : null;
  const paused = !isInstalled && st === 'Paused';
  const downloading = !isInstalled && (st === 'Downloading' || st === 'Queued' || st === 'Verifying');
  const pct = progress && progress.total > 0 ? (progress.bytesDone / progress.total) * 100 : 0;

  const download = async () => {
    if (!chosen) return;
    setProgress(null);
    try {
      await api.modelInstall(chosen);
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  const act = async (f: () => Promise<void>, after?: () => void) => {
    try {
      await f();
      after?.();
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  const proceed = async () => {
    // The recommended model may already be installed without being active.
    try {
      if (chosen) await api.modelSetActive(chosen);
      onNext();
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  const choose = (id: string) => {
    setChosen(id);
    setProgress(null);
    setPicking(false);
  };

  return (
    <>
      <h1>Choose a model</h1>
      <p className="muted">We picked the best fit for this computer. You can switch models any time.</p>
      {entry && !picking && (
        <div className="row">
          <div className="row-main">
            <div className="row-title">{entry.name}</div>
            <div className="row-sub">{entry.languages.length > 6 ? `${entry.languages.length} languages` : entry.languages.join(', ')} · {fmtBytes(entry.files.reduce((n, f) => n + f.size, 0))}</div>
            {(downloading || paused) && progress && (
              <div className="row-progress">
                <ProgressBar value={pct} label="Model download" />
                <div className="row-sub">{st === 'Verifying' ? 'Verifying…' : paused ? 'Paused' : `${fmtBytes(progress.bytesDone)} of ${fmtBytes(progress.total)} ${fmtEta(progress.etaSecs)}`}</div>
              </div>
            )}
            {failed && <div className="row-error">Download failed: {failed}</div>}
          </div>
          <div className="row-actions">
            {isInstalled ? <span className="badge badge-ok">Installed</span> : paused ? (
              <>
                <button className="btn btn-primary" onClick={() => chosen && act(() => api.modelResume(chosen))}>Resume</button>
                <button className="btn btn-ghost" onClick={() => chosen && act(() => api.modelCancel(chosen), () => setProgress(null))}>Cancel</button>
              </>
            ) : !downloading && <button className="btn btn-primary" onClick={download}>{failed ? 'Retry' : 'Download'}</button>}
          </div>
        </div>
      )}
      {picking && (
        <div className="list">
          {catalog.map((c) => (
            <button key={c.id} className="row row-pick" onClick={() => choose(c.id)}>
              <div className="row-main"><div className="row-title">{c.name}</div><div className="row-sub">{c.notes || c.languages.join(', ')}</div></div>
            </button>
          ))}
        </div>
      )}
      <div className="onb-actions">
        {!downloading && <button className="btn btn-ghost" onClick={() => setPicking((x) => !x)}>{picking ? 'Back' : 'Choose another'}</button>}
        <button className="btn btn-primary" disabled={!isInstalled} onClick={proceed}>Continue</button>
      </div>
    </>
  );
}

function TryIt() {
  const toast = useToast();
  const [hotkey, setHotkey] = useState('Fn');
  const [done, setDone] = useState(false);
  const finished = useRef(false);

  useEffect(() => {
    api.settingsGet().then((s) => setHotkey(bindingLabel(s.hotkey)), () => {});
  }, []);

  useEffect(() => {
    if (!done) return;
    const t = setTimeout(() => navigate('/history'), 1800);
    return () => clearTimeout(t);
  }, [done]);

  useEvent('transcriptAdded', async () => {
    if (finished.current) return;
    finished.current = true;
    setDone(true);
    try {
      const s: Settings = await api.settingsGet();
      await api.settingsSet({ ...s, onboarded: true });
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  });

  return (
    <>
      <h1>{done ? "You're all set" : 'Try it out'}</h1>
      <p className="muted">{done ? 'Taking you to your history…' : `Click the box below, then hold ${hotkey} and speak. Release to insert your words.`}</p>
      <textarea className="input onb-text" autoFocus rows={5} placeholder="Your words will appear here…" />
      {done && <div className="onb-actions"><button className="btn btn-primary" onClick={() => navigate('/history')}>Open FlowLocal</button></div>}
    </>
  );
}
