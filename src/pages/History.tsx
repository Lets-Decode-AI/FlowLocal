import { useCallback, useEffect, useRef, useState } from 'react';
import { api, type Transcript } from '../ipc';
import { useToast } from '../components/Toasts';
import Button from '../components/ui/Button';
import Group from '../components/ui/Group';
import Icon from '../components/ui/Icon';
import IconButton from '../components/ui/IconButton';
import { keycapsFor } from '../components/ui/Keycap';
import SearchField from '../components/ui/SearchField';
import { dayLabel, errMsg, fmtClock } from '../lib/format';
import { useEvent } from '../lib/hooks';
import { useBinding } from '../lib/useBinding';

const PAGE_SIZE = 50;

const metaLine = (t: Transcript) =>
  [t.appName, fmtClock(t.createdAt), `${t.wordCount} ${t.wordCount === 1 ? 'word' : 'words'}`].filter(Boolean).join(' · ');

/** Consecutive transcripts that fall on the same calendar day. Input is newest first. */
function groupByDay(items: Transcript[]): { key: string; title: string; items: Transcript[] }[] {
  const out: { key: string; title: string; items: Transcript[] }[] = [];
  for (const t of items) {
    const key = new Date(t.createdAt).toDateString();
    const last = out[out.length - 1];
    if (last && last.key === key) last.items.push(t);
    else out.push({ key, title: dayLabel(t.createdAt), items: [t] });
  }
  return out;
}

export default function History() {
  const toast = useToast();
  const { binding } = useBinding();
  const [input, setInput] = useState('');
  const [query, setQuery] = useState('');
  const [items, setItems] = useState<Transcript[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [open, setOpen] = useState<number | null>(null);
  const busy = useRef(false);
  const gen = useRef(0);
  const sentinel = useRef<HTMLDivElement>(null);
  const q = query.trim() || undefined;

  useEffect(() => {
    const t = setTimeout(() => setQuery(input), 300);
    return () => clearTimeout(t);
  }, [input]);

  // (Re)load from page 0 whenever the debounced query changes.
  useEffect(() => {
    const g = ++gen.current;
    busy.current = true;
    api.historyList(0, q).then(
      (r) => {
        if (g !== gen.current) return;
        setItems(r);
        setHasMore(r.length === PAGE_SIZE);
        setLoaded(true);
        busy.current = false;
      },
      (e) => {
        if (g !== gen.current) return;
        busy.current = false;
        toast(errMsg(e), 'error');
      },
    );
  }, [q, toast]);

  const loadMore = useCallback(async () => {
    if (busy.current || !hasMore) return;
    busy.current = true;
    const g = gen.current;
    try {
      // Page by how much we hold, not by a page counter: local deletes shift server offsets.
      // Re-fetching an overlapping page is safe because rows are deduped by id.
      const r = await api.historyList(Math.floor(items.length / PAGE_SIZE), q);
      if (g !== gen.current) return;
      const fresh = r.filter((n) => !items.some((o) => o.id === n.id));
      setItems((x) => [...x, ...fresh.filter((n) => !x.some((o) => o.id === n.id))]);
      setHasMore(r.length === PAGE_SIZE && fresh.length > 0);
    } catch (e) {
      if (g === gen.current) toast(errMsg(e), 'error');
    } finally {
      if (g === gen.current) busy.current = false;
    }
  }, [hasMore, items, q, toast]);

  useEffect(() => {
    const el = sentinel.current;
    if (!el || !hasMore || typeof IntersectionObserver === 'undefined') return;
    const io = new IntersectionObserver((e) => e[0]?.isIntersecting && loadMore(), { rootMargin: '200px' });
    io.observe(el);
    return () => io.disconnect();
  }, [hasMore, loadMore, items.length]);

  // Live prepend: fetch the newest page and add what we have not seen.
  useEvent('transcriptAdded', async () => {
    const g = gen.current;
    try {
      const r = await api.historyList(0, q);
      if (g !== gen.current) return;
      setItems((x) => {
        const fresh = r.filter((n) => !x.some((o) => o.id === n.id));
        return fresh.length ? [...fresh, ...x] : x;
      });
    } catch {
      /* next reload will catch up */
    }
  });

  const copy = async (t: Transcript) => {
    try {
      await navigator.clipboard.writeText(t.text);
      toast('Copied');
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };
  const del = async (t: Transcript) => {
    try {
      await api.historyDelete(t.id);
      setItems((x) => x.filter((i) => i.id !== t.id));
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  };

  return (
    <div className="page">
      <header className="page-head page-head-row">
        <h1>History</h1>
        <SearchField label="Search dictations" value={input} onChange={setInput} />
      </header>
      {loaded && items.length === 0 && (
        <div className="empty">
          <span className="empty-icon" aria-hidden="true"><Icon name={q ? 'search' : 'mic'} size={28} /></span>
          <div className="empty-title">{q ? 'No results' : 'No dictations yet'}</div>
          <p className="muted">{q ? 'Try a different search.' : <>Hold {keycapsFor(binding)} in any app to start.</>}</p>
        </div>
      )}
      {groupByDay(items).map((g) => (
        <Group key={g.key} title={g.title}>
          {g.items.map((t) => (
            <div className="ui-row hist-row" key={t.id}>
              <span className="ui-row-text">
                <button type="button" className={`hist-text selectable${open === t.id ? ' hist-open' : ''}`} aria-expanded={open === t.id} onClick={() => setOpen(open === t.id ? null : t.id)}>
                  {t.text}
                </button>
                <span className="hist-meta">{metaLine(t)}</span>
              </span>
              <span className="hist-actions">
                <IconButton icon="copy" label="Copy" onClick={() => copy(t)} />
                <IconButton icon="trash" label="Delete" tone="destructive" onClick={() => del(t)} />
              </span>
            </div>
          ))}
        </Group>
      ))}
      {hasMore && (
        <div ref={sentinel} className="foot">
          <Button onClick={loadMore}>Load more</Button>
        </div>
      )}
    </div>
  );
}
