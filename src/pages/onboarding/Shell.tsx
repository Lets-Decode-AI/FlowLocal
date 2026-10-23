import type { ReactNode } from 'react';
import Button from '../../components/ui/Button';

export const STEP_NAMES = ['Welcome', 'Shortcut', 'Permissions', 'Speech model', 'Try it'];

export interface StepProps {
  onNext: () => void;
  onBack: () => void;
}

export function Dots({ step }: { step: number }) {
  return (
    <div className="onb-dots" role="group" aria-label={`Step ${step + 1} of ${STEP_NAMES.length}: ${STEP_NAMES[step]}`}>
      {STEP_NAMES.map((s, i) => <span key={s} className={`onb-dot${i <= step ? ' onb-dot-on' : ''}`} />)}
    </div>
  );
}

/** Body (centered 440px column) plus the Back / Continue footer shared by every step. */
export function StepFrame({ onBack, onNext, nextDisabled, nextLabel = 'Continue', hideNext, note, children }: {
  onBack?: () => void;
  onNext?: () => void;
  nextDisabled?: boolean;
  nextLabel?: string;
  hideNext?: boolean;
  note?: ReactNode;
  children: ReactNode;
}) {
  return (
    <>
      <div className="onb-body">{children}</div>
      <footer className="onb-footer">
        {note && <p className="onb-note">{note}</p>}
        <div className="onb-actions">
          {onBack ? <Button variant="plain" size="large" onClick={onBack}>Back</Button> : <span />}
          {!hideNext && <Button variant="primary" size="large" disabled={nextDisabled} onClick={onNext}>{nextLabel}</Button>}
        </div>
      </footer>
    </>
  );
}
