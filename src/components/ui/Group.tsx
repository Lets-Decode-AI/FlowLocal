import type { ReactNode } from 'react';

/** Inset grouped list container, like a System Settings section. */
export default function Group({ title, footer, children }: { title?: string; footer?: ReactNode; children: ReactNode }) {
  return (
    <section className="ui-group">
      {title && <h2 className="ui-group-title">{title}</h2>}
      <div className="ui-group-body">{children}</div>
      {footer && <p className="ui-group-footer">{footer}</p>}
    </section>
  );
}
