import { useSyncExternalStore } from "react";

function subscribe(cb: () => void): () => void {
  document.addEventListener("visibilitychange", cb);
  window.addEventListener("blur", cb);
  window.addEventListener("focus", cb);
  return () => {
    document.removeEventListener("visibilitychange", cb);
    window.removeEventListener("blur", cb);
    window.removeEventListener("focus", cb);
  };
}

/** false, когда окно свёрнуто или скрыто — циклические анимации должны остановиться. */
export function useWindowVisible(): boolean {
  return useSyncExternalStore(subscribe, () => !document.hidden, () => true);
}
