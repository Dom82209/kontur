import { BridgeError, type DroppedItem, type InboxFile, type InboxImportReport } from "./types";

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

/** UUID v4 для operationId импорта: связывает operation:progress с вызовом и делает повтор безопасным. */
export function newOperationId(): string {
  const cryptoApi = globalThis.crypto;
  if (typeof cryptoApi?.randomUUID === "function") return cryptoApi.randomUUID();
  const bytes = new Uint8Array(16);
  cryptoApi.getRandomValues(bytes);
  bytes[6] = (bytes[6]! & 0x0f) | 0x40;
  bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** Файлы «Входящих», появившиеся или найденные при импорте (без ошибок). */
export function filesFromImportReport(report: InboxImportReport): InboxFile[] {
  return report.items.flatMap((item) => (item.status === "failed" ? [] : [item.file]));
}
