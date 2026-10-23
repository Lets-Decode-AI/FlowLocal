import type { ReactNode } from 'react';
import Icon, { type IconName } from './Icon';

interface RowProps {
  label: ReactNode;
  detail?: ReactNode;
  icon?: IconName;
  accessory?: ReactNode;
  onClick?: () => void;
}

/** One row of a Group. Rows are separated by an inset line that starts at the text column. */
export default function Row({ label, detail, icon, accessory, onClick }: RowProps) {
  const body = (
    <>
      {icon && <span className="ui-row-icon"><Icon name={icon} size={16} /></span>}
      <span className="ui-row-text">
        <span className="ui-row-label">{label}</span>
        {detail && <span className="ui-row-detail">{detail}</span>}
      </span>
      {accessory && <span className="ui-row-accessory">{accessory}</span>}
    </>
  );
  return onClick ? (
    <button type="button" className="ui-row ui-row-click" onClick={onClick}>{body}</button>
  ) : (
    <div className="ui-row">{body}</div>
  );
}
