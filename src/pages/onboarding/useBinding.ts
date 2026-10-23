import { useCallback, useEffect, useState } from 'react';
import { api, type Binding, type Settings } from '../../ipc';

const DEFAULT: Binding = { keys: ['Fn'] };

/** The saved dictation hotkey, with a setter that persists it. */
export function useBinding(onError?: (e: unknown) => void) {
  const [settings, setSettings] = useState<Settings | null>(null);
  useEffect(() => {
    api.settingsGet().then(setSettings, () => {});
  }, []);
  const save = useCallback(async (hotkey: Binding) => {
    if (!settings) return;
    const next = { ...settings, hotkey };
    setSettings(next);
    try {
      await api.settingsSet(next);
    } catch (e) {
      onError?.(e);
    }
  }, [settings, onError]);
  return { binding: settings?.hotkey ?? DEFAULT, save };
}
