import { bridge } from "../bridge";
import { useBridgeEvent } from "./useBridgeEvent";
import { useBridgeQuery } from "./useBridgeQuery";

/** Состояние watcher загрузок с живым обновлением по `downloads:watcher-updated`. */
export function useDownloadsWatcher() {
  const q = useBridgeQuery(() => bridge.getDownloadsWatcher(), []);
  useBridgeEvent("downloads:watcher-updated", ({ watcher }) => q.setData(() => watcher));
  return q;
}
