import { AnimatePresence, motion } from "motion/react";
import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from "lucide-react";
import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from "react";

export type ToastTone = "ok" | "info" | "warn" | "danger";

export interface ToastInput {
  tone: ToastTone;
  title: string;
  text?: string;
  action?: { label: string; run: () => void };
  /** мс; 0 — не закрывать автоматически */
  duration?: number;
}

interface ToastItem extends ToastInput {
  id: number;
}

interface ToastApi {
  push: (t: ToastInput) => number;
  dismiss: (id: number) => void;
}

const Ctx = createContext<ToastApi | null>(null);

const ICONS = { ok: CircleCheck, info: Info, warn: TriangleAlert, danger: CircleAlert } as const;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const seq = useRef(0);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());

  const dismiss = useCallback((id: number) => {
    setItems((list) => list.filter((t) => t.id !== id));
    const tm = timers.current.get(id);
    if (tm) clearTimeout(tm);
    timers.current.delete(id);
  }, []);

  const push = useCallback(
    (t: ToastInput) => {
      seq.current += 1;
      const id = seq.current;
      setItems((list) => [...list.slice(-3), { ...t, id }]);
      const duration = t.duration ?? (t.action ? 7000 : 4200);
      if (duration > 0) timers.current.set(id, setTimeout(() => dismiss(id), duration));
      return id;
    },
    [dismiss],
  );

  const api = useMemo(() => ({ push, dismiss }), [push, dismiss]);

  return (
    <Ctx.Provider value={api}>
      {children}
      <div className="toasts" role="region" aria-live="polite" aria-label="Уведомления">
        <AnimatePresence initial={false}>
          {items.map((t) => {
            const Icon = ICONS[t.tone];
            return (
              <motion.div
                key={t.id}
                layout
                className="toast"
                data-tone={t.tone}
                role={t.tone === "danger" ? "alert" : "status"}
                initial={{ opacity: 0, y: 14, scale: 0.98 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, x: 24, transition: { duration: 0.16 } }}
              >
                {t.tone === "ok" && (
                  // Короткий контролируемый импульс успеха — одна волна по левому краю.
                  <motion.span className="toast__pulse" initial={{ scaleY: 0, opacity: 1 }} animate={{ scaleY: 1, opacity: [1, 1, 0.35] }} transition={{ duration: 0.5, ease: [0.2, 0.8, 0.2, 1] }} />
                )}
                <span className="toast__icon">
                  <Icon />
                </span>
                <div style={{ minWidth: 0 }}>
                  <div className="toast__title">{t.title}</div>
                  {t.text && <div className="toast__text">{t.text}</div>}
                </div>
                <div className="toast__actions">
                  {t.action && (
                    <button
                      className="btn btn--sm"
                      onClick={() => {
                        t.action?.run();
                        dismiss(t.id);
                      }}
                    >
                      {t.action.label}
                    </button>
                  )}
                  <button className="btn btn--sm btn--ghost btn--icon" aria-label="Закрыть уведомление" onClick={() => dismiss(t.id)}>
                    <X />
                  </button>
                </div>
              </motion.div>
            );
          })}
        </AnimatePresence>
      </div>
    </Ctx.Provider>
  );
}

export function useToast(): ToastApi {
  const v = useContext(Ctx);
  if (!v) throw new Error("useToast вне ToastProvider");
  return v;
}
