import { useCallback, useEffect, useRef, useState } from 'react';
import { api, type Transcript } from '../ipc';
import { useToast } from '../components/Toasts';
import { errMsg, fmtTime } from '../lib/format';
import { useEvent } from '../lib/hooks';

const PAGE_SIZE = 50;

export default function History() {
  const toast = useToast();
  const [input, setInput] = useState('');
  const [query, setQuery] = useState('');
  const [items, setItems] = useState<Transcript[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [hasMore, setHasMore] = useState(false);
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
      toast('Copied to clipboard');
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
      <header className="page-head">
        <h1>History</h1>
        <p className="muted">Everything you have dictated, stored only on this device.</p>
      </header>
      <input className="input" placeholder="Search transcripts…" value={input} onChange={(e) => setInput(e.target.value)} />
      <section className="list">
        {loaded && items.length === 0 && (
          <div className="empty">
            <div className="empty-title">{q ? 'No matches' : 'Nothing dictated yet'}</div>
            <p>{q ? 'Try a different search.' : 'Hold your hotkey in any app and start talking. Your transcripts will show up here.'}</p>
          </div>
        )}
        {items.map((t) => (
          <article className="row row-top" key={t.id}>
            <div className="row-main">
              <div className="row-text">{t.text}</div>
              <div className="row-sub">{[t.appName, fmtTime(t.createdAt), `${t.wordCount} ${t.wordCount === 1 ? 'word' : 'words'}`].filter(Boolean).join(' · ')}</div>
            </div>
            <div className="row-actions">
              <button className="btn btn-ghost" onClick={() => copy(t)}>Copy</button>
              <button className="btn btn-ghost" onClick={() => del(t)}>Delete</button>
            </div>
          </article>
        ))}
        {hasMore && (
          <div ref={sentinel} className="foot">
            <button className="btn" onClick={loadMore}>Load more</button>
          </div>
        )}
      </section>
    </div>
  );
}
