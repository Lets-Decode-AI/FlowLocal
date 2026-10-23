export interface SegmentedOption<T extends string> { value: T; label: string }

export default function Segmented<T extends string>({ options, value, onChange }: { options: SegmentedOption<T>[]; value: T; onChange: (v: T) => void }) {
  return (
    <div className="ui-segmented" role="tablist">
      {options.map((o) => (
        <button key={o.value} type="button" role="tab" aria-selected={o.value === value} className={`ui-seg${o.value === value ? ' ui-seg-on' : ''}`} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}
