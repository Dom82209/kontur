import { BridgeError } from "../types";

let runtimeOverride: boolean | undefined;

export function isTauriRuntimeAvailable(): boolean {
  return runtimeOverride ?? ("__TAURI_INTERNALS__" in globalThis);
}

export function requireTauriRuntime(): void {
  if (!isTauriRuntimeAvailable()) {
    throw new BridgeError("backend_unavailable", "Tauri backend недоступен вне нативного приложения");
  }
}

/** Только для unit-тестов runtime boundary. */
export function __setTauriRuntimeForTests(available: boolean | undefined): void {
  runtimeOverride = available;
}
