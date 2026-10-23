import { useState } from 'react';
import Model from './onboarding/Model';
import Permissions from './onboarding/Permissions';
import { Dots, STEP_NAMES } from './onboarding/Shell';
import Shortcut from './onboarding/Shortcut';
import TryIt from './onboarding/TryIt';
import Welcome from './onboarding/Welcome';

export default function Onboarding() {
  const [step, setStep] = useState(0);
  const next = () => setStep((s) => Math.min(s + 1, STEP_NAMES.length - 1));
  const back = () => setStep((s) => Math.max(s - 1, 0));
  const props = { onNext: next, onBack: back };
  return (
    <div className="onb">
      <Dots step={step} />
      {step === 0 && <Welcome onNext={next} />}
      {step === 1 && <Shortcut {...props} />}
      {step === 2 && <Permissions {...props} />}
      {step === 3 && <Model {...props} />}
      {step === 4 && <TryIt onBack={back} />}
    </div>
  );
}
