import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';

export type ThemePref = 'system' | 'light' | 'dark';
export type Theme = 'light' | 'dark';

const KEY = 'flowlocal.theme';
const media = () => window.matchMedia?.('(prefers-color-scheme: dark)');
const listeners = new Set<() => void>();

export function themePref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    if (v === 'light' || v === 'dark') return v;
  } catch { /* storage unavailable: fall back to the system */ }
  return 'system';
}

export function resolvedTheme(pref = themePref()): Theme {
  return pref === 'system' ? (media()?.matches ? 'dark' : 'light') : pref;
}

/** Applies the theme to the page and the native window (vibrancy, traffic lights). */
export function applyTheme() {
  const pref = themePref();
  document.documentElement.dataset.theme = resolvedTheme(pref);
  try {
    getCurrentWindow().setTheme(pref === 'system' ? null : pref).catch(() => {});
  } catch { /* not running inside Tauri */ }
  listeners.forEach((l) => l());
}

export function setThemePref(pref: ThemePref) {
  try {
    if (pref === 'system') localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, pref);
  } catch { /* the choice lasts for this session only */ }
  applyTheme();
}

/** Call once at startup: applies the saved theme and follows the OS while set to System. */
export function initTheme() {
  applyTheme();
  media()?.addEventListener('change', () => { if (themePref() === 'system') applyTheme(); });
}

export function useTheme(): { pref: ThemePref; theme: Theme; setPref: (p: ThemePref) => void } {
  const [, bump] = useState(0);
  useEffect(() => {
    const l = () => bump((n) => n + 1);
    listeners.add(l);
    return () => { listeners.delete(l); };
  }, []);
  return { pref: themePref(), theme: resolvedTheme(), setPref: setThemePref };
}
