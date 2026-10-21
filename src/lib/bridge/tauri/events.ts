import type { BridgeEventName, BridgeEvents, BridgeListener, Unsubscribe } from "../types";
import { toBridgeError } from "./errors";
import { requireTauriRuntime } from "./runtime";

interface TauriEvent {
  payload: unknown;
}

type Listen = (event: string, handler: (message: TauriEvent) => void) => Promise<Unsubscribe>;
type RegistrationErrorHandler = (error: Error) => void;

let testListen: Listen | null | undefined;
let registrationErrorHandler: RegistrationErrorHandler = (error) => {
  console.error("[Контур] Не удалось зарегистрировать Tauri event listener", error);
};

function safelyUnlisten(unlisten: Unsubscribe): void {
  try {
    unlisten();
  } catch (error) {
    registrationErrorHandler(toBridgeError(error));
  }
}

async function resolveListen(): Promise<Listen> {
  if (testListen !== undefined) {
    if (testListen === null) {
      requireTauriRuntime();
      throw new Error("Недоступный test event adapter");
    }
    return testListen;
  }

  requireTauriRuntime();
  try {
    const { listen } = await import("@tauri-apps/api/event");
    return (event, handler) => listen(event, handler);
  } catch (error) {
    throw toBridgeError(error);
  }
}

export function listenToBridgeEvent<E extends BridgeEventName>(
  event: E,
  listener: BridgeListener<E>,
): Unsubscribe {
  let active = true;
  let registeredUnlisten: Unsubscribe | undefined;

  void resolveListen()
    .then((listen) =>
      listen(event, (message) => {
        if (active) listener(message.payload as BridgeEvents[E]);
      }),
    )
    .then((unlisten) => {
      if (active) {
        registeredUnlisten = unlisten;
      } else {
        safelyUnlisten(unlisten);
      }
    })
    .catch((error: unknown) => {
      registrationErrorHandler(toBridgeError(error));
    });

  return () => {
    if (!active) return;
    active = false;
    const unlisten = registeredUnlisten;
    registeredUnlisten = undefined;
    if (unlisten) safelyUnlisten(unlisten);
  };
}

/** Только для unit-тестов event boundary. */
export function __setListenForTests(listen: Listen | null | undefined): void {
  testListen = listen;
}

/** Только для unit-тестов контролируемой обработки ошибок регистрации. */
export function __setRegistrationErrorHandlerForTests(handler: RegistrationErrorHandler | undefined): void {
  registrationErrorHandler = handler ?? ((error) => console.error("[Контур] Не удалось зарегистрировать Tauri event listener", error));
}
