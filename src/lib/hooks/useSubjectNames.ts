import { useCallback } from "react";
import { bridge } from "../bridge";
import { useBridgeQuery } from "./useBridgeQuery";

/** Возвращает функцию id → название предмета (данные из bridge.getSubjects). */
export function useSubjectNames(short = false): (id: string | null | undefined) => string {
  const { data } = useBridgeQuery(() => bridge.getSubjects(), []);
  return useCallback((id) => {
    const s = data?.find((x) => x.id === id);
    return s ? (short ? s.shortName : s.name) : "—";
  }, [data, short]);
}
