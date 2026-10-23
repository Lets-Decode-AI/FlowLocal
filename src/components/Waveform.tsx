import { useEffect, useRef, useState } from 'react';

export const BAR_COUNT = 16;
/** Slight fixed per-bar variation so the wave never looks like a flat block. */
const VARIATION = [0.82, 0.95, 0.88, 1, 0.9, 0.97, 0.85, 1, 0.93, 0.86, 0.98, 0.9, 1, 0.84, 0.94, 0.88];
const MIN_SCALE = 0.12;
const DECAY_MS = 120;

/** Scrolling 16-bar voice waveform. Bars are driven by `level` (~0..1) and animated with
 * transforms only: fast attack, ~120 ms exponential decay. */
export default function Waveform({ level }: { level: number }) {
  const [targets, setTargets] = useState<number[]>(() => Array(BAR_COUNT).fill(0));
  const barRefs = useRef<(HTMLSpanElement | null)[]>([]);
  const shown = useRef<number[]>(Array(BAR_COUNT).fill(0));
  const targetRef = useRef(targets);
  targetRef.current = targets;

  // Newest sample enters on the right; older ones shift left.
  useEffect(() => {
    const v = Math.min(1, Math.max(0, level));
    setTargets((t) => [...t.slice(1), v]);
  }, [level]);

  useEffect(() => {
    if (typeof requestAnimationFrame === 'undefined') return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = now - last;
      last = now;
      const k = 1 - Math.exp(-dt / DECAY_MS);
      targetRef.current.forEach((t, i) => {
        const goal = Math.min(1, t * VARIATION[i]);
        const cur = shown.current[i];
        // Attack: jump straight up; decay: ease down.
        const next = goal > cur ? goal : cur + (goal - cur) * k;
        shown.current[i] = next;
        const el = barRefs.current[i];
        if (el) el.style.transform = `scaleY(${MIN_SCALE + (1 - MIN_SCALE) * next})`;
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div className="flex h-full items-center justify-center gap-[3px]" aria-hidden="true">
      {targets.map((t, i) => (
        <span
          key={i}
          ref={(el) => {
            barRefs.current[i] = el;
          }}
          data-testid="wave-bar"
          data-level={(t * VARIATION[i]).toFixed(3)}
          className="fb-bar"
          style={{ transform: `scaleY(${MIN_SCALE + (1 - MIN_SCALE) * t * VARIATION[i]})` }}
        />
      ))}
    </div>
  );
}
