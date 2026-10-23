import { forwardRef, type ButtonHTMLAttributes } from 'react';
import Icon, { type IconName } from './Icon';

interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'aria-label'> {
  icon: IconName;
  label: string;
  tone?: 'default' | 'destructive';
}

/** Borderless icon-only button. The label is required so it always has an accessible name. */
const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton({ icon, label, tone = 'default', className, type = 'button', ...rest }, ref) {
  return (
    <button ref={ref} type={type} aria-label={label} title={label} className={`ui-iconbtn${tone === 'destructive' ? ' ui-iconbtn-destructive' : ''}${className ? ` ${className}` : ''}`} {...rest}>
      <Icon name={icon} size={16} />
    </button>
  );
});

export default IconButton;
