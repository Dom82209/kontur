import { BridgeError, type DroppedItem } from "./types";

/**
 * Native Tauri intake must provide an absolute sourcePath for every file.
 * Browser/HTML5 intake intentionally has no path and must not call this guard.
 */
export function assertNativeSourcePaths(items: DroppedItem[]): void {
  const missing = items.filter((item) => !item.sourcePath?.trim());
  if (missing.length > 0) {
    throw new BridgeError(
      "io_error",
      `Tauri не передал исходный путь для ${missing.length} файл(ов). Импорт остановлен без имитации файловой операции.`,
    );
  }
}

/** Преобразование путей из Tauri drag-drop event; размер уточняет native backend. */
export function droppedItemsFromNativePaths(paths: string[]): DroppedItem[] {
  const items = paths.map((sourcePath) => ({
    name: sourcePath.split("/").filter(Boolean).pop() ?? sourcePath,
    sizeBytes: 0,
    sourcePath,
  }));
  assertNativeSourcePaths(items);
  return items;
}
