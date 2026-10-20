import { useEffect, useRef } from "react";
import { bridge, type BridgeEventName, type BridgeListener } from "../bridge";

/** Подписка на событие backend. Обработчик всегда актуален, переподписки нет. */
export function useBridgeEvent<E extends BridgeEventName>(event: E, handler: BridgeListener<E>): void {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => bridge.on(event, (payload) => ref.current(payload)), [event]);
}
