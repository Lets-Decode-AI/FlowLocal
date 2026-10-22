import { useCallback, useEffect, useRef, useState } from 'react';
import { api, type CatalogEntry, type DownloadProgress, type HfRepoSummary, type Inspection, type InstalledModelDto } from '../ipc';
import ModelRow from '../components/ModelRow';
import { useToast } from '../components/Toasts';
import { errMsg, fmtBytes } from '../lib/format';
import { useEvent } from '../lib/hooks';

type Tab = 'installed' | 'catalog' | 'hf';
const TABS: [Tab, string][] = [['installed', 'Installed'], ['catalog', 'Catalog'], ['hf', 'Search Hugging Face']];

const size = (e: CatalogEntry) => fmtBytes(e.files.reduce((n, f) => n + f.size, 0));
const meta = (e: CatalogEntry, extra?: string) => [e.languages.length > 6 ? `${e.languages.length} languages` : e.languages.join(', '), extra ?? size(e)].join(' · ');

export default function Models() {
  const toast = useToast();
  const [tab, setTab] = useState<Tab>('installed');
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [installed, setInstalled] = useState<InstalledModelDto[]>([]);
  const [progress, setProgress] = useState<Record<string, DownloadProgress>>({});

  const refresh = useCallback(async () => {
    try {
      const [c, i] = await Promise.all([api.catalog(), api.installed()]);
      setCatalog(c);
      setInstalled(i);
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  }, [toast]);

  useEffect(() => {
    refresh();
    window.addEventListener('focus', refresh); // the tray can change the active model
    return () => window.removeEventListener('focus', refresh);
  }, [refresh]);

  useEvent('progress', (p) => {
    setProgress((m) => ({ ...m, [p.modelId]: p }));
    if (p.status === 'Installed') refresh();
  });

  const run = async (fn: () => Promise<unknown>, after?: () => void) => {
    try {
      await fn();
      after?.();
    } catch (e) {
      toast(errMsg(e), 'error');
    }
    refresh();
  };
  const actions = (id: string) => ({
    onInstall: () => run(() => api.modelInstall(id)),
    onPause: () => run(() => api.modelPause(id)),
    onResume: () => run(() => api.modelResume(id)),
    onCancel: () => run(() => api.modelCancel(id), () => setProgress(({ [id]: _, ...rest }) => rest)),
    onActivate: () => run(() => api.modelSetActive(id)),
    onDelete: () => {
      if (window.confirm('Delete this model from disk? You can download it again later.')) {
        run(() => api.modelDelete(id), () => setProgress(({ [id]: _, ...rest }) => rest));
      }
    },
  });

  const installedIds = new Set(installed.map((m) => m.id));
  const inFlight = Object.values(progress).filter((p) => !installedIds.has(p.modelId) && p.status !== 'Installed');
  const totalDisk = installed.reduce((n, m) => n + m.diskBytes, 0);
  const nameOf = (id: string) => catalog.find((c) => c.id === id)?.name ?? id;

  return (
    <div className="page">
      <header className="page-head">
        <h1>Models</h1>
        <p className="muted">Speech models run entirely on this computer.</p>
      </header>
      <div className="tabs" role="tablist">
        {TABS.map(([k, label]) => (
          <button key={k} role="tab" aria-selected={tab === k} className={`tab${tab === k ? ' tab-on' : ''}`} onClick={() => setTab(k)}>{label}</button>
        ))}
      </div>

      {tab === 'installed' && (
        <section className="list">
          {installed.length === 0 && inFlight.length === 0 && (
            <div className="empty">
              <div className="empty-title">No models installed yet</div>
              <p>Download a model from the catalog to start dictating.</p>
              <button className="btn btn-primary" onClick={() => setTab('catalog')}>Browse catalog</button>
            </div>
          )}
          {installed.map((m) => (
            <ModelRow key={m.id} id={m.id} name={m.name} meta={meta(m, fmtBytes(m.diskBytes))} installed active={m.active} progress={progress[m.id]} {...actions(m.id)} />
          ))}
          {inFlight.map((p) => (
            <ModelRow key={p.modelId} id={p.modelId} name={nameOf(p.modelId)} meta={typeof p.status === 'object' ? 'Failed' : 'Downloading'} installed={false} active={false} progress={p} {...actions(p.modelId)} />
          ))}
          {installed.length > 0 && <div className="foot muted">Disk usage: {fmtBytes(totalDisk)}</div>}
        </section>
      )}

      {tab === 'catalog' && (
        <section className="list">
          {catalog.map((e) => (
            <ModelRow key={e.id} id={e.id} name={e.name} meta={meta(e) + (e.notes ? ` · ${e.notes}` : '')} installed={installedIds.has(e.id)} active={installed.find((m) => m.id === e.id)?.active ?? false} progress={progress[e.id]} {...actions(e.id)} />
          ))}
        </section>
      )}

      {tab === 'hf' && <HfSearch progress={progress} installedIds={installedIds} onInstalled={refresh} />}
    </div>
  );
}

function HfSearch({ progress, installedIds, onInstalled }: { progress: Record<string, DownloadProgress>; installedIds: Set<string>; onInstalled: () => void }) {
  const toast = useToast();
  const [q, setQ] = useState('');
  const [results, setResults] = useState<HfRepoSummary[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [inspections, setInspections] = useState<Record<string, Inspection>>({});
  const seq = useRef(0);

  useEffect(() => {
    const term = q.trim();
    if (!term) {
      seq.current++; // drop any in-flight response
      setLoading(false);
      setResults(null);
      return;
    }
    const t = setTimeout(async () => {
      const n = ++seq.current;
      setLoading(true);
      try {
        const r = await api.hfSearch(term);
        if (n === seq.current) setResults(r);
      } catch (e) {
        toast(errMsg(e), 'error');
      } finally {
        if (n === seq.current) setLoading(false);
      }
    }, 300);
    return () => clearTimeout(t);
  }, [q, toast]);

  const inspect = async (repo: string) => {
    try {
      const i = await api.hfInspect(repo);
      setInspections((m) => ({ ...m, [repo]: i }));
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  const install = async (entry: CatalogEntry) => {
    try {
      await api.modelInstallEntry(entry);
      onInstalled();
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };

  return (
    <section>
      <input className="input" placeholder="Search Hugging Face for sherpa-onnx models…" value={q} onChange={(e) => setQ(e.target.value)} />
      <div className="list">
        {results === null && <div className="empty"><p>Search for a repository, then inspect it to check it is compatible.</p></div>}
        {results?.length === 0 && !loading && <div className="empty"><p>No repositories found.</p></div>}
        {results?.map((r) => {
          const ins = inspections[r.repo];
          return (
            <div className="row" key={r.repo}>
              <div className="row-main">
                <div className="row-title">{r.repo}</div>
                <div className="row-sub">{r.downloads.toLocaleString()} downloads · {r.likes} likes</div>
                {ins?.kind === 'incompatible' && <div className="row-error">{ins.reason}</div>}
                {ins?.kind === 'compatible' && <div className="row-sub">Compatible · {ins.engine} · {size(ins)}</div>}
                {ins?.kind === 'compatible' && progress[ins.id] && <div className="row-sub">{typeof progress[ins.id].status === 'string' ? progress[ins.id].status as string : 'Failed'}</div>}
              </div>
              <div className="row-actions">
                {!ins && <button className="btn" onClick={() => inspect(r.repo)}>Inspect</button>}
                {ins?.kind === 'compatible' && !installedIds.has(ins.id) && (
                  <button className="btn btn-primary" onClick={() => { const { kind: _k, ...entry } = ins; install(entry); }}>Install</button>
                )}
                {ins?.kind === 'compatible' && installedIds.has(ins.id) && <span className="badge">Installed</span>}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
