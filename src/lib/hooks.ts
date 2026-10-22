import { useEffect, useRef, useState } from 'react';
import { on, type EventPayloads } from '../ipc';

export function useHash(): string {
  const read = () => window.location.hash.replace(/^#/, '') || '/history';
  const [h, setH] = useState(read);
  useEffect(() => {
    const f = () => setH(read());
    window.addEventListener('hashchange', f);
    return () => window.removeEventListener('hashchange', f);
  }, []);
  return h;
}

export const navigate = (route: string) => {
  window.location.hash = `#${route}`;
};

/** Subscribe to a backend event for the lifetime of the component. */
export function useEvent<K extends keyof EventPayloads>(name: K, cb: (p: EventPayloads[K]) => void) {
  const ref = useRef(cb);
  ref.current = cb;
  useEffect(() => {
    let un: (() => void) | undefined;
    let dead = false;
    on(name, (p) => ref.current(p)).then((u) => (dead ? u() : (un = u)));
    return () => {
      dead = true;
      un?.();
    };
  }, [name]);
}
