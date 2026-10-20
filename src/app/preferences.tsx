import { MotionConfig } from "motion/react";
import { createContext, useContext, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import { useWindowVisible } from "../lib/hooks/useWindowVisible";

export type ThemePref = "system" | "light" | "dark";
export type MotionPref = "system" | "reduced" | "full";

interface Prefs {
  theme: ThemePref;
  motion: MotionPref;
  resolvedTheme: "light" | "dark";
  /** Итоговое решение: показывать ли анимации движения. */
  reduceMotion: boolean;
  setTheme: (t: ThemePref) => void;
  setMotion: (m: MotionPref) => void;
}

const Ctx = createContext<Prefs | null>(null);
// В localStorage только UI-настройки. Никаких секретов.
const KEY = "kontur.ui-prefs";

function readStored(): { theme: ThemePref; motion: MotionPref } {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const v = JSON.parse(raw) as Partial<{ theme: ThemePref; motion: MotionPref }>;
      return { theme: v.theme ?? "system", motion: v.motion ?? "system" };
    }
  } catch {
    /* пустое хранилище — значения по умолчанию */
  }
  return { theme: "dark", motion: "system" };
}

function mediaStore(query: string) {
  return {
    subscribe(cb: () => void) {
      const m = window.matchMedia(query);
      m.addEventListener("change", cb);
      return () => m.removeEventListener("change", cb);
    },
    get: () => window.matchMedia(query).matches,
  };
}
const darkMq = mediaStore("(prefers-color-scheme: dark)");
const reducedMq = mediaStore("(prefers-reduced-motion: reduce)");

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState(readStored);
  const systemDark = useSyncExternalStore(darkMq.subscribe, darkMq.get, () => true);
  const systemReduced = useSyncExternalStore(reducedMq.subscribe, reducedMq.get, () => false);
  const visible = useWindowVisible();

  const resolvedTheme = state.theme === "system" ? (systemDark ? "dark" : "light") : state.theme;
  const reduceMotion = state.motion === "reduced" || (state.motion === "system" && systemReduced);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = resolvedTheme;
    root.dataset.motion = reduceMotion ? "reduced" : state.motion === "full" ? "full" : "system";
    root.dataset.windowHidden = String(!visible);
  }, [resolvedTheme, reduceMotion, state.motion, visible]);

  useEffect(() => {
    try {
      localStorage.setItem(KEY, JSON.stringify(state));
    } catch {
      /* недоступно — не критично */
    }
  }, [state]);

  const value = useMemo<Prefs>(
    () => ({
      ...state,
      resolvedTheme,
      reduceMotion,
      setTheme: (theme) => setState((s) => ({ ...s, theme })),
      setMotion: (motion) => setState((s) => ({ ...s, motion })),
    }),
    [state, resolvedTheme, reduceMotion],
  );

  return (
    <Ctx.Provider value={value}>
      <MotionConfig reducedMotion={reduceMotion ? "always" : "never"} transition={{ type: "spring", stiffness: 420, damping: 38, mass: 0.9 }}>
        {children}
      </MotionConfig>
    </Ctx.Provider>
  );
}

export function usePrefs(): Prefs {
  const v = useContext(Ctx);
  if (!v) throw new Error("usePrefs вне PreferencesProvider");
  return v;
}
