import { useEffect, useRef, useState } from 'react';
import IconButton from './IconButton';

export interface MenuItem { label: string; onSelect: () => void; destructive?: boolean }

/** A "more" button that opens a small menu. Closes on selection, Escape or an outside click. */
export default function Menu({ label, items }: { label: string; items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false);
        trigger.current?.focus();
      }
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    <span className="ui-menu" ref={root}>
      <IconButton ref={trigger} icon="more" label={label} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)} />
      {open && (
        <div className="ui-menu-list" role="menu">
          {items.map((it) => (
            <button
              key={it.label} type="button" role="menuitem" autoFocus={items[0] === it}
              className={`ui-menu-item${it.destructive ? ' ui-menu-item-destructive' : ''}`}
              onClick={() => { setOpen(false); it.onSelect(); }}
            >
              {it.label}
            </button>
          ))}
        </div>
      )}
    </span>
  );
}
