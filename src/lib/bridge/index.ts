/**
 * Единая точка доступа к backend.
 *
 * Компоненты импортируют ТОЛЬКО `bridge` отсюда. В настоящем Tauri runtime всегда
 * используется цельный `TauriDesktopBridge`; ошибки IPC не подменяются demo-данными.
 * Mock доступен только тестам и явно включённому browser demo
 * (`VITE_KONTUR_BRIDGE=demo npm run dev`).
 */
import { MockDesktopBridge } from "./mock/MockDesktopBridge";
import { createTauriBridge } from "./tauri";
import { isTauriRuntimeAvailable } from "./tauri/runtime";
import type { DesktopBridge } from "./types";

export function createBridge(flag: string | undefined = import.meta.env.VITE_KONTUR_BRIDGE): DesktopBridge {
  if (isTauriRuntimeAvailable()) return createTauriBridge();
  if (flag === "demo") return new MockDesktopBridge();
  return createTauriBridge();
}

export const bridge: DesktopBridge = createBridge();

export * from "./types";
export * from "./intake";
