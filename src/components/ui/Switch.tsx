export default function Switch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={checked} aria-label={label} className="ui-switch" onClick={() => onChange(!checked)}>
      <span className="ui-switch-knob" />
    </button>
  );
}
