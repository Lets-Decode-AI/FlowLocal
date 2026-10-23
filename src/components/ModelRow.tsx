import type { DownloadProgress } from '../ipc';
import { fmtBytes, fmtEta } from '../lib/format';
import ProgressBar from './ProgressBar';

export interface ModelRowProps {
  id: string;
  name: string;
  meta: string;
  installed: boolean;
  active: boolean;
  progress?: DownloadProgress;
  onInstall: () => void;
  onPause: () => void;
  onResume: () => void;
  onCancel: () => void;
  onActivate: () => void;
  onDelete: () => void;
}

export default function ModelRow(p: ModelRowProps) {
  const st = p.progress?.status;
  const failed = typeof st === 'object' && st !== null ? st.Failed : null;
  const busy = st === 'Downloading' || st === 'Queued' || st === 'Verifying';
  const paused = st === 'Paused';
  const pct = p.progress && p.progress.total > 0 ? (p.progress.bytesDone / p.progress.total) * 100 : 0;
  return (
    <div className="row" data-testid="model-row">
      <div className="row-main">
        <div className="row-title">
          {p.name}
          {p.active && <span className="badge">Active</span>}
        </div>
        <div className="row-sub">{p.meta}</div>
        {(busy || paused) && p.progress && (
          <div className="row-progress">
            <ProgressBar value={pct} label={`${p.name} download`} />
            <div className="row-sub">
              {st === 'Verifying' ? 'Verifying…' : paused ? 'Paused' : st === 'Queued' ? 'Queued' : `${fmtBytes(p.progress.bytesDone)} of ${fmtBytes(p.progress.total)} · ${fmtBytes(p.progress.bytesPerSec)}/s ${fmtEta(p.progress.etaSecs)}`}
            </div>
          </div>
        )}
        {failed && <div className="row-error">Failed: {failed}</div>}
      </div>
      <div className="row-actions">
        {p.installed && !p.active && <button className="btn" onClick={p.onActivate}>Set active</button>}
        {p.installed && <button className="btn btn-ghost" onClick={p.onDelete}>Delete</button>}
        {!p.installed && !busy && !paused && <button className="btn btn-primary" onClick={p.onInstall}>{failed ? 'Retry' : 'Download'}</button>}
        {busy && st !== 'Verifying' && <button className="btn" onClick={p.onPause}>Pause</button>}
        {paused && <button className="btn btn-primary" onClick={p.onResume}>Resume</button>}
        {(busy || paused) && <button className="btn btn-ghost" onClick={p.onCancel}>Cancel</button>}
      </div>
    </div>
  );
}
