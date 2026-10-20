import type { BridgeEventName, BridgeEvents, BridgeListener, Unsubscribe } from "./types";

/** Типизированный шина событий. Используется mock- и tauri-адаптерами. */
export class TypedEmitter {
  private listeners = new Map<BridgeEventName, Set<(payload: never) => void>>();

  on<E extends BridgeEventName>(event: E, listener: BridgeListener<E>): Unsubscribe {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    const fn = listener as (payload: never) => void;
    set.add(fn);
    return () => {
      set?.delete(fn);
    };
  }

  emit<E extends BridgeEventName>(event: E, payload: BridgeEvents[E]): void {
    const set = this.listeners.get(event);
    if (!set) return;
    for (const fn of set) (fn as BridgeListener<E>)(payload);
  }
}
