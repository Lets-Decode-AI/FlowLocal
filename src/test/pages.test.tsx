import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const handlers: Record<string, Array<(p: any) => void>> = {};
const entry = (id: string, name: string) => ({
  id, name, hf_repo: `o/${id}`, revision: 'main', files: [{ path: 'a.onnx', size: 100_000_000, sha256: 'x' }],
  engine: 'transducer', languages: ['en'], min_ram_gb: 4, notes: '',
});
const settings = {
  hotkey: { keys: ['Fn'] }, mic: null, showFlowbar: true, idleUnloadMin: 10, preload: true, launchAtLogin: false, onboarded: false,
};

vi.mock('../ipc', () => ({
  api: {
    permissionsStatus: vi.fn(), openPermissionSettings: vi.fn(async () => {}), requestMicrophone: vi.fn(async () => {}),
    catalog: vi.fn(), installed: vi.fn(), recommend: vi.fn(), hfSearch: vi.fn(), hfInspect: vi.fn(),
    modelInstall: vi.fn(async () => {}), modelInstallEntry: vi.fn(async () => {}), modelPause: vi.fn(async () => {}),
    modelResume: vi.fn(async () => {}), modelCancel: vi.fn(async () => {}), modelDelete: vi.fn(async () => {}),
    modelSetActive: vi.fn(async () => {}), historyList: vi.fn(), historyDelete: vi.fn(async () => {}),
    settingsGet: vi.fn(), settingsSet: vi.fn(async () => {}), hfTokenSet: vi.fn(async () => {}),
    dictionaryList: vi.fn(async () => []), dictionaryAdd: vi.fn(async () => 1), dictionaryDelete: vi.fn(async () => {}),
    micList: vi.fn(async () => ['Built-in Mic']),
  },
  on: vi.fn(async (name: string, cb: (p: any) => void) => {
    (handlers[name] ||= []).push(cb);
    return () => {};
  }),
}));

import { api } from '../ipc';
import Onboarding from '../pages/Onboarding';
import Models from '../pages/Models';
import History from '../pages/History';
import Settings from '../pages/Settings';

const a = api as any;
const emit = (name: string, p: unknown) => act(() => handlers[name]?.forEach((h) => h(p)));
const flush = () => act(async () => {});
const tick = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

beforeEach(() => {
  vi.useFakeTimers();
  for (const k of Object.keys(handlers)) delete handlers[k];
  vi.clearAllMocks();
  a.permissionsStatus.mockResolvedValue({ microphone: 'granted', accessibility: 'granted' });
  a.catalog.mockResolvedValue([entry('m1', 'Parakeet Small'), entry('m2', 'Whisper Tiny')]);
  a.installed.mockResolvedValue([]);
  a.recommend.mockResolvedValue({ id: 'm1', hardware: {} });
  a.settingsGet.mockResolvedValue(settings);
  a.historyList.mockResolvedValue([]);
  a.hfSearch.mockResolvedValue([]);
  a.micList.mockResolvedValue(['Built-in Mic']);
  a.dictionaryList.mockResolvedValue([]);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('Onboarding', () => {
  it('onboarding_advances_when_permissions_granted', async () => {
    a.permissionsStatus.mockResolvedValue({ microphone: 'denied', accessibility: 'denied' });
    render(<Onboarding />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /get started/i }));
    await flush();
    expect((screen.getByRole('button', { name: /^continue$/i }) as HTMLButtonElement).disabled).toBe(true);
    a.permissionsStatus.mockResolvedValue({ microphone: 'granted', accessibility: 'granted' });
    await tick(1000);
    const next = screen.getByRole('button', { name: /^continue$/i }) as HTMLButtonElement;
    expect(next.disabled).toBe(false);
    fireEvent.click(next);
    await flush();
    expect(screen.getByText('Parakeet Small')).toBeTruthy();
  });

  it('onboarding_download_shows_progress', async () => {
    render(<Onboarding />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /get started/i }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /^continue$/i }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /download/i }));
    await flush();
    expect(a.modelInstall).toHaveBeenCalledWith('m1');
    emit('progress', { modelId: 'm1', bytesDone: 50, total: 100, bytesPerSec: 10, etaSecs: 5, status: 'Downloading', error: null });
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('50');
  });
});

describe('Onboarding paused', () => {
  it('onboarding_paused_download_offers_resume', async () => {
    render(<Onboarding />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /get started/i }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /^continue$/i }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /download/i }));
    await flush();
    emit('progress', { modelId: 'm1', bytesDone: 50, total: 100, bytesPerSec: 0, etaSecs: null, status: 'Paused', error: null });
    fireEvent.click(screen.getByRole('button', { name: /^resume$/i }));
    expect(a.modelResume).toHaveBeenCalledWith('m1');
    expect(screen.getByRole('button', { name: /^cancel$/i })).toBeTruthy();
    expect(screen.getByRole('button', { name: /choose another/i })).toBeTruthy();
  });
});

describe('Models', () => {
  it('models_install_button_calls_api', async () => {
    render(<Models />);
    await flush();
    fireEvent.click(screen.getByRole('tab', { name: /catalog/i }));
    await flush();
    const row = screen.getByText('Whisper Tiny').closest('[data-testid="model-row"]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: /download/i }));
    expect(a.modelInstall).toHaveBeenCalledWith('m2');
  });

  it('models_search_shows_incompatible_reason', async () => {
    a.hfSearch.mockResolvedValue([{ repo: 'foo/bar', downloads: 5, likes: 1, lastModified: '2026-01-01' }]);
    a.hfInspect.mockResolvedValue({ kind: 'incompatible', reason: 'No supported engine files found' });
    render(<Models />);
    await flush();
    fireEvent.click(screen.getByRole('tab', { name: /hugging face/i }));
    fireEvent.change(screen.getByPlaceholderText(/search/i), { target: { value: 'bar' } });
    await tick(299);
    expect(a.hfSearch).not.toHaveBeenCalled();
    await tick(2);
    expect(a.hfSearch).toHaveBeenCalledWith('bar');
    fireEvent.click(screen.getByRole('button', { name: /inspect/i }));
    await flush();
    expect(a.hfInspect).toHaveBeenCalledWith('foo/bar');
    expect(screen.getByText('No supported engine files found')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /install/i })).toBeNull();
  });
});

describe('History', () => {
  it('history_search_debounces_and_queries', async () => {
    render(<History />);
    await flush();
    expect(a.historyList).toHaveBeenCalledWith(0, undefined);
    const box = screen.getByPlaceholderText(/search/i);
    for (const v of ['h', 'he', 'hello']) fireEvent.change(box, { target: { value: v } });
    await tick(200);
    expect(a.historyList).toHaveBeenCalledTimes(1);
    await tick(150);
    expect(a.historyList).toHaveBeenCalledTimes(2);
    expect(a.historyList).toHaveBeenLastCalledWith(0, 'hello');
  });
});

describe('History paging', () => {
  it('history_delete_then_load_more_does_not_skip', async () => {
    let server = Array.from({ length: 120 }, (_, i) => ({ id: i, createdAt: 0, text: `row-${i}`, rawText: '', appName: null, durationMs: 0, wordCount: 1, wpm: 0, modelId: 'm' }));
    a.historyList.mockImplementation(async (page: number) => server.slice(page * 50, page * 50 + 50));
    a.historyDelete.mockImplementation(async (id: number) => { server = server.filter((r) => r.id !== id); });
    render(<History />);
    await flush();
    fireEvent.click(screen.getAllByRole('button', { name: /^delete$/i })[0]);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /load more/i }));
    await flush();
    expect(screen.getAllByText('row-50')).toHaveLength(1);
    expect(screen.queryByText('row-0')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /load more/i }));
    await flush();
    expect(screen.getAllByText('row-51')).toHaveLength(1);
    expect(screen.getAllByText('row-100')).toHaveLength(1);
  });
});

describe('Settings', () => {
  it('settings_hotkey_change_persists', async () => {
    render(<Settings />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Right ⌥' }));
    await flush();
    expect(a.settingsSet).toHaveBeenCalledWith({ ...settings, hotkey: { keys: ['AltRight'] } });
  });

  it('hotkey_recording_ignores_escape', async () => {
    render(<Settings />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /record/i }));
    fireEvent.keyDown(window, { code: 'Escape' });
    fireEvent.keyUp(window, { code: 'Escape' });
    await flush();
    expect(a.settingsSet).not.toHaveBeenCalled();
    expect(screen.queryByText(/press the keys/i)).toBeNull();
  });

  it('hotkey_records_a_chord', async () => {
    render(<Settings />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /record/i }));
    fireEvent.keyDown(window, { code: 'ControlLeft' });
    fireEvent.keyDown(window, { code: 'MetaLeft' });
    fireEvent.keyUp(window, { code: 'MetaLeft' });
    await flush();
    expect(a.settingsSet).toHaveBeenCalledWith({ ...settings, hotkey: { keys: ['ControlLeft', 'MetaLeft'] } });
  });

  it('settings_save_error_reverts', async () => {
    a.settingsSet.mockRejectedValueOnce('invalid binding');
    render(<Settings />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Right ⌥' }));
    await flush();
    expect(screen.getByRole('alert').textContent).toContain('invalid binding');
    expect(screen.getByText('Fn', { selector: '.hotkey-current' })).toBeTruthy();
  });
});
