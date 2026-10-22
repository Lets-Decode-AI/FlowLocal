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
});
