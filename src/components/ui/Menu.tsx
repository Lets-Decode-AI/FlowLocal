import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import IconButton from './IconButton';

export interface MenuItem { label: string; onSelect: () => void; destructive?: boolean }

/** A "more" button that opens a small menu. Closes on selection, Escape, Tab or an outside click. */
export default function Menu({ label, items }: { label: string; items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) close();
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  const onKeyDown = (e: KeyboardEvent) => {
    const cur = itemRefs.current.findIndex((el) => el === document.activeElement);
    const n = items.length;
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if (e.key === 'Tab') {
      close();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const next = e.key === 'ArrowDown' ? (cur + 1) % n : (cur - 1 + n) % n;
      itemRefs.current[next]?.focus();
    }
  };

  return (
    <span className="ui-menu" ref={root} onKeyDown={open ? onKeyDown : undefined}>
      <IconButton ref={trigger} icon="more" label={label} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)} />
      {open && (
        <div className="ui-menu-list" role="menu">
          {items.map((it, i) => (
            <button
              key={it.label} ref={(el) => { itemRefs.current[i] = el; }} type="button" role="menuitem" autoFocus={i === 0}
              className={`ui-menu-item${it.destructive ? ' ui-menu-item-destructive' : ''}`}
              onClick={() => { close(); it.onSelect(); }}
            >
              {it.label}
            </button>
          ))}
        </div>
      )}
    </span>
  );
}
