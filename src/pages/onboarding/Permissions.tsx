import { useEffect, useState } from 'react';
import { api, type Perm, type PermissionsStatus } from '../../ipc';
import { useToast } from '../../components/Toasts';
import Button from '../../components/ui/Button';
import Group from '../../components/ui/Group';
import Icon from '../../components/ui/Icon';
import { currentPlatform } from '../../components/ui/Keycap';
import Row from '../../components/ui/Row';
import { errMsg } from '../../lib/format';
import { StepFrame, type StepProps } from './Shell';

const ok = (p: Perm) => p === 'granted' || p === 'notRequired';

function Accessory({ status, name, onAllow }: { status: Perm; name: string; onAllow: () => void }) {
  if (status === 'notRequired') return <span>Not required</span>;
  if (status === 'granted') {
    return <span className="onb-allowed"><Icon name="check" size={14} />Allowed</span>;
  }
  return <Button aria-label={`Allow ${name}`} onClick={onAllow}>Allow…</Button>;
}

export default function Permissions({ onNext, onBack }: StepProps) {
  const toast = useToast();
  const [p, setP] = useState<PermissionsStatus | null>(null);
  useEffect(() => {
    let live = true;
    const poll = () => api.permissionsStatus().then((s) => live && setP(s), () => {});
    poll();
    const t = setInterval(poll, 1000);
    return () => { live = false; clearInterval(t); };
  }, []);
  const attempt = (f: () => Promise<void>) => f().catch((e) => toast(errMsg(e), 'error'));
  const mic = p?.microphone ?? 'unknown';
  const acc = p?.accessibility ?? 'unknown';
  const where = currentPlatform() === 'mac' ? 'System Settings' : 'your system settings';
  return (
    <StepFrame onBack={onBack} onNext={onNext} nextDisabled={!(ok(mic) && ok(acc))} note={`You can change these later in ${where}.`}>
      <h1>Permissions</h1>
      <p className="muted">FlowLocal needs two permissions to hear you and type for you.</p>
      <Group>
        <Row
          icon="mic" label="Microphone" detail="Record your voice while you hold the shortcut."
          accessory={<Accessory status={mic} name="Microphone" onAllow={() => attempt(() => (mic === 'denied' ? api.openPermissionSettings('microphone') : api.requestMicrophone()))} />}
        />
        <Row
          icon="keyboard" label="Accessibility" detail="Insert the text into the app you are using."
          accessory={<Accessory status={acc} name="Accessibility" onAllow={() => attempt(() => api.openPermissionSettings('accessibility'))} />}
        />
      </Group>
    </StepFrame>
  );
}
