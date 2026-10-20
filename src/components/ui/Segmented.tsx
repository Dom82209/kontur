import { motion } from "motion/react";
import { useId, type ReactNode } from "react";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

interface Props<T extends string> {
  value: T;
  options: SegmentedOption<T>[];
  onChange: (v: T) => void;
  label: string;
}

/** Радиогруппа с «ползунком», который перетекает между вариантами (shared layoutId). */
export function Segmented<T extends string>({ value, options, onChange, label }: Props<T>) {
  const id = useId();
  const move = (dir: 1 | -1) => {
    const i = options.findIndex((o) => o.value === value);
    const next = options[(i + dir + options.length) % options.length];
    if (next) onChange(next.value);
  };
  return (
    <div
      className="segmented"
      role="radiogroup"
      aria-label={label}
      onKeyDown={(e) => {
        if (e.key === "ArrowRight") move(1);
        if (e.key === "ArrowLeft") move(-1);
      }}
    >
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button key={o.value} type="button" role="radio" aria-checked={active} tabIndex={active ? 0 : -1} className="segmented__item" onClick={() => onChange(o.value)}>
            {active && <motion.span layoutId={`seg-${id}`} className="segmented__thumb" />}
            {o.icon}
            <span className="segmented__label">{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
