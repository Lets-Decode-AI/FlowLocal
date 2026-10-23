import type { ReactNode } from 'react';
import type { DownloadProgress } from '../ipc';
import { fmtBytes, fmtEta } from '../lib/format';
import ProgressBar from './ProgressBar';
import Button from './ui/Button';
import Icon from './ui/Icon';
import Menu from './ui/Menu';

export interface ModelRowProps {
  id: string;
  name: string;
  /** Second line, for example "English · 661 MB". */
  meta: string;
  /** Optional third line. */
  note?: ReactNode;
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
    <div className="ui-row model-row" data-testid="model-row">
      <span className="ui-row-text">
        <span className="ui-row-label">{p.name}</span>
        <span className="ui-row-detail">{p.meta}</span>
        {p.note && <span className="ui-row-detail">{p.note}</span>}
        {(busy || paused) && p.progress && (
          <span className="model-progress">
            <ProgressBar value={pct} label={`${p.name} download`} />
            <span className="ui-row-detail">
              {st === 'Verifying' ? 'Verifying' : paused ? 'Paused' : st === 'Queued' ? 'Waiting to start' : `${fmtBytes(p.progress.bytesDone)} of ${fmtBytes(p.progress.total)} · ${fmtBytes(p.progress.bytesPerSec)}/s ${fmtEta(p.progress.etaSecs)}`}
            </span>
          </span>
        )}
        {failed && <span className="ui-row-detail ui-row-error">Download failed: {failed}</span>}
      </span>
      <span className="ui-row-accessory">
        {p.installed && p.active && <span className="ui-ok"><Icon name="check" size={14} />In use</span>}
        {p.installed && (
          <Menu
            label={`Options for ${p.name}`}
            items={[
              ...(p.active ? [] : [{ label: 'Use', onSelect: p.onActivate }]),
              { label: 'Delete', onSelect: p.onDelete, destructive: true },
            ]}
          />
        )}
        {!p.installed && !busy && !paused && <Button variant="primary" onClick={p.onInstall}>{failed ? 'Retry' : 'Download'}</Button>}
        {busy && st !== 'Verifying' && <Button onClick={p.onPause}>Pause</Button>}
        {paused && <Button variant="primary" onClick={p.onResume}>Resume</Button>}
        {(busy || paused) && <Button variant="plain" onClick={p.onCancel}>Cancel</Button>}
      </span>
    </div>
  );
}
