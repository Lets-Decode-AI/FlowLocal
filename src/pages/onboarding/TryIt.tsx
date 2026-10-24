import { useEffect, useRef, useState } from 'react';
import { api, type DictationState, type Settings, type Transcript } from '../../ipc';
import MicPicker from '../../components/MicPicker';
import { useToast } from '../../components/Toasts';
import Button from '../../components/ui/Button';
import Icon from '../../components/ui/Icon';
import { keycapsFor } from '../../components/ui/Keycap';
import { errMsg } from '../../lib/format';
import { navigate, useEvent } from '../../lib/hooks';
import { StepFrame, type StepProps } from './Shell';
import { useBinding } from '../../lib/useBinding';

function describe(s: DictationState): string {
  switch (s.kind) {
    case 'recording': return s.handsFree ? 'Listening, hands-free' : 'Listening';
    case 'transcribing': return 'Transcribing';
    case 'inserting': return 'Inserting';
    case 'error': return 'Something went wrong. Try again.';
    default: return 'Ready';
  }
}

export default function TryIt({ onBack }: Pick<StepProps, 'onBack'>) {
  const toast = useToast();
  const { binding } = useBinding();
  const [state, setState] = useState<DictationState>({ kind: 'idle' });
  const [done, setDone] = useState(false);
  const [transcript, setTranscript] = useState<Transcript | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const finished = useRef(false);

  useEffect(() => {
    api.settingsGet().then(setSettings, () => {});
  }, []);
  const chooseMic = async (mic: string | null) => {
    if (!settings) return;
    const next = { ...settings, mic };
    setSettings(next);
    try {
      await api.settingsSet(next);
    } catch (e) {
      setSettings(settings);
      toast(errMsg(e), 'error');
    }
  };

  useEffect(() => {
    let live = true;
    api.getState().then((s) => live && setState((cur) => (cur.kind === 'idle' ? s : cur)), () => {});
    return () => { live = false; };
  }, []);
  useEvent('state', setState);
  useEvent('transcriptAdded', async ({ id }) => {
    if (finished.current) return;
    finished.current = true;
    setDone(true);
    try {
      const rows = await api.historyList(0);
      setTranscript(rows.find((r) => r.id === id) ?? rows[0] ?? null);
    } catch {
      // The success state still shows without the transcript details.
    }
    try {
      const s = await api.settingsGet();
      await api.settingsSet({ ...s, onboarded: true });
      try { window.localStorage.removeItem('uttr.onboardingStep'); } catch { /* storage unavailable */ }
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  });

  if (done) {
    return (
      <StepFrame hideNext>
        <div className="onb-success" aria-hidden="true"><Icon name="check" size={28} /></div>
        <h1>You're set up.</h1>
        {transcript && (
          <>
            <p className="onb-quote selectable">{transcript.text}</p>
            <p className="muted">{transcript.wordCount} {transcript.wordCount === 1 ? 'word' : 'words'} · {Math.round(transcript.wpm)} WPM</p>
          </>
        )}
        <Button variant="primary" size="large" onClick={() => navigate('/history')}>Start using uttr</Button>
      </StepFrame>
    );
  }
  const active = state.kind === 'recording' || state.kind === 'transcribing';
  return (
    <StepFrame onBack={onBack} hideNext>
      <h1>Try it</h1>
      <p className="onb-lead">Hold {keycapsFor(binding)} and say something.</p>
      <textarea className="onb-textarea" autoFocus rows={5} aria-label="Dictation test area" placeholder="Your words will appear here" />
      <div className={`onb-listening${active ? ' onb-listening-on' : ''}`} role="status">
        <span className="onb-pulse" aria-hidden="true" />
        {describe(state)}
      </div>
      {settings && (
        <label className="onb-mic">
          <span className="muted">Microphone</span>
          <MicPicker value={settings.mic} onChange={chooseMic} />
        </label>
      )}
    </StepFrame>
  );
}
