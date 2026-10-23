import { useState } from 'react';
import type { DownloadProgress } from '../../ipc';
import { useEvent } from '../../lib/hooks';

const ACTIVE = ['Queued', 'Downloading', 'Paused', 'Verifying'];

/** Latest progress per model, kept above the steps so it survives Back and Continue. */
export function useDownloads() {
  const [byId, setById] = useState<Record<string, DownloadProgress>>({});
  useEvent('progress', (p) => setById((m) => ({ ...m, [p.modelId]: p })));
  const clear = (id: string) => setById(({ [id]: _drop, ...rest }) => rest);
  const activeId = Object.values(byId).find((p) => typeof p.status === 'string' && ACTIVE.includes(p.status))?.modelId ?? null;
  return { byId, clear, activeId };
}

export type Downloads = ReturnType<typeof useDownloads>;
