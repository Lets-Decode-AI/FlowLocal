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

describe('Models', () => {
  it('models_install_button_calls_api', async () => {
    render(<Models />);
    await flush();
    fireEvent.click(screen.getByRole('tab', { name: /library/i }));
    await flush();
    const row = screen.getByText('Whisper Tiny').closest('[data-testid="model-row"]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: /download/i }));
    expect(a.modelInstall).toHaveBeenCalledWith('m2');
  });

  it('models_list_refreshes_when_the_tray_changes_the_active_model', async () => {
    render(<Models />);
    await flush();
    const before = a.installed.mock.calls.length;
    emit('modelsChanged', null);
    await flush();
    expect(a.installed.mock.calls.length).toBeGreaterThan(before);
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

describe('Models rows', () => {
  const installedModel = (id: string, name: string, active: boolean) => ({ ...entry(id, name), diskBytes: 661_000_000, active });

  it('models_installed_row_shows_in_use_and_menu_uses_or_deletes', async () => {
    a.installed.mockResolvedValue([installedModel('m1', 'Parakeet Small', true), installedModel('m2', 'Whisper Tiny', false)]);
    const confirm = vi.spyOn(window, 'confirm').mockReturnValueOnce(false).mockReturnValue(true);
    render(<Models />);
    await flush();
    const rows = screen.getAllByTestId('model-row');
    expect(within(rows[0]).getByText('In use')).toBeTruthy();
    expect(within(rows[1]).queryByText('In use')).toBeNull();
    expect(within(rows[0]).getByText('English · 661 MB')).toBeTruthy();
    fireEvent.click(within(rows[1]).getByRole('button', { name: /options for whisper tiny/i }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Use' }));
    expect(a.modelSetActive).toHaveBeenCalledWith('m2');
    fireEvent.click(within(rows[1]).getByRole('button', { name: /options for whisper tiny/i }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    expect(a.modelDelete).not.toHaveBeenCalled();
    fireEvent.click(within(rows[1]).getByRole('button', { name: /options for whisper tiny/i }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(a.modelDelete).toHaveBeenCalledWith('m2');
    confirm.mockRestore();
  });

  it('models_download_progress_can_pause_resume_and_cancel', async () => {
    render(<Models />);
    await flush();
    fireEvent.click(screen.getByRole('tab', { name: /library/i }));
    await flush();
    emit('progress', { modelId: 'm2', status: 'Downloading', bytesDone: 50_000_000, total: 100_000_000, bytesPerSec: 1_000_000, etaSecs: 50 });
    const row = screen.getByText('Whisper Tiny').closest('[data-testid="model-row"]') as HTMLElement;
    expect(within(row).getByRole('progressbar').getAttribute('aria-valuenow')).toBe('50');
    fireEvent.click(within(row).getByRole('button', { name: /^pause$/i }));
    expect(a.modelPause).toHaveBeenCalledWith('m2');
    emit('progress', { modelId: 'm2', status: 'Paused', bytesDone: 50_000_000, total: 100_000_000, bytesPerSec: 0, etaSecs: null });
    fireEvent.click(within(row).getByRole('button', { name: /^resume$/i }));
    expect(a.modelResume).toHaveBeenCalledWith('m2');
    fireEvent.click(within(row).getByRole('button', { name: /^cancel$/i }));
    expect(a.modelCancel).toHaveBeenCalledWith('m2');
  });
});

describe('History', () => {
  it('history_groups_by_day', async () => {
    const at = (daysAgo: number, h: number) => { const d = new Date(); d.setDate(d.getDate() - daysAgo); d.setHours(h, 5, 0, 0); return d.getTime(); };
    const row = (id: number, daysAgo: number, h: number, text: string) => ({ id, createdAt: at(daysAgo, h), text, rawText: '', appName: id === 1 ? 'Notes' : null, durationMs: 0, wordCount: id === 1 ? 1 : 3, wpm: 0, modelId: 'm' });
    a.historyList.mockResolvedValue([row(1, 0, 10, 'first'), row(2, 0, 9, 'second'), row(3, 1, 8, 'third'), row(4, 5, 7, 'fourth')]);
    render(<History />);
    await flush();
    const titles = screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent);
    expect(titles).toHaveLength(3);
    expect(titles[0]).toBe('Today');
    expect(titles[1]).toBe('Yesterday');
    expect(titles[2]).toBe(new Date(at(5, 7)).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' }));
    expect(screen.getByText(/^Notes · .* · 1 word$/)).toBeTruthy();
    expect(screen.getAllByText(/ · 3 words$/)).toHaveLength(3);
  });

  it('history_empty_state_points_at_the_hotkey', async () => {
    render(<History />);
    await flush();
    expect(screen.getByText('No dictations yet')).toBeTruthy();
    expect(screen.getByText(/in any app to start\./)).toBeTruthy();
    expect(document.querySelectorAll('kbd')).toHaveLength(1);
  });

  it('history_row_text_expands_on_click', async () => {
    a.historyList.mockResolvedValue([{ id: 1, createdAt: Date.now(), text: 'a long note', rawText: '', appName: null, durationMs: 0, wordCount: 3, wpm: 0, modelId: 'm' }]);
    render(<History />);
    await flush();
    const text = screen.getByRole('button', { name: /a long note/ });
    expect(text.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(text);
    expect(text.getAttribute('aria-expanded')).toBe('true');
  });

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
  it('settings_groups_follow_system_settings_layout', async () => {
    render(<Settings />);
    await flush();
    expect(screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual(['Dictation', 'Model', 'General', 'Personal dictionary', 'Hugging Face']);
    expect(screen.getByRole('switch', { name: 'Show Flow Bar' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: 'Load at startup' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: 'Open at login' })).toBeTruthy();
    fireEvent.click(screen.getByRole('switch', { name: 'Open at login' }));
    await flush();
    expect(a.settingsSet).toHaveBeenCalledWith({ ...settings, launchAtLogin: true });
  });

  it('settings_hotkey_change_persists', async () => {
    render(<Settings />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /^change$/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Right ⌥' }));
    await flush();
    expect(a.settingsSet).toHaveBeenCalledWith({ ...settings, hotkey: { keys: ['AltRight'] } });
  });

  it('hotkey_recording_ignores_escape', async () => {
    render(<Settings />);
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /^change$/i }));
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
    fireEvent.click(screen.getByRole('button', { name: /^change$/i }));
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
    fireEvent.click(screen.getByRole('button', { name: /^change$/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Right ⌥' }));
    await flush();
    expect(screen.getByRole('alert').textContent).toContain('invalid binding');
    expect(screen.getByRole('group', { name: 'Current shortcut' }).textContent).toMatch(/^fn$/i);
  });
});
