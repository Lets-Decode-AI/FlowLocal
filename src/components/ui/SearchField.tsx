import Icon from './Icon';

/** macOS-style search field: magnifier inside a rounded fill. */
export default function SearchField({ value, onChange, placeholder = 'Search', label }: { value: string; onChange: (v: string) => void; placeholder?: string; label: string }) {
  return (
    <label className="ui-search">
      <Icon name="search" size={14} />
      <input type="search" aria-label={label} placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} autoComplete="off" spellCheck={false} />
    </label>
  );
}
