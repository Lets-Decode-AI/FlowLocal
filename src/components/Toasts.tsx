import { createContext, useCallback, useContext, useState, type ReactNode } from 'react';
import { useEvent } from '../lib/hooks';
import Icon from './ui/Icon';

interface Item { id: number; level: string; message: string }
type Push = (message: string, level?: string) => void;
const Ctx = createContext<Push>(() => {});
export const useToast = () => useContext(Ctx);

let seq = 0;
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Item[]>([]);
  const push = useCallback<Push>((message, level = 'info') => {
    const id = ++seq;
    setItems((x) => [...x.slice(-3), { id, level, message }]);
    setTimeout(() => setItems((x) => x.filter((i) => i.id !== id)), 5000);
  }, []);
  useEvent('toast', (t) => push(t.message, t.level));
  return (
    <Ctx.Provider value={push}>
      {children}
      <div className="toasts" role="status" aria-live="polite">
        {items.map((t) => (
          <div key={t.id} className={`toast toast-${t.level}`}>
            {(t.level === 'error' || t.level === 'warning') && <Icon name="warning" />}
            <span>{t.message}</span>
          </div>
        ))}
      </div>
    </Ctx.Provider>
  );
}
