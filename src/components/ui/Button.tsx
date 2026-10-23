import type { ButtonHTMLAttributes } from 'react';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'plain' | 'destructive';
  size?: 'regular' | 'large';
}

export default function Button({ variant = 'secondary', size = 'regular', className, type = 'button', ...rest }: ButtonProps) {
  return <button type={type} className={`ui-btn ui-btn-${variant} ui-btn-${size}${className ? ` ${className}` : ''}`} {...rest} />;
}
