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
  const page = useRef(0);
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
    page.current = 0;
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
      const r = await api.historyList(page.current + 1, q);
      if (g !== gen.current) return;
      page.current += 1;
      setItems((x) => [...x, ...r.filter((n) => !x.some((o) => o.id === n.id))]);
      setHasMore(r.length === PAGE_SIZE);
    } catch (e) {
      toast(errMsg(e), 'error');
    } finally {
      busy.current = false;
    }
  }, [hasMore, q, toast]);

  useEffect(() => {
    const el = sentinel.current;
    if (!el || !hasMore || typeof IntersectionObserver === 'undefined') return;
    const io = new IntersectionObserver((e) => e[0]?.isIntersecting && loadMore(), { rootMargin: '200px' });
    io.observe(el);
    return () => io.disconnect();
  }, [hasMore, loadMore, items.length]);

  // Live prepend: fetch the newest page and add what we have not seen.
  useEvent('transcriptAdded', async () => {
    try {
      const r = await api.historyList(0, q);
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
