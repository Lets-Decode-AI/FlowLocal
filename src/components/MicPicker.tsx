import { useCallback, useEffect, useState } from 'react';
import { api } from '../ipc';

interface MicPickerProps {
  /** Device name; `null` = system default. */
  value: string | null;
  onChange: (mic: string | null) => void;
}

/** Picks the input device. The list refreshes on focus, so a mic plugged in later shows up. */
export default function MicPicker({ value, onChange }: MicPickerProps) {
  const [mics, setMics] = useState<string[]>([]);
  const refresh = useCallback(() => { api.micList().then(setMics, () => {}); }, []);
  useEffect(refresh, [refresh]);
  return (
    <select className="ui-select" aria-label="Microphone" value={value ?? ''} onFocus={refresh} onChange={(e) => onChange(e.target.value || null)}>
      <option value="">System default</option>
      {mics.map((m) => <option key={m} value={m}>{m}</option>)}
      {value && !mics.includes(value) && <option value={value}>{value} (not connected)</option>}
    </select>
  );
}
