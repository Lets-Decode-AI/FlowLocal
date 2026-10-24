import forestTile from '../../assets/brand/uttr-icon-forest.svg';
import creamTile from '../../assets/brand/uttr-icon-cream.svg';
import { useTheme } from '../../lib/theme';

/** The uttr app tile: forest on the light theme, cream on the dark theme. */
export default function Logo({ size = 24, label }: { size?: number; label?: string }) {
  const { theme } = useTheme();
  return (
    <img
      className="ui-logo" src={theme === 'dark' ? creamTile : forestTile} width={size} height={size}
      alt={label ?? ''} aria-hidden={label ? undefined : true} draggable={false}
    />
  );
}
