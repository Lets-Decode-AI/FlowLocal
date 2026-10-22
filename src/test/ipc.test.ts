import { describe, it, expect, vi, beforeEach } from 'vitest';

const invoke = vi.fn();
const listen = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: (...a: unknown[]) => listen(...a) }));

import { api, on, EVENTS } from '../ipc';

describe('ipc', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
    listen.mockReset();
  });

  it('ipc_wrappers_invoke_correct_commands', async () => {
    const entry = { id: 'x' } as never;
    const settings = { onboarded: true } as never;
    const cases: [() => Promise<unknown>, string, Record<string, unknown> | undefined][] = [
      [() => api.getState(), 'get_state', undefined],
      [() => api.setPaused(true), 'set_paused', { paused: true }],
      [() => api.permissionsStatus(), 'permissions_status', undefined],
      [() => api.openPermissionSettings('microphone'), 'open_permission_settings', { kind: 'microphone' }],
      [() => api.requestMicrophone(), 'request_microphone', undefined],
      [() => api.catalog(), 'catalog', undefined],
      [() => api.installed(), 'installed', undefined],
      [() => api.recommend(), 'recommend', undefined],
      [() => api.hfSearch('q'), 'hf_search', { q: 'q' }],
      [() => api.hfInspect('a/b'), 'hf_inspect', { repo: 'a/b' }],
      [() => api.modelInstall('parakeet-v2'), 'model_install', { id: 'parakeet-v2', entry: undefined }],
      [() => api.modelInstallEntry(entry), 'model_install', { id: undefined, entry }],
      [() => api.modelPause('m'), 'model_pause', { id: 'm' }],
      [() => api.modelResume('m'), 'model_resume', { id: 'm' }],
      [() => api.modelCancel('m'), 'model_cancel', { id: 'm' }],
      [() => api.modelDelete('m'), 'model_delete', { id: 'm' }],
      [() => api.modelSetActive('m'), 'model_set_active', { id: 'm' }],
      [() => api.historyList(2, 'hi'), 'history_list', { page: 2, query: 'hi' }],
      [() => api.historyDelete(7), 'history_delete', { id: 7 }],
      [() => api.settingsGet(), 'settings_get', undefined],
      [() => api.settingsSet(settings), 'settings_set', { settings }],
      [() => api.hfTokenSet('t'), 'hf_token_set', { token: 't' }],
      [() => api.dictionaryList(), 'dictionary_list', undefined],
      [() => api.dictionaryAdd('a', 'b'), 'dictionary_add', { term: 'a', replacement: 'b' }],
      [() => api.dictionaryDelete(3), 'dictionary_delete', { id: 3 }],
      [() => api.micList(), 'mic_list', undefined],
    ];
    for (const [call, cmd, args] of cases) {
      invoke.mockClear();
      await call();
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke.mock.calls[0][0]).toBe(cmd);
      expect(invoke.mock.calls[0][1]).toEqual(args);
    }
  });

  it('on_wraps_listen_and_unwraps_payload', async () => {
    const unlisten = vi.fn();
    listen.mockImplementation(async (_n: string, h: (e: { payload: unknown }) => void) => {
      h({ payload: { kind: 'idle' } });
      return unlisten;
    });
    const cb = vi.fn();
    const un = await on('state', cb);
    expect(listen.mock.calls[0][0]).toBe(EVENTS.state);
    expect(EVENTS.state).toBe('dictation://state');
    expect(cb).toHaveBeenCalledWith({ kind: 'idle' });
    expect(un).toBe(unlisten);
  });
});
