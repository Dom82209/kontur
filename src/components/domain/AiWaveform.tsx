import { useEffect, useRef } from "react";
import { usePrefs } from "../../app/preferences";
import { useWindowVisible } from "../../lib/hooks/useWindowVisible";

interface Props {
  active: boolean;
  progress: number;
  label: string;
}

const BARS = 36;

/**
 * Спокойная волна обработки: амплитуда растёт с прогрессом, фаза медленно бежит.
 * Работает только пока идёт обработка и окно видно; анимируется transform (scaleY).
 */
export function AiWaveform({ active, progress, label }: Props) {
  const ref = useRef<SVGGElement>(null);
  const visible = useWindowVisible();
  const { reduceMotion } = usePrefs();
  const running = active && visible && !reduceMotion;

  useEffect(() => {
    const g = ref.current;
    if (!g) return;
    const bars = [...g.querySelectorAll<SVGRectElement>("rect")];
    if (!running) {
      bars.forEach((b, i) => {
        const filled = i / BARS < progress;
        b.style.transform = `scaleY(${active && filled ? 0.5 : 0.12})`;
      });
      return;
    }
    let raf = 0;
    const start = performance.now();
    const tick = (now: number) => {
      const t = (now - start) / 1000;
      bars.forEach((b, i) => {
        const x = i / BARS;
        const env = Math.sin(Math.PI * x);
        const wave = 0.5 + 0.5 * Math.sin(t * 2.2 - i * 0.42) * Math.cos(t * 0.7 + i * 0.13);
        const amp = 0.18 + env * (0.35 + 0.5 * wave);
        b.style.transform = `scaleY(${amp.toFixed(3)})`;
        b.dataset.filled = String(x < progress);
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [running, active, progress]);

  return (
    <div className="wave" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)}>
      <svg viewBox={`0 0 ${BARS * 6} 40`} preserveAspectRatio="none" aria-hidden>
        <g ref={ref}>
          {Array.from({ length: BARS }, (_, i) => (
            <rect key={i} x={i * 6 + 1.5} y={2} width={3} height={36} rx={1.5} data-filled={i / BARS < progress} style={{ transformOrigin: `${i * 6 + 3}px 20px`, transform: "scaleY(0.12)" }} />
          ))}
        </g>
      </svg>
    </div>
  );
}
