import { useCallback, useEffect, useRef, useState } from 'react';
import { api, type CatalogEntry, type DownloadProgress, type HfRepoSummary, type Inspection, type InstalledModelDto } from '../ipc';
import ModelRow from '../components/ModelRow';
import Button from '../components/ui/Button';
import Group from '../components/ui/Group';
import Icon from '../components/ui/Icon';
import Row from '../components/ui/Row';
import Segmented from '../components/ui/Segmented';
import SearchField from '../components/ui/SearchField';
import { useToast } from '../components/Toasts';
import { errMsg, fmtBytes, languageLabel } from '../lib/format';
import { useEvent } from '../lib/hooks';

type Tab = 'installed' | 'library' | 'hf';
const TABS: { value: Tab; label: string }[] = [
  { value: 'installed', label: 'Installed' },
  { value: 'library', label: 'Library' },
  { value: 'hf', label: 'Hugging Face' },
];

const size = (e: CatalogEntry) => fmtBytes(e.files.reduce((n, f) => n + f.size, 0));
const meta = (e: CatalogEntry, bytes?: string) => `${languageLabel(e.languages)} · ${bytes ?? size(e)}`;

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

  useEvent('modelsChanged', () => void refresh());

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
      if (window.confirm('Delete this model? You can download it again later.')) {
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
      <header className="page-head page-head-row">
        <h1>Models</h1>
        <Segmented options={TABS} value={tab} onChange={setTab} />
      </header>

      {tab === 'installed' && (
        <>
          {installed.length === 0 && inFlight.length === 0 && (
            <div className="empty">
              <span className="empty-icon" aria-hidden="true"><Icon name="models" size={28} /></span>
              <div className="empty-title">No models installed</div>
              <p className="muted">Download a speech model to start dictating.</p>
              <Button variant="primary" onClick={() => setTab('library')}>Browse the library</Button>
            </div>
          )}
          {(installed.length > 0 || inFlight.length > 0) && (
            <Group footer={installed.length > 0 ? `Installed models use ${fmtBytes(totalDisk)} on this computer.` : undefined}>
              {installed.map((m) => (
                <ModelRow key={m.id} id={m.id} name={m.name} meta={meta(m, fmtBytes(m.diskBytes))} installed active={m.active} progress={progress[m.id]} {...actions(m.id)} />
              ))}
              {inFlight.map((p) => (
                <ModelRow key={p.modelId} id={p.modelId} name={nameOf(p.modelId)} meta={typeof p.status === 'object' ? 'Not installed' : 'Downloading'} installed={false} active={false} progress={p} {...actions(p.modelId)} />
              ))}
            </Group>
          )}
        </>
      )}

      {tab === 'library' && (
        <Group footer="Models run on this computer. Your audio is never uploaded.">
          {catalog.map((e) => (
            <ModelRow key={e.id} id={e.id} name={e.name} meta={meta(e)} note={e.notes || undefined} installed={installedIds.has(e.id)} active={installed.find((m) => m.id === e.id)?.active ?? false} progress={progress[e.id]} {...actions(e.id)} />
          ))}
        </Group>
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
    <>
      <SearchField label="Search Hugging Face" placeholder="Search Hugging Face" value={q} onChange={setQ} />
      {results === null && <p className="muted empty-note">Search for a sherpa-onnx model repository, then check that it works with FlowLocal.</p>}
      {results?.length === 0 && !loading && <p className="muted empty-note">No repositories found.</p>}
      {results && results.length > 0 && (
        <Group>
          {results.map((r) => {
            const ins = inspections[r.repo];
            const status = ins?.kind === 'compatible' ? progress[ins.id]?.status : undefined;
            return (
              <Row
                key={r.repo}
                label={r.repo}
                detail={
                  <>
                    {r.downloads.toLocaleString()} downloads · {r.likes} likes
                    {ins?.kind === 'incompatible' && <><br /><span className="ui-row-error">{ins.reason}</span></>}
                    {ins?.kind === 'compatible' && <><br />Works with FlowLocal · {ins.engine} · {size(ins)}</>}
                    {status && <><br />{typeof status === 'string' ? status : 'Download failed'}</>}
                  </>
                }
                accessory={
                  <>
                    {!ins && <Button onClick={() => inspect(r.repo)}>Inspect</Button>}
                    {ins?.kind === 'incompatible' && <span className="ui-warn"><Icon name="warning" size={14} />Not compatible</span>}
                    {ins?.kind === 'compatible' && !installedIds.has(ins.id) && (
                      <Button variant="primary" onClick={() => { const { kind: _k, ...entry } = ins; install(entry); }}>Install</Button>
                    )}
                    {ins?.kind === 'compatible' && installedIds.has(ins.id) && <span className="ui-ok"><Icon name="check" size={14} />Installed</span>}
                  </>
                }
              />
            );
          })}
        </Group>
      )}
    </>
  );
}
