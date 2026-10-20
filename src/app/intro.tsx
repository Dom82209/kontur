import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { usePrefs } from "./preferences";

interface IntroApi {
  done: boolean;
  finish: () => void;
}

const Ctx = createContext<IntroApi | null>(null);
const KEY = "kontur.intro-shown";

export function IntroProvider({ children }: { children: ReactNode }) {
  const { reduceMotion } = usePrefs();
  const [done, setDone] = useState(() => reduceMotion || sessionStorage.getItem(KEY) === "1");
  const finish = useCallback(() => {
    sessionStorage.setItem(KEY, "1");
    setDone(true);
  }, []);
  const value = useMemo(() => ({ done, finish }), [done, finish]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useIntro(): IntroApi {
  const v = useContext(Ctx);
  if (!v) throw new Error("useIntro вне IntroProvider");
  return v;
}
