import { useEffect, useRef, useState } from 'react';
import { api, on, type DictationState } from '../ipc';
import Waveform from '../components/Waveform';

const DONE_MS = 600;

type Kind = 'idle' | 'recording' | 'transcribing' | 'done' | 'error';

export default function FlowBar() {
  const [state, setState] = useState<DictationState>({ kind: 'idle' });
  const [level, setLevel] = useState(0);
  const [show, setShow] = useState(true);
  const [done, setDone] = useState(false);
  const prev = useRef<DictationState['kind']>('idle');
  const doneTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const flashDone = () => {
    clearTimeout(doneTimer.current);
    setDone(true);
    doneTimer.current = setTimeout(() => setDone(false), DONE_MS);
  };

  useEffect(() => {
    let alive = true;
    const unlisten: Array<() => void> = [];
    const add = (p: Promise<() => void>) =>
      p.then((u) => (alive ? unlisten.push(u) : u())).catch(() => {});

    add(
      on('state', (s) => {
        if (prev.current === 'inserting' && s.kind === 'idle') flashDone();
        else if (s.kind !== 'idle') {
          clearTimeout(doneTimer.current);
          setDone(false);
        }
        prev.current = s.kind;
        setState(s);
      }),
    );
    add(on('level', setLevel));
    add(on('settingsChanged', (s) => setShow(s.showFlowbar)));
    add(on('transcriptAdded', () => flashDone()));
    api.getState().then((s) => alive && (prev.current = s.kind, setState(s))).catch(() => {});
    api.settingsGet().then((s) => alive && setShow(s.showFlowbar)).catch(() => {});
    return () => {
      alive = false;
      clearTimeout(doneTimer.current);
      unlisten.forEach((u) => u());
    };
  }, []);

  const kind: Kind =
    state.kind === 'recording' ? 'recording'
    : state.kind === 'error' ? 'error'
    : state.kind === 'transcribing' || state.kind === 'inserting' ? 'transcribing'
    : done ? 'done'
    : 'idle';

  // Idle + hidden by setting: render nothing. The window is click-through regardless.
  if (kind === 'idle' && !show) return <div className="fb-root" />;

  return (
    <div className="fb-root">
      <div
        data-testid={kind === 'idle' ? 'flowbar-idle' : 'flowbar-pill'}
        data-kind={kind}
        className={`fb-pill fb-${kind}`}
      >
        {kind === 'recording' && (
          <>
            {state.kind === 'recording' && state.handsFree && (
              <span data-testid="hands-free-dot" className="fb-dot" />
            )}
            <Waveform level={level} />
          </>
        )}
        {kind === 'transcribing' && <span className="fb-shimmer" />}
        {kind === 'done' && (
          <span data-testid="flowbar-done" className="fb-check">✓</span>
        )}
        {kind === 'error' && state.kind === 'error' && (
          <span className="fb-error-text">{state.message}</span>
        )}
      </div>
    </div>
  );
}
