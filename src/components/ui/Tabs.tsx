import { motion } from "motion/react";
import { useId, useRef, type ReactNode } from "react";

export interface TabItem<T extends string> {
  value: T;
  label: string;
  badge?: ReactNode;
}

/** Вкладки с перетекающим подчёркиванием. ←/→ переключают вкладку. */
export function Tabs<T extends string>({ value, items, onChange, label, idBase }: { value: T; items: TabItem<T>[]; onChange: (v: T) => void; label: string; idBase?: string }) {
  const id = useId();
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const base = idBase ?? `tabs-${id.replace(/:/g, "")}`;
  const select = (i: number) => {
    const next = items[i];
    if (!next) return;
    onChange(next.value);
    requestAnimationFrame(() => refs.current[i]?.focus());
  };
  const move = (d: 1 | -1) => {
    const i = items.findIndex((t) => t.value === value);
    select((i + d + items.length) % items.length);
  };
  return (
    <div
      className="tabs"
      role="tablist"
      aria-label={label}
      onKeyDown={(e) => {
        if (e.key === "ArrowRight") { e.preventDefault(); move(1); }
        if (e.key === "ArrowLeft") { e.preventDefault(); move(-1); }
        if (e.key === "Home") { e.preventDefault(); select(0); }
        if (e.key === "End") { e.preventDefault(); select(items.length - 1); }
      }}
    >
      {items.map((t, i) => (
        <button ref={(el) => { refs.current[i] = el; }} key={t.value} id={`${base}-tab-${t.value}`} aria-controls={`${base}-panel-${t.value}`} role="tab" aria-selected={t.value === value} tabIndex={t.value === value ? 0 : -1} className="tabs__tab" onClick={() => onChange(t.value)}>
          {t.label}
          {t.badge}
          {t.value === value && <motion.span layoutId={`tabs-${id}`} className="tabs__line" />}
        </button>
      ))}
    </div>
  );
}
