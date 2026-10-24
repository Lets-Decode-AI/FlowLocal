import Logo from '../../components/ui/Logo';
import { currentPlatform } from '../../components/ui/Keycap';
import { StepFrame, type StepProps } from './Shell';

export default function Welcome({ onNext }: Pick<StepProps, 'onNext'>) {
  const where = currentPlatform() === 'mac' ? 'this Mac' : 'this computer';
  return (
    <StepFrame onNext={onNext}>
      <div className="onb-glyph"><Logo size={72} /></div>
      <h1>uttr</h1>
      <p className="onb-lead">Dictate into any app. Speech is turned into text on {where} and never leaves it.</p>
    </StepFrame>
  );
}
