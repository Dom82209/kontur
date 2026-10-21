import type {
  AutomationRules,
  Homework,
  InboxFile,
  Operation,
  Subject,
  Week,
} from "../types";
import { BridgeError, type BridgeErrorCode } from "../types";

export interface BackendHealth {
  ready: boolean;
  appVersion: string;
  schemaVersion: number;
  platform: string;
  databaseExists: boolean;
}

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

let testInvoke: Invoke | null | undefined;

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

function hasTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in globalThis;
}

async function resolveInvoke(): Promise<Invoke> {
  if (testInvoke !== undefined) {
    if (testInvoke === null) {
      throw new BridgeError("backend_unavailable", "Tauri backend недоступен вне нативного приложения");
    }
    return testInvoke;
  }

  if (!hasTauriRuntime()) {
    throw new BridgeError("backend_unavailable", "Tauri backend недоступен вне нативного приложения");
  }

  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return (command, args) => invoke(command, args);
  } catch {
    throw new BridgeError("backend_unavailable", "Не удалось загрузить Tauri IPC runtime");
  }
}

function toBridgeError(error: unknown): BridgeError {
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
  return new BridgeError("unknown", error instanceof Error ? error.message : "Неизвестная ошибка backend");
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const invoke = await resolveInvoke();
    return (await invoke(command, args)) as T;
  } catch (error) {
    throw toBridgeError(error);
  }
}

export function backendHealth(): Promise<BackendHealth> {
  return call("backend_health");
}

export function getSubjects(): Promise<Subject[]> {
  return call("get_subjects");
}

export function getWeeks(subjectId: string): Promise<Week[]> {
  return call("get_weeks", { subjectId });
}

export function getWeek(subjectId: string, weekNumber: number): Promise<Week> {
  return call("get_week", { subjectId, weekNumber });
}

export function getHomeworkList(): Promise<Homework[]> {
  return call("get_homework_list");
}

export function getHomework(homeworkId: string): Promise<Homework> {
  return call("get_homework", { homeworkId });
}

export function getInboxFiles(): Promise<InboxFile[]> {
  return call("get_inbox_files");
}

export function getAutomationRules(): Promise<AutomationRules> {
  return call("get_automation_rules");
}

export function getActivityLog(): Promise<Operation[]> {
  return call("get_activity_log");
}

/** Только для unit-тестов низкоуровневого IPC-клиента. */
export function __setInvokeForTests(invoke: Invoke | null | undefined): void {
  testInvoke = invoke;
}
