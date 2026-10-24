import { useEffect, useState } from 'react';
import Model from './onboarding/Model';
import Permissions from './onboarding/Permissions';
import { Dots, STEP_NAMES } from './onboarding/Shell';
import Shortcut from './onboarding/Shortcut';
import TryIt from './onboarding/TryIt';
import Welcome from './onboarding/Welcome';
import { useDownloads } from './onboarding/useDownloads';

// Remembered so a restart (needed on macOS to pick up Accessibility) resumes the same step.
const STEP_KEY = 'flowlocal.onboardingStep';

function savedStep(): number {
  try {
    const n = Number(window.localStorage.getItem(STEP_KEY));
    return Number.isInteger(n) && n >= 0 && n < STEP_NAMES.length ? n : 0;
  } catch {
    return 0;
  }
}

export default function Onboarding() {
  const [step, setStep] = useState(savedStep);
  useEffect(() => {
    try {
      window.localStorage.setItem(STEP_KEY, String(step));
    } catch {
      // Storage can be unavailable; resuming is a convenience only.
    }
  }, [step]);
  const next = () => setStep((s) => Math.min(s + 1, STEP_NAMES.length - 1));
  const back = () => setStep((s) => Math.max(s - 1, 0));
  const downloads = useDownloads();
  const props = { onNext: next, onBack: back };
  return (
    <div className="onb">
      <Dots step={step} />
      {step === 0 && <Welcome onNext={next} />}
      {step === 1 && <Shortcut {...props} />}
      {step === 2 && <Permissions {...props} />}
      {step === 3 && <Model {...props} downloads={downloads} />}
      {step === 4 && <TryIt onBack={back} />}
    </div>
  );
}
