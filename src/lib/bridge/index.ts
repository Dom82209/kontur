/**
 * Единая точка доступа к backend.
 *
 * Компоненты импортируют ТОЛЬКО `bridge` отсюда. Чтобы подключить настоящий backend,
 * достаточно заменить адаптер в createBridge() (см. FRONTEND_HANDOFF.md → «Подключение backend»).
 */
import { MockDesktopBridge } from "./mock/MockDesktopBridge";
import type { DesktopBridge } from "./types";

function createBridge(): DesktopBridge {
  // Точка замены адаптера. Пример для backend-разработчика:
  //   if (import.meta.env.VITE_BRIDGE === "tauri") return new TauriDesktopBridge();
  return new MockDesktopBridge();
}

export const bridge: DesktopBridge = createBridge();

export * from "./types";
export * from "./intake";
