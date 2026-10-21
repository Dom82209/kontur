import { BridgeError, type BridgeErrorCode } from "../types";

const ERROR_CODES = new Set<BridgeErrorCode>([
  "not_found",
  "conflict",
  "duplicate",
  "provider_unavailable",
  "reauth_required",
  "confirmation_required",
  "io_error",
  "not_undoable",
  "immutable",
  "blocked",
  "cancelled",
  "database",
  "backend_unavailable",
  "unknown",
]);

export function toBridgeError(error: unknown): BridgeError {
  if (error instanceof BridgeError) return error;
  if (typeof error === "object" && error !== null) {
    const candidate = error as { code?: unknown; message?: unknown };
    if (typeof candidate.code === "string" && typeof candidate.message === "string") {
      const code = ERROR_CODES.has(candidate.code as BridgeErrorCode)
        ? (candidate.code as BridgeErrorCode)
        : "unknown";
      return new BridgeError(code, candidate.message);
    }
  }
  const normalized = new BridgeError(
    "unknown",
    error instanceof Error ? error.message : "Неизвестная ошибка backend",
  );
  if (error instanceof Error && error.stack) normalized.stack = error.stack;
  return normalized;
}

export function unsupportedBackend(feature: string): Promise<never> {
  return Promise.reject(
    new BridgeError("backend_unavailable", `Функция «${feature}» ещё не реализована в Tauri backend`),
  );
}

export function unsupportedBackendSync(feature: string): never {
  throw new BridgeError("backend_unavailable", `Функция «${feature}» ещё не реализована в Tauri backend`);
}
