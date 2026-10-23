import { useEffect, useRef, useState } from 'react';
import { api, type CatalogEntry, type DownloadProgress, type Hardware } from '../../ipc';
import ProgressBar from '../../components/ProgressBar';
import { useToast } from '../../components/Toasts';
import Button from '../../components/ui/Button';
import Group from '../../components/ui/Group';
import Icon from '../../components/ui/Icon';
import Row from '../../components/ui/Row';
import { errMsg, fmtBytes } from '../../lib/format';
import { useEvent } from '../../lib/hooks';
import { StepFrame, type StepProps } from './Shell';

const sizeOf = (e: CatalogEntry) => fmtBytes(e.files.reduce((n, f) => n + f.size, 0));

function languageLabel(langs: string[]): string {
  if (langs.length === 1) {
    try {
      return new Intl.DisplayNames(['en'], { type: 'language' }).of(langs[0]) ?? langs[0];
    } catch {
      return langs[0];
    }
  }
  return `${langs.length} languages`;
}

function hardwareLabel(h: Partial<Hardware> | undefined): string {
  if (h?.appleSilicon) return 'Apple silicon';
  if (h?.totalRamGb) return `${Math.round(h.totalRamGb)} GB of memory`;
  return 'this computer';
}

function timeLeft(secs: number | null): string {
  if (secs == null || !isFinite(secs)) return '';
  if (secs < 60) return ' · less than a minute left';
  return ` · about ${Math.round(secs / 60)} min left`;
}

export default function Model({ onNext, onBack }: StepProps) {
  const toast = useToast();
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [recommended, setRecommended] = useState<{ id: string; hardware?: Partial<Hardware> } | null>(null);
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
        setRecommended({ id: r.id, hardware: r.hardware });
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

  const run = async (f: () => Promise<void>, after?: () => void) => {
    try {
      await f();
      after?.();
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  const download = () => {
    if (!chosen) return;
    setProgress(null);
    run(() => api.modelInstall(chosen));
  };
  // The recommended model may already be installed without being active.
  const proceed = () => chosen && run(() => api.modelSetActive(chosen), onNext);
  const choose = (id: string) => {
    setChosen(id);
    setProgress(null);
    setPicking(false);
  };

  const why = entry && (recommended?.id === entry.id
    ? `Recommended for ${languageLabel(entry.languages)} on ${hardwareLabel(recommended.hardware)}`
    : entry.notes);
  const accessory = isInstalled ? (
    <span className="onb-allowed"><Icon name="check" size={14} />Ready</span>
  ) : paused || downloading ? null : (
    <Button variant="primary" onClick={download}>{failed ? 'Retry' : 'Download'}</Button>
  );

  return (
    <StepFrame onBack={onBack} onNext={proceed} nextDisabled={!isInstalled}>
      <h1>Speech model</h1>
      <p className="muted">The model turns your voice into text on this device. You can switch models later.</p>
      {entry && (
        <Group>
          <Row
            icon="models" label={entry.name}
            detail={<>{why && <>{why}<br /></>}{sizeOf(entry)} · {languageLabel(entry.languages)}</>}
            accessory={accessory}
          />
          {(downloading || paused || failed) && (
            <div className="ui-row onb-dl">
              {(downloading || paused) && progress && (
                <>
                  <ProgressBar value={pct} label="Model download" />
                  <div className="onb-dl-line">
                    <span className="muted">
                      {st === 'Verifying' ? 'Verifying' : paused ? 'Paused' : `${fmtBytes(progress.bytesDone)} of ${fmtBytes(progress.total)}${timeLeft(progress.etaSecs)}`}
                    </span>
                    <span className="onb-dl-actions">
                      {paused
                        ? <Button variant="primary" onClick={() => chosen && run(() => api.modelResume(chosen))}>Resume</Button>
                        : st !== 'Verifying' && <Button onClick={() => chosen && run(() => api.modelPause(chosen))}>Pause</Button>}
                      {st !== 'Verifying' && <Button variant="plain" onClick={() => chosen && run(() => api.modelCancel(chosen), () => setProgress(null))}>Cancel</Button>}
                    </span>
                  </div>
                </>
              )}
              {failed && <span className="onb-error">Download failed: {failed}</span>}
            </div>
          )}
        </Group>
      )}
      {!downloading && (
        <div>
          <Button variant="plain" aria-expanded={picking} onClick={() => setPicking((x) => !x)}>Choose a different model</Button>
        </div>
      )}
      {picking && (
        <Group>
          {catalog.map((c) => (
            <Row key={c.id} label={c.name} detail={c.notes || languageLabel(c.languages)} accessory={c.id === chosen ? <Icon name="check" size={14} label="Selected" /> : sizeOf(c)} onClick={() => choose(c.id)} />
          ))}
        </Group>
      )}
    </StepFrame>
  );
}
