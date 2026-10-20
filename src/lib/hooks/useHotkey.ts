import { useEffect, useRef } from "react";

export interface HotkeyOptions {
  meta?: boolean;
  shift?: boolean;
  /** Срабатывать, даже если фокус в поле ввода. */
  allowInInput?: boolean;
  enabled?: boolean;
}

export function isTypingTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName);
}

/** key — значение KeyboardEvent.key в нижнем регистре или code (KeyK). Cmd на macOS, Ctrl в остальных системах. */
export function useHotkey(key: string, handler: (e: KeyboardEvent) => void, opts: HotkeyOptions = {}): void {
  const ref = useRef(handler);
  ref.current = handler;
  const { meta = false, shift = false, allowInInput = false, enabled = true } = opts;
  useEffect(() => {
    if (!enabled) return;
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (meta !== mod || shift !== e.shiftKey) return;
      if (e.key.toLowerCase() !== key && e.code !== key) return;
      if (!allowInInput && isTypingTarget(e.target)) return;
      e.preventDefault();
      ref.current(e);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [key, meta, shift, allowInInput, enabled]);
}
