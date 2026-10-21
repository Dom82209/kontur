/**
 * Единая точка доступа к backend.
 *
 * Компоненты импортируют ТОЛЬКО `bridge` отсюда. По умолчанию это `MockDesktopBridge`:
 * основной UI на production bridge пока не переключён (см. FRONTEND_HANDOFF.md).
 *
 * Только для ручной проверки реального импорта разработчиком:
 * `VITE_KONTUR_BRIDGE=tauri npm run tauri:dev` создаёт `TauriDesktopBridge` целиком —
 * без смешивания с mock. Ещё не реализованные backend-методы при этом честно
 * отвечают `backend_unavailable`. Ни один npm-скрипт этот флаг не выставляет.
 */
import { MockDesktopBridge } from "./mock/MockDesktopBridge";
import { createTauriBridge } from "./tauri";
import { isTauriRuntimeAvailable } from "./tauri/runtime";
import type { DesktopBridge } from "./types";

export function createBridge(flag: string | undefined = import.meta.env.VITE_KONTUR_BRIDGE): DesktopBridge {
  if (flag === "tauri" && isTauriRuntimeAvailable()) return createTauriBridge();
  return new MockDesktopBridge();
}

export const bridge: DesktopBridge = createBridge();

export * from "./types";
export * from "./intake";
