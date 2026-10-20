import { useCallback, useEffect, useRef, useState } from "react";
import { bridge, type BridgeEventName } from "../bridge";

export interface QueryState<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  reload: () => Promise<void>;
  setData: (updater: (prev: T | null) => T | null) => void;
}

/**
 * Загружает данные через bridge и перезагружает их по событиям backend.
 * Перезагрузка «тихая»: loading=true только при первой загрузке.
 */
export function useBridgeQuery<T>(load: () => Promise<T>, deps: readonly unknown[], refetchOn: readonly BridgeEventName[] = []): QueryState<T> {
  const [data, setDataState] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const loadRef = useRef(load);
  loadRef.current = load;
  const seq = useRef(0);

  const run = useCallback(async (silent: boolean) => {
    const id = ++seq.current;
    if (!silent) setLoading(true);
    try {
      const v = await loadRef.current();
      if (id === seq.current) {
        setDataState(v);
        setError(null);
      }
    } catch (e) {
      if (id === seq.current) setError(e instanceof Error ? e.message : "Неизвестная ошибка");
    } finally {
      if (id === seq.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    setDataState(null);
    void run(false);
  }, deps);

  const events = refetchOn.join("|");
  useEffect(() => {
    if (!events) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const schedule = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void run(true), 60);
    };
    const offs = (events.split("|") as BridgeEventName[]).map((e) => bridge.on(e, schedule));
    return () => {
      if (timer) clearTimeout(timer);
      offs.forEach((off) => off());
    };
  }, [events, run]);

  const setData = useCallback((updater: (prev: T | null) => T | null) => setDataState((p) => updater(p)), []);
  return { data, error, loading, reload: () => run(true), setData };
}
