import { motion } from "motion/react";
import { useId, type ReactNode } from "react";

export interface TabItem<T extends string> {
  value: T;
  label: string;
  badge?: ReactNode;
}

/** Вкладки с перетекающим подчёркиванием. ←/→ переключают вкладку. */
export function Tabs<T extends string>({ value, items, onChange, label }: { value: T; items: TabItem<T>[]; onChange: (v: T) => void; label: string }) {
  const id = useId();
  const move = (d: 1 | -1) => {
    const i = items.findIndex((t) => t.value === value);
    const next = items[(i + d + items.length) % items.length];
    if (next) onChange(next.value);
  };
  return (
    <div
      className="tabs"
      role="tablist"
      aria-label={label}
      onKeyDown={(e) => {
        if (e.key === "ArrowRight") move(1);
        if (e.key === "ArrowLeft") move(-1);
      }}
    >
      {items.map((t) => (
        <button key={t.value} role="tab" aria-selected={t.value === value} tabIndex={t.value === value ? 0 : -1} className="tabs__tab" onClick={() => onChange(t.value)}>
          {t.label}
          {t.badge}
          {t.value === value && <motion.span layoutId={`tabs-${id}`} className="tabs__line" />}
        </button>
      ))}
    </div>
  );
}
