import Switch from './ui/Switch';

/** Kept for existing pages; renders the shared macOS switch. */
export default function Toggle(props: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return <Switch {...props} />;
}
