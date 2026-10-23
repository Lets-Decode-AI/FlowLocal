export function fmtBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}

export function fmtEta(secs: number | null): string {
  if (secs == null || !isFinite(secs)) return '';
  if (secs < 60) return `${Math.max(1, Math.round(secs))}s left`;
  const m = Math.floor(secs / 60);
  return m < 60 ? `${m}m ${Math.round(secs % 60)}s left` : `${Math.floor(m / 60)}h ${m % 60}m left`;
}

export const fmtClock = (ms: number): string => new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

/** "Today", "Yesterday", or a full date like "Monday, October 19". */
export function dayLabel(ms: number, now: Date = new Date()): string {
  const d = new Date(ms);
  if (d.toDateString() === now.toDateString()) return 'Today';
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return 'Yesterday';
  return d.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' });
}

export function languageLabel(langs: string[]): string {
  if (langs.length === 1) {
    try {
      return new Intl.DisplayNames(['en'], { type: 'language' }).of(langs[0]) ?? langs[0];
    } catch {
      return langs[0];
    }
  }
  return `${langs.length} languages`;
}

export const errMsg = (e: unknown): string => (typeof e === 'string' ? e : e instanceof Error ? e.message : String(e));
