import { useRef, type KeyboardEvent } from 'react';

export interface SegmentedOption<T extends string> { value: T; label: string }

export default function Segmented<T extends string>({ options, value, onChange }: { options: SegmentedOption<T>[]; value: T; onChange: (v: T) => void }) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const onKeyDown = (e: KeyboardEvent) => {
    const cur = options.findIndex((o) => o.value === value);
    const n = options.length;
    const next = e.key === 'ArrowRight' ? (cur + 1) % n : e.key === 'ArrowLeft' ? (cur - 1 + n) % n : e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : -1;
    if (next < 0) return;
    e.preventDefault();
    onChange(options[next].value);
    refs.current[next]?.focus();
  };
  return (
    <div className="ui-segmented" role="tablist" onKeyDown={onKeyDown}>
      {options.map((o, i) => (
        <button
          key={o.value} ref={(el) => { refs.current[i] = el; }} type="button" role="tab" aria-selected={o.value === value}
          tabIndex={o.value === value ? 0 : -1} className={`ui-seg${o.value === value ? ' ui-seg-on' : ''}`} onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
