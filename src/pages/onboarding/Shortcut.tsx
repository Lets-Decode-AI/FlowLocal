import { useState } from 'react';
import { api } from '../../ipc';
import HotkeyInput from '../../components/HotkeyInput';
import { useToast } from '../../components/Toasts';
import Button from '../../components/ui/Button';
import Group from '../../components/ui/Group';
import { currentPlatform, keycapsFor } from '../../components/ui/Keycap';
import Row from '../../components/ui/Row';
import { errMsg } from '../../lib/format';
import { StepFrame, type StepProps } from './Shell';
import { useBinding } from './useBinding';

export default function Shortcut({ onNext, onBack }: StepProps) {
  const toast = useToast();
  const { binding, save } = useBinding((e) => toast(errMsg(e), 'error'));
  const [editing, setEditing] = useState(false);
  const mac = currentPlatform() === 'mac';
  return (
    <StepFrame onBack={onBack} onNext={onNext}>
      <h1>Your shortcut</h1>
      <div className="onb-hero" aria-label="Current shortcut">{keycapsFor(binding)}</div>
      <p className="onb-lead">Hold to talk. Release to insert the text.</p>
      <p className="muted">Double-press to keep listening hands-free. Press Esc to cancel.</p>
      {editing ? <HotkeyInput value={binding} onChange={save} /> : (
        <Button variant="plain" onClick={() => setEditing(true)}>Change shortcut</Button>
      )}
      {mac && (
        <Group>
          <Row
            icon="keyboard"
            label="Globe key"
            detail="Set Press 🌐 key to Do Nothing so macOS doesn't open the emoji picker."
            accessory={<Button onClick={() => api.openPermissionSettings('keyboard').catch((e) => toast(errMsg(e), 'error'))}>Open Keyboard Settings</Button>}
          />
        </Group>
      )}
    </StepFrame>
  );
}
