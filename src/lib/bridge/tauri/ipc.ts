import type {
  AutomationRules,
  ClassificationPatch,
  Homework,
  InboxFile,
  Operation,
  OperationResult,
  Subject,
  Week,
} from "../types";
import { BridgeError } from "../types";
import { toBridgeError } from "./errors";
import { requireTauriRuntime } from "./runtime";

export interface BackendHealth {
  ready: boolean;
  appVersion: string;
  schemaVersion: number;
  platform: string;
  databaseExists: boolean;
}

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

let testInvoke: Invoke | null | undefined;

async function resolveInvoke(): Promise<Invoke> {
  if (testInvoke !== undefined) {
    if (testInvoke === null) {
      throw new BridgeError("backend_unavailable", "Tauri backend недоступен вне нативного приложения");
    }
    return testInvoke;
  }

  requireTauriRuntime();

  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return (command, args) => invoke(command, args);
  } catch {
    throw new BridgeError("backend_unavailable", "Не удалось загрузить Tauri IPC runtime");
  }
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

type PatchField<T> =
  | { state: "absent" }
  | { state: "null" }
  | { state: "value"; value: T };

interface ClassificationPatchIpc {
  subjectId: PatchField<string>;
  weekNumber: PatchField<number>;
  category: PatchField<NonNullable<ClassificationPatch["category"]>>;
}

function patchField<K extends keyof ClassificationPatch>(
  patch: ClassificationPatch,
  key: K,
): PatchField<NonNullable<ClassificationPatch[K]>> {
  if (!Object.prototype.hasOwnProperty.call(patch, key) || patch[key] === undefined) {
    return { state: "absent" };
  }
  const value = patch[key];
  if (value === null) return { state: "null" };
  return { state: "value", value };
}

export function updateAutomationRules(
  rules: AutomationRules,
): Promise<OperationResult<AutomationRules>> {
  return call("update_automation_rules", { rules });
}

export function updateClassification(
  fileId: string,
  patch: ClassificationPatch,
): Promise<InboxFile> {
  const encoded: ClassificationPatchIpc = {
    subjectId: patchField(patch, "subjectId"),
    weekNumber: patchField(patch, "weekNumber"),
    category: patchField(patch, "category"),
  };
  return call("update_classification", { fileId, patch: encoded });
}

/** Только для unit-тестов низкоуровневого IPC-клиента. */
export function __setInvokeForTests(invoke: Invoke | null | undefined): void {
  testInvoke = invoke;
}
