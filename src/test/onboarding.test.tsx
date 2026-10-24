import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const handlers: Record<string, Array<(p: any) => void>> = {};
const entry = (id: string, name: string) => ({
  id, name, hf_repo: `o/${id}`, revision: 'main', files: [{ path: 'a.onnx', size: 100_000_000, sha256: 'x' }],
  engine: 'transducer', languages: ['en'], min_ram_gb: 4, notes: '',
});
const settings = {
  hotkey: { keys: ['Fn'] }, mic: null, showFlowbar: true, idleUnloadMin: 10, preload: true, launchAtLogin: false, onboarded: false,
};
const transcript = { id: 7, createdAt: 1, text: 'Hello from uttr', rawText: 'hello from uttr', appName: null, durationMs: 2000, wordCount: 3, wpm: 90.4, modelId: 'm1' };

vi.mock('../ipc', () => ({
  api: {
    getState: vi.fn(async () => ({ kind: 'idle' })),
    permissionsStatus: vi.fn(), openPermissionSettings: vi.fn(async () => {}), requestMicrophone: vi.fn(async () => {}), restartApp: vi.fn(async () => {}),
    catalog: vi.fn(), installed: vi.fn(), recommend: vi.fn(),
    modelInstall: vi.fn(async () => {}), modelPause: vi.fn(async () => {}), modelResume: vi.fn(async () => {}),
    modelCancel: vi.fn(async () => {}), modelSetActive: vi.fn(async () => {}),
    historyList: vi.fn(), settingsGet: vi.fn(), settingsSet: vi.fn(async () => {}),
  },
  on: vi.fn(async (name: string, cb: (p: any) => void) => {
    (handlers[name] ||= []).push(cb);
    return () => {};
  }),
}));

import { api } from '../ipc';
import Onboarding from '../pages/Onboarding';

const a = api as any;
const emit = (name: string, p: unknown) => act(() => handlers[name]?.forEach((h) => h(p)));
const flush = () => act(async () => {});
const tick = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
const setPlatform = (v: string) => Object.defineProperty(window.navigator, 'platform', { value: v, configurable: true });
const next = async () => { fireEvent.click(screen.getByRole('button', { name: /^continue$/i })); await flush(); };
const toStep = async (n: number) => { for (let i = 1; i < n; i++) await next(); };
const stepLabel = () => screen.getByRole('group', { name: /^Step \d of 5/ }).getAttribute('aria-label');

beforeEach(() => {
  window.localStorage.clear();
  vi.useFakeTimers();
  for (const k of Object.keys(handlers)) delete handlers[k];
  vi.clearAllMocks();
  setPlatform('MacIntel');
  window.location.hash = '#/onboarding';
  a.permissionsStatus.mockResolvedValue({ microphone: 'granted', accessibility: 'granted' });
  a.catalog.mockResolvedValue([entry('m1', 'Parakeet Small'), entry('m2', 'Whisper Tiny')]);
  a.installed.mockResolvedValue([]);
  a.recommend.mockResolvedValue({ id: 'm1', hardware: { appleSilicon: true, totalRamGb: 16 } });
  a.settingsGet.mockResolvedValue(settings);
  a.historyList.mockResolvedValue([transcript]);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  delete (window.navigator as any).platform;
});

describe('Onboarding flow', () => {
  it('onboarding_has_five_steps_and_back_navigation', async () => {
    a.installed.mockResolvedValue([{ ...entry('m1', 'Parakeet Small'), diskBytes: 1, active: false }]);
    render(<Onboarding />);
    await flush();
    expect(stepLabel()).toMatch(/Step 1 of 5/);
    expect(screen.queryByRole('button', { name: /^back$/i })).toBeNull();
    await next();
    expect(stepLabel()).toMatch(/Step 2 of 5/);
    await next();
    expect(stepLabel()).toMatch(/Step 3 of 5/);
    await next();
    expect(stepLabel()).toMatch(/Step 4 of 5/);
    await next();
    expect(stepLabel()).toMatch(/Step 5 of 5/);
    fireEvent.click(screen.getByRole('button', { name: /^back$/i }));
    await flush();
    expect(stepLabel()).toMatch(/Step 4 of 5/);
    fireEvent.click(screen.getByRole('button', { name: /^back$/i }));
    await flush();
    expect(stepLabel()).toMatch(/Step 3 of 5/);
  });

  it('welcome_says_this_mac_on_macos_and_this_computer_elsewhere', async () => {
    render(<Onboarding />);
    await flush();
    expect(screen.getByRole('heading', { name: 'uttr' })).toBeTruthy();
    expect(screen.getByText('Dictate into any app. Speech is turned into text on this Mac and never leaves it.')).toBeTruthy();
    cleanup();
    setPlatform('Win32');
    render(<Onboarding />);
    await flush();
    expect(screen.getByText('Dictate into any app. Speech is turned into text on this computer and never leaves it.')).toBeTruthy();
  });
});

describe('Shortcut step', () => {
  it('shortcut_step_shows_current_binding_keycaps', async () => {
    a.settingsGet.mockResolvedValue({ ...settings, hotkey: { keys: ['ControlLeft', 'MetaLeft'] } });
    render(<Onboarding />);
    await flush();
    await next();
    const caps = Array.from(document.querySelectorAll('kbd')).map((k) => k.textContent);
    expect(caps).toEqual(['⌃', '⌘']);
    expect(screen.getByText('Hold to talk. Release to insert the text.')).toBeTruthy();
    expect(screen.getByText('Double-press to keep listening hands-free. Press Esc to cancel.')).toBeTruthy();
  });

  it('shortcut_step_explains_the_globe_key_and_opens_keyboard_settings_on_macos', async () => {
    render(<Onboarding />);
    await flush();
    await next();
    expect(screen.getByText(/Press .* key to Do Nothing/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /open keyboard settings/i }));
    expect(a.openPermissionSettings).toHaveBeenCalledWith('keyboard');
  });

  it('shortcut_step_omits_the_globe_note_off_macos', async () => {
    setPlatform('Win32');
    render(<Onboarding />);
    await flush();
    await next();
    expect(screen.queryByText(/Do Nothing/)).toBeNull();
    expect(screen.queryByRole('button', { name: /open keyboard settings/i })).toBeNull();
  });

  it('change_shortcut_opens_the_picker_and_saves', async () => {
    render(<Onboarding />);
    await flush();
    await next();
    fireEvent.click(screen.getByRole('button', { name: /change shortcut/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Right ⌥' }));
    await flush();
    expect(a.settingsSet).toHaveBeenCalledWith({ ...settings, hotkey: { keys: ['AltRight'] } });
  });
});

describe('Permissions step', () => {
  it('onboarding_advances_when_permissions_granted', async () => {
    a.permissionsStatus.mockResolvedValue({ microphone: 'denied', accessibility: 'denied' });
    render(<Onboarding />);
    await flush();
    await toStep(3);
    expect((screen.getByRole('button', { name: /^continue$/i }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('You can change these later in System Settings.')).toBeTruthy();
    a.permissionsStatus.mockResolvedValue({ microphone: 'granted', accessibility: 'granted' });
    await tick(1000);
    expect(screen.getAllByText('Allowed').length).toBe(2);
    const cont = screen.getByRole('button', { name: /^continue$/i }) as HTMLButtonElement;
    expect(cont.disabled).toBe(false);
    fireEvent.click(cont);
    await flush();
    expect(screen.getByText('Parakeet Small')).toBeTruthy();
  });

  it('continue_without_permissions_is_available_on_mac', async () => {
    a.permissionsStatus.mockResolvedValue({ microphone: 'granted', accessibility: 'denied' });
    render(<Onboarding />);
    await flush();
    await toStep(3);
    expect((screen.getByRole('button', { name: /^continue$/i }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: /continue without/i }));
    await flush();
    expect(stepLabel()).toMatch(/Step 4 of 5/);
  });

  it('no_allow_button_flashes_before_the_first_poll', async () => {
    a.permissionsStatus.mockReturnValue(new Promise(() => {}));
    render(<Onboarding />);
    await flush();
    await toStep(3);
    expect(screen.queryByRole('button', { name: /allow/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /continue without/i })).toBeNull();
  });

  it('allow_buttons_request_or_open_settings', async () => {
    a.permissionsStatus.mockResolvedValue({ microphone: 'unknown', accessibility: 'denied' });
    render(<Onboarding />);
    await flush();
    await toStep(3);
    fireEvent.click(screen.getByRole('button', { name: /allow microphone/i }));
    expect(a.requestMicrophone).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: /allow accessibility/i }));
    expect(a.openPermissionSettings).toHaveBeenCalledWith('accessibility');
  });

  it('permission_polling_stops_when_leaving_the_step', async () => {
    render(<Onboarding />);
    await flush();
    await toStep(3);
    await tick(2000);
    fireEvent.click(screen.getByRole('button', { name: /^back$/i }));
    await flush();
    const calls = a.permissionsStatus.mock.calls.length;
    await tick(5000);
    expect(a.permissionsStatus.mock.calls.length).toBe(calls);
  });
  it('restart_offered_after_allowing_accessibility', async () => {
    a.permissionsStatus.mockResolvedValue({ microphone: 'granted', accessibility: 'denied' });
    render(<Onboarding />);
    await flush();
    await toStep(3);
    expect(screen.queryByRole('button', { name: /restart uttr/i })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /allow accessibility/i }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /restart uttr/i }));
    expect(a.restartApp).toHaveBeenCalled();
  });

  it('resumes_at_the_saved_step_after_restart', async () => {
    window.localStorage.setItem('uttr.onboardingStep', '2');
    a.permissionsStatus.mockResolvedValue({ microphone: 'granted', accessibility: 'denied' });
    render(<Onboarding />);
    await flush();
    expect(screen.getByRole('heading', { name: /permissions/i })).toBeTruthy();
    window.localStorage.removeItem('uttr.onboardingStep');
  });
});

describe('Model step', () => {
  it('onboarding_download_shows_progress', async () => {
    render(<Onboarding />);
    await flush();
    await toStep(4);
    expect(screen.getByText(/Recommended for English on Apple silicon/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /^download$/i }));
    await flush();
    expect(a.modelInstall).toHaveBeenCalledWith('m1');
    emit('progress', { modelId: 'm1', bytesDone: 50_000_000, total: 100_000_000, bytesPerSec: 10, etaSecs: 120, status: 'Downloading', error: null });
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('50');
    expect(screen.getByText('50 MB of 100 MB · about 2 min left')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /^pause$/i }));
    expect(a.modelPause).toHaveBeenCalledWith('m1');
  });

  it('onboarding_paused_download_offers_resume', async () => {
    render(<Onboarding />);
    await flush();
    await toStep(4);
    fireEvent.click(screen.getByRole('button', { name: /^download$/i }));
    await flush();
    emit('progress', { modelId: 'm1', bytesDone: 50, total: 100, bytesPerSec: 0, etaSecs: null, status: 'Paused', error: null });
    fireEvent.click(screen.getByRole('button', { name: /^resume$/i }));
    expect(a.modelResume).toHaveBeenCalledWith('m1');
    expect(screen.getByRole('button', { name: /^cancel$/i })).toBeTruthy();
    expect(screen.getByRole('button', { name: /choose a different model/i })).toBeTruthy();
  });

  it('download_progress_survives_back_and_forward', async () => {
    render(<Onboarding />);
    await flush();
    await toStep(4);
    fireEvent.click(screen.getByRole('button', { name: /^download$/i }));
    await flush();
    emit('progress', { modelId: 'm1', bytesDone: 50, total: 100, bytesPerSec: 1, etaSecs: 5, status: 'Downloading', error: null });
    fireEvent.click(screen.getByRole('button', { name: /^back$/i }));
    await flush();
    emit('progress', { modelId: 'm1', bytesDone: 60, total: 100, bytesPerSec: 1, etaSecs: 4, status: 'Downloading', error: null });
    await next();
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('60');
    expect(screen.queryByRole('button', { name: /^download$/i })).toBeNull();
  });

  it('failed_download_offers_retry', async () => {
    render(<Onboarding />);
    await flush();
    await toStep(4);
    fireEvent.click(screen.getByRole('button', { name: /^download$/i }));
    await flush();
    emit('progress', { modelId: 'm1', bytesDone: 0, total: 100, bytesPerSec: 0, etaSecs: null, status: { Failed: 'network' }, error: 'network' });
    expect(screen.getByText(/network/)).toBeTruthy();
    expect(screen.getByRole('button', { name: /^retry$/i })).toBeTruthy();
  });

  it('choosing_a_different_model_switches_the_card', async () => {
    render(<Onboarding />);
    await flush();
    await toStep(4);
    fireEvent.click(screen.getByRole('button', { name: /choose a different model/i }));
    fireEvent.click(screen.getByRole('button', { name: /whisper tiny/i }));
    expect(screen.getByText('Whisper Tiny')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /^download$/i }));
    await flush();
    expect(a.modelInstall).toHaveBeenCalledWith('m2');
  });

  it('installed_model_is_ready_and_continue_activates_it', async () => {
    a.installed.mockResolvedValue([{ ...entry('m1', 'Parakeet Small'), diskBytes: 1, active: false }]);
    render(<Onboarding />);
    await flush();
    await toStep(4);
    expect(screen.getByText('Ready')).toBeTruthy();
    await next();
    expect(a.modelSetActive).toHaveBeenCalledWith('m1');
    expect(stepLabel()).toMatch(/Step 5 of 5/);
  });
});

describe('Try it step', () => {
  const toTryIt = async () => {
    a.installed.mockResolvedValue([{ ...entry('m1', 'Parakeet Small'), diskBytes: 1, active: false }]);
    render(<Onboarding />);
    await flush();
    await toStep(5);
  };

  it('try_it_mirrors_dictation_state', async () => {
    await toTryIt();
    expect(screen.getByText(/and say something/)).toBeTruthy();
    emit('state', { kind: 'recording', handsFree: false });
    expect(screen.getByRole('status').textContent).toMatch(/Listening/);
  });

  it('try_it_reads_current_state_and_does_not_light_up_for_errors', async () => {
    a.getState.mockResolvedValueOnce({ kind: 'recording', handsFree: false });
    await toTryIt();
    expect(screen.getByRole('status').textContent).toMatch(/Listening/);
    emit('state', { kind: 'error', message: 'cpal: device lost 0x5' });
    const st = screen.getByRole('status');
    expect(st.textContent).not.toMatch(/cpal/);
    expect(st.className).not.toMatch(/listening-on/);
  });

  it('try_it_shows_transcript_and_finish', async () => {
    await toTryIt();
    emit('transcriptAdded', { id: 7 });
    await flush();
    expect(screen.getByText("You're set up.")).toBeTruthy();
    expect(screen.getByText('Hello from uttr')).toBeTruthy();
    expect(screen.getByText('3 words · 90 WPM')).toBeTruthy();
    expect(a.settingsSet).toHaveBeenCalledWith({ ...settings, onboarded: true });
    fireEvent.click(screen.getByRole('button', { name: /start using uttr/i }));
    expect(window.location.hash).toBe('#/history');
  });
});
