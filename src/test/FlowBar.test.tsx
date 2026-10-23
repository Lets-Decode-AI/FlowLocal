import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationState } from '../ipc';

const handlers: Record<string, (p: any) => void> = {};
let showFlowbar = true;
vi.mock('../ipc', () => ({
  api: {
    getState: vi.fn(async () => ({ kind: 'idle' })),
    settingsGet: vi.fn(async () => ({ showFlowbar })),
  },
  on: vi.fn(async (name: string, cb: (p: any) => void) => {
    handlers[name] = cb;
    return () => {};
  }),
}));

import FlowBar from '../flowbar/FlowBar';

const emit = (name: string, p: unknown) => act(() => handlers[name]?.(p));
const setState = (s: DictationState) => emit('state', s);

async function mount() {
  render(<FlowBar />);
  await act(async () => {});
}

beforeEach(() => {
  showFlowbar = true;
  for (const k of Object.keys(handlers)) delete handlers[k];
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('FlowBar', () => {
  it('renders_idle_handle', async () => {
    await mount();
    expect(screen.getByTestId('flowbar-idle')).toBeTruthy();
  });

  it('renders_waveform_when_recording', async () => {
    await mount();
    setState({ kind: 'recording', handsFree: false });
    for (const l of [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1, 0.9, 0.8, 0.7, 0.6, 0.5, 0.9]) emit('level', l);
    const bars = screen.getAllByTestId('wave-bar');
    expect(bars).toHaveLength(16);
    const levels = bars.map((b) => Number(b.getAttribute('data-level')));
    expect(Math.max(...levels)).toBeGreaterThan(Math.min(...levels));
    expect(Math.max(...levels)).toBeGreaterThan(0.5);
    expect(screen.queryByTestId('hands-free-dot')).toBeNull();
    setState({ kind: 'recording', handsFree: true });
    expect(screen.getByTestId('hands-free-dot')).toBeTruthy();
  });

  it('renders_error_message', async () => {
    await mount();
    setState({ kind: 'error', message: 'No model installed' });
    expect(screen.getByText('No model installed')).toBeTruthy();
    expect(screen.getByTestId('flowbar-pill').getAttribute('data-kind')).toBe('error');
  });

  it('hidden_when_flowbar_disabled_and_idle', async () => {
    showFlowbar = false;
    await mount();
    expect(screen.queryByTestId('flowbar-idle')).toBeNull();
    expect(screen.queryByTestId('flowbar-pill')).toBeNull();
    setState({ kind: 'recording', handsFree: false });
    expect(screen.getByTestId('flowbar-pill')).toBeTruthy();
  });

  it('shows_done_check_after_insert', async () => {
    await mount();
    vi.useFakeTimers();
    setState({ kind: 'transcribing' });
    expect(screen.getByTestId('flowbar-pill').getAttribute('data-kind')).toBe('transcribing');
    setState({ kind: 'inserting' });
    setState({ kind: 'idle' });
    expect(screen.getByTestId('flowbar-done')).toBeTruthy();
    act(() => void vi.advanceTimersByTime(650));
    expect(screen.queryByTestId('flowbar-done')).toBeNull();
    expect(screen.getByTestId('flowbar-idle')).toBeTruthy();
  });

  it('shows_toast_message_in_the_pill_for_about_three_seconds', async () => {
    await mount();
    vi.useFakeTimers();
    emit('toast', { level: 'warning', message: 'Grant Accessibility access to turn on the shortcut.' });
    expect(screen.getByText('Grant Accessibility access to turn on the shortcut.')).toBeTruthy();
    const pill = screen.getByTestId('flowbar-pill');
    expect(pill.getAttribute('data-kind')).toBe('toast');
    expect(pill.className).toContain('fb-toast-warning');
    act(() => void vi.advanceTimersByTime(2900));
    expect(screen.queryByText('Grant Accessibility access to turn on the shortcut.')).toBeTruthy();
    act(() => void vi.advanceTimersByTime(200));
    expect(screen.queryByText('Grant Accessibility access to turn on the shortcut.')).toBeNull();
    expect(screen.getByTestId('flowbar-idle')).toBeTruthy();
  });

  it('toast_is_visible_even_when_flowbar_is_hidden_by_setting', async () => {
    showFlowbar = false;
    await mount();
    emit('toast', { level: 'error', message: 'Could not save to history' });
    expect(screen.getByText('Could not save to history')).toBeTruthy();
  });

  it('copied_only_shows_message_instead_of_check', async () => {
    await mount();
    vi.useFakeTimers();
    setState({ kind: 'transcribing' });
    setState({ kind: 'inserting' });
    emit('toast', { level: 'warning', message: 'Copied. Press \u2318V to paste.' });
    setState({ kind: 'idle' });
    emit('transcriptAdded', { id: 1 });
    expect(screen.queryByTestId('flowbar-done')).toBeNull();
    expect(screen.getByText('Copied. Press \u2318V to paste.')).toBeTruthy();
  });

  it('transcribing_shows_three_pulsing_dots', async () => {
    await mount();
    setState({ kind: 'transcribing' });
    expect(screen.getAllByTestId('transcribing-dot')).toHaveLength(3);
  });

  it('error_text_wraps_to_two_lines_instead_of_truncating', async () => {
    await mount();
    setState({ kind: 'error', message: 'The microphone was disconnected.' });
    const text = screen.getByText('The microphone was disconnected.');
    expect(text.className).toContain('fb-clamp');
    expect(screen.getByTestId('flowbar-pill').querySelector('svg')).toBeTruthy();
  });

  it('toast_pill_has_an_icon_for_warnings_and_errors', async () => {
    await mount();
    emit('toast', { level: 'error', message: 'Nope' });
    expect(screen.getByTestId('flowbar-pill').querySelector('svg')).toBeTruthy();
  });

  it('recording_takes_priority_over_a_toast', async () => {
    await mount();
    emit('toast', { level: 'warning', message: 'hello' });
    setState({ kind: 'recording', handsFree: false });
    expect(screen.queryByText('hello')).toBeNull();
    expect(screen.getByTestId('flowbar-pill').getAttribute('data-kind')).toBe('recording');
  });
});
