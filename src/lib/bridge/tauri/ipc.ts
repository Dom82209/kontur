import type {
  AssignmentLinkTarget,
  AutomationRules,
  ClassificationPatch,
  CorpusPreview,
  CorpusSelection,
  CourseLinkTarget,
  DownloadCandidate,
  DownloadsWatcher,
  DroppedItem,
  ExportImportRequest,
  ExportPreview,
  ExtractionBatchReport,
  ExtractionSettings,
  Homework,
  InboxFile,
  InboxImportReport,
  ManagedRoots,
  MaterialExtraction,
  MaterialExtractionItem,
  MaterialRef,
  Operation,
  OperationRecord,
  OperationResult,
  RootSelection,
  SelectableRootKind,
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

// ── Этап 5: управляемые каталоги, native intake, импорт во «Входящие» ──

export interface NativeIntakeItem {
  name: string;
  sizeBytes: number;
  sourcePath: string;
}

export interface ImportItemIpc {
  sourcePath: string;
  origin: "picker" | "drop";
}

export function checkManagedRoots(): Promise<ManagedRoots> {
  return call("check_managed_roots");
}

export function chooseManagedRoot(kind: SelectableRootKind): Promise<RootSelection> {
  return call("choose_managed_root", { kind });
}

export function prepareInboxRoot(): Promise<ManagedRoots> {
  return call("prepare_inbox_root");
}

/** Системный выбор нескольких файлов (диалог открывает Rust). Отмена — пустой массив. */
export function pickImportFiles(): Promise<NativeIntakeItem[]> {
  return call("pick_import_files");
}

export function importInboxFiles(
  operationId: string,
  items: ImportItemIpc[],
): Promise<OperationResult<InboxImportReport>> {
  return call("import_inbox_files", { operationId, items });
}

export function getOperationResult(operationId: string): Promise<OperationRecord> {
  return call("get_operation_result", { operationId });
}

export function openInboxFile(fileId: string): Promise<void> {
  return call("open_inbox_file", { fileId });
}

export function revealInboxFile(fileId: string): Promise<void> {
  return call("reveal_inbox_file", { fileId });
}

export function revealInFinder(relativePath: string): Promise<void> {
  return call("reveal_in_finder", { relativePath });
}

// ── Этап 6: watcher загрузок, экспорт ЦУ, ссылки LMS ──

export function getDownloadsWatcher(): Promise<DownloadsWatcher> {
  return call("get_downloads_watcher");
}

export function setDownloadsWatcher(enabled: boolean): Promise<DownloadsWatcher> {
  return call("set_downloads_watcher", { enabled });
}

export function dismissDownloadCandidate(candidateId: string): Promise<DownloadsWatcher> {
  return call("dismiss_download_candidate", { candidateId });
}

/** Системный выбор ZIP/PDF экспорта (диалог открывает Rust). Отмена — null. */
export function pickCuExport(): Promise<DownloadCandidate | null> {
  return call("pick_cu_export");
}

export function previewCuExport(candidateId: string): Promise<ExportPreview> {
  return call("preview_cu_export", { candidateId });
}

export function importCuExport(
  operationId: string,
  request: ExportImportRequest,
): Promise<OperationResult<InboxImportReport>> {
  return call("import_cu_export", {
    operationId,
    request: { ...request, courseUrl: request.courseUrl ?? null, assignmentUrl: request.assignmentUrl ?? null },
  });
}

export function setSubjectCourseUrl(subjectId: string, url: string | null): Promise<Subject> {
  return call("set_subject_course_url", { subjectId, url });
}

export function setHomeworkAssignmentUrl(homeworkId: string, url: string | null): Promise<Homework> {
  return call("set_homework_assignment_url", { homeworkId, url });
}

export function openCourse(target: CourseLinkTarget): Promise<void> {
  return call("open_course", "subjectId" in target ? { subjectId: target.subjectId, fileId: null } : { subjectId: null, fileId: target.fileId });
}

export function openAssignment(target: AssignmentLinkTarget): Promise<void> {
  return call("open_assignment", "homeworkId" in target ? { homeworkId: target.homeworkId, fileId: null } : { homeworkId: null, fileId: target.fileId });
}

// ── Этап 7: локальное извлечение текста, подборка материалов ──

const materialArg = (material: MaterialRef): MaterialRef => ({ kind: material.kind, id: material.id });

export function listMaterialExtractions(): Promise<MaterialExtractionItem[]> {
  return call("list_material_extractions");
}

export function getMaterialExtraction(material: MaterialRef): Promise<MaterialExtraction> {
  return call("get_material_extraction", { material: materialArg(material) });
}

export function analyzeMaterial(material: MaterialRef, force: boolean): Promise<MaterialExtraction> {
  return call("analyze_material", { material: materialArg(material), force });
}

export function analyzeMaterials(
  operationId: string,
  materials: MaterialRef[],
  force: boolean,
): Promise<OperationResult<ExtractionBatchReport>> {
  return call("analyze_materials", { operationId, materials: materials.map(materialArg), force });
}

export function cancelExtraction(operationId: string): Promise<boolean> {
  return call("cancel_extraction", { operationId });
}

export function clearExtractionCache(): Promise<{ removed: number }> {
  return call("clear_extraction_cache");
}

export function getExtractionSettings(): Promise<ExtractionSettings> {
  return call("get_extraction_settings");
}

export function setExtractionSettings(ocrEnabled: boolean): Promise<ExtractionSettings> {
  return call("set_extraction_settings", { ocrEnabled });
}

export function getCorpusSelection(homeworkId: string | null): Promise<CorpusSelection> {
  return call("get_corpus_selection", { homeworkId });
}

export function setCorpusSelection(homeworkId: string | null, materials: MaterialRef[]): Promise<CorpusSelection> {
  return call("set_corpus_selection", { homeworkId, materials: materials.map(materialArg) });
}

export function previewCorpus(homeworkId: string | null): Promise<CorpusPreview> {
  return call("preview_corpus", { homeworkId });
}

/** Преобразует DroppedItem в аргумент IPC; без sourcePath импорт не начинается. */
export function toImportItems(items: DroppedItem[]): ImportItemIpc[] {
  return items.map((item) => ({ sourcePath: item.sourcePath!, origin: item.origin ?? "drop" }));
}

/** Только для unit-тестов низкоуровневого IPC-клиента. */
export function __setInvokeForTests(invoke: Invoke | null | undefined): void {
  testInvoke = invoke;
}
