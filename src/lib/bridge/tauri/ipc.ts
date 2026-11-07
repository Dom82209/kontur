import type {
  AiConfirmation,
  AiConfirmInput,
  AiJob,
  AiTask,
  ProviderId,
  ProviderStatus,
  IsolationReport,
  TransportPreference,
  AssignmentLinkTarget,
  Dashboard,
  FileRef,
  HomeworkIteration,
  HomeworkVersion,
  HomeworkWorkflow,
  HomeworkWorkflowStage,
  ManualAnalysisInput,
  ManualReviewInput,
  ReviewRun,
  RevisionPrompt,
  RunKind,
  RunMode,
  SourceBundle,
  StyleConfirmation,
  CommentAuthorityInput,
  TeacherComment,
  TeacherCommentInput,
  TeacherRequirement,
  TeacherRequirementInput,
  VersionAuthor,
  VersionComparison,
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
  RouteDestination,
  RoutePreview,
  SelectableRootKind,
  Subject,
  Week,
  StyleProfile,
  StyleProfileState,
  StyleSourceKind,
  StyleLanguage,
  StyleCheck,
  TeacherTranscriptInput,
  SubmissionPackage,
  SubmissionReceipt,
  SubmissionConfirmation,
  PackageCheckId,
  WorkspaceActivation,
  WorkspaceOverview,
  WorkspaceTransitionPreview,
  WorkspaceTransitionRequest,
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

export function createSubject(input: import("../types").SubjectInput): Promise<Subject> {
  return call("create_subject", { input });
}

export function updateSubject(subjectId: string, revision: number, input: import("../types").SubjectInput): Promise<Subject> {
  return call("update_subject", { subjectId, revision, input });
}

export function setSubjectArchived(subjectId: string, revision: number, archived: boolean): Promise<Subject> {
  return call("set_subject_archived", { subjectId, revision, archived });
}

export function previewSubjectMerge(sourceId: string, targetId: string): Promise<import("../types").SubjectMergePreview> {
  return call("preview_subject_merge", { sourceId, targetId });
}

export function mergeSubjects(sourceId: string, targetId: string, fingerprint: string): Promise<import("../types").SubjectMergeResult> {
  return call("merge_subjects", { sourceId, targetId, fingerprint });
}

export function createHomework(input: import("../types").NewHomeworkInput): Promise<import("../types").Homework> {
  return call("create_homework", { input });
}

export function getSemester(): Promise<import("../types").Semester | null> {
  return call("get_semester");
}

export function setSemester(input: import("../types").SemesterInput, expectedRevision: number | null): Promise<import("../types").SemesterSaveResult> {
  return call("set_semester", { input, expectedRevision });
}

export function setHomeworkDeadline(homeworkId: string, revision: number, deadline: string | null): Promise<import("../types").Homework> {
  return call("set_homework_deadline", { homeworkId, revision, deadline });
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
  homeworkId: PatchField<string>;
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
    homeworkId: patchField(patch, "homeworkId"),
  };
  return call("update_classification", { fileId, patch: encoded });
}

export function createWeek(subjectId: string, weekNumber: number): Promise<OperationResult<Week>> {
  return call("create_week", { subjectId, weekNumber });
}

export function classifyFiles(fileIds: string[]): Promise<OperationResult<InboxFile[]>> {
  return call("classify_files", { fileIds });
}

export function previewRoute(fileId: string, destination: RouteDestination): Promise<RoutePreview> {
  return call("preview_route", { fileId, destination });
}

export function routeFiles(fileIds: string[], destination: RouteDestination | null): Promise<OperationResult<RoutePreview[]>> {
  return call("route_files", { fileIds, destination });
}

export function undoOperation(operationId: string): Promise<OperationResult<null>> {
  return call("undo_operation", { operationId });
}

export function getStyleProfile(): Promise<StyleProfile | null> { return call("get_style_profile"); }
export function getStyleProfileState(): Promise<StyleProfileState> { return call("get_style_profile_state"); }
export function addStyleSource(relativePath: string, kind: StyleSourceKind, language: StyleLanguage): Promise<StyleProfileState> {
  return call("add_style_source", { relativePath, kind, language });
}
export function removeStyleSource(sourceId: string): Promise<StyleProfileState> { return call("remove_style_source", { sourceId }); }
export function refreshStyleSources(): Promise<StyleProfileState> { return call("refresh_style_sources"); }
export function rebuildStyleProfile(language: StyleLanguage): Promise<StyleProfileState> { return call("rebuild_style_profile", { language }); }
export function runStyleCheck(homeworkId: string): Promise<StyleCheck> { return call("run_style_check", { homeworkId }); }
export function confirmStyleCheck(checkId: string, note: string): Promise<StyleCheck> { return call("confirm_style_check", { checkId, note }); }
export function previewTeacherTranscript(input: TeacherTranscriptInput): Promise<TeacherCommentInput[]> { return call("preview_teacher_transcript", { input }); }
export function importTeacherTranscript(homeworkId: string, input: TeacherTranscriptInput): Promise<OperationResult<TeacherComment[]>> {
  return call("import_teacher_transcript", { homeworkId, input });
}
export function removeTeacherComment(commentId: string): Promise<void> { return call("remove_teacher_comment", { commentId }); }
export function buildSubmissionPackage(homeworkId: string): Promise<OperationResult<SubmissionPackage>> { return call("build_submission_package", { homeworkId }); }
export function verifySubmissionPackage(homeworkId: string): Promise<SubmissionPackage | null> { return call("verify_submission_package", { homeworkId }); }
export function setPackageCheck(packageId: string, checkId: PackageCheckId, done: boolean): Promise<SubmissionPackage> { return call("set_package_check", { packageId, checkId, done }); }
export function confirmSubmission(homeworkId: string, confirmation: SubmissionConfirmation): Promise<OperationResult<SubmissionReceipt>> { return call("confirm_submission", { homeworkId, confirmation }); }

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

export function getWorkspaces(): Promise<WorkspaceOverview> {
  return call("get_workspaces");
}

export function previewWorkspaceTransition(request: WorkspaceTransitionRequest): Promise<WorkspaceTransitionPreview | null> {
  return call("preview_workspace_transition", { request });
}

export function confirmWorkspaceTransition(transitionId: string): Promise<WorkspaceActivation> {
  return call("confirm_workspace_transition", { transitionId });
}

export function cancelWorkspaceTransition(transitionId: string): Promise<void> {
  return call("cancel_workspace_transition", { transitionId });
}

export function previewVaultAdoption(): Promise<import("../types").VaultAdoptionPreview> {
  return call("preview_vault_adoption");
}

export function applyVaultAdoption(
  planId: string,
  overrides: import("../types").AdoptionCategoryOverride[],
  dryRun: boolean,
): Promise<import("../types").VaultAdoptionReport> {
  return call("apply_vault_adoption", { planId, overrides, dryRun });
}

export function cancelVaultAdoption(planId: string): Promise<void> {
  return call("cancel_vault_adoption", { planId });
}

export function exportDiagnosticReport(): Promise<import("../types").DiagnosticExport> {
  return call("export_diagnostic_report");
}

export function openObsidianNote(relativePath: string): Promise<void> {
  return call("open_obsidian_note", { relativePath });
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

// ── Этап 8: рабочий цикл ДЗ ──

/** Ревизия ДЗ, которую видел клиент; null — без проверки устаревшего состояния. */
type Revision = number | null;

export function getWorkflow(homeworkId: string): Promise<HomeworkWorkflow> {
  return call("get_workflow", { homeworkId });
}

export function getDashboard(): Promise<Dashboard> {
  return call("get_dashboard");
}

export function getIterations(homeworkId: string): Promise<HomeworkIteration[]> {
  return call("get_iterations", { homeworkId });
}

export function compareVersions(fromVersionId: string, toVersionId: string): Promise<VersionComparison> {
  return call("compare_versions", { fromVersionId, toVersionId });
}

export function getHomeworkCandidates(homeworkId: string): Promise<FileRef[]> {
  return call("get_homework_candidates", { homeworkId });
}

export function createSourceBundle(
  operationId: string,
  homeworkId: string,
  purpose: RunKind,
  versionId: string | null,
  expectedRevision: Revision,
): Promise<OperationResult<SourceBundle>> {
  return call("create_source_bundle", { operationId, homeworkId, purpose, versionId, expectedRevision });
}

export interface StartRunRequest {
  homeworkId: string;
  kind: RunKind;
  versionId: string | null;
  bundleId: string;
  mode: RunMode;
}

export function startWorkflowRun(
  operationId: string,
  request: StartRunRequest,
  expectedRevision: Revision,
): Promise<OperationResult<ReviewRun>> {
  return call("start_workflow_run", { operationId, request, expectedRevision });
}

export function cancelWorkflowRun(operationId: string, runId: string, expectedRevision: Revision): Promise<OperationResult<ReviewRun>> {
  return call("cancel_workflow_run", { operationId, runId, expectedRevision });
}

export function pinWorkflowRun(operationId: string, runId: string, expectedRevision: Revision): Promise<OperationResult<ReviewRun>> {
  return call("pin_workflow_run", { operationId, runId, expectedRevision });
}

export function saveManualAnalysis(
  operationId: string,
  runId: string,
  input: ManualAnalysisInput,
  expectedRevision: Revision,
): Promise<OperationResult<ReviewRun>> {
  return call("save_manual_analysis", { operationId, runId, input: { summary: input.summary, promptText: input.promptText }, expectedRevision });
}

/** Ручной результат проверки: необязательные списки передаются явными пустыми массивами. */
export function saveManualReview(
  operationId: string,
  runId: string,
  input: ManualReviewInput,
  expectedRevision: Revision,
): Promise<OperationResult<ReviewRun>> {
  const finding = (item: NonNullable<ManualReviewInput["critical"]>[number]) => ({
    criterion: item.criterion,
    text: item.text,
    location: item.location,
    evidence: item.evidence ?? [],
  });
  return call("save_manual_review", {
    operationId,
    runId,
    input: {
      verdict: input.verdict,
      summary: input.summary,
      correctness: input.correctness,
      taskFit: input.taskFit,
      courseGrounding: input.courseGrounding,
      teacher: input.teacher,
      styleScore: input.styleScore,
      styleReasons: input.styleReasons ?? [],
      critical: (input.critical ?? []).map(finding),
      remarks: (input.remarks ?? []).map(finding),
      unsupportedClaims: input.unsupportedClaims ?? [],
      missing: input.missing ?? [],
    },
    expectedRevision,
  });
}

export function createRevisionPrompt(
  operationId: string,
  reviewRunId: string,
  text: string | null,
  expectedRevision: Revision,
): Promise<OperationResult<RevisionPrompt>> {
  return call("create_revision_prompt", { operationId, reviewRunId, text, expectedRevision });
}

export function updatePromptDraft(operationId: string, promptId: string, text: string, expectedRevision: Revision): Promise<OperationResult<RevisionPrompt>> {
  return call("update_prompt_draft", { operationId, promptId, text, expectedRevision });
}

export function commitPrompt(operationId: string, promptId: string, expectedRevision: Revision): Promise<OperationResult<RevisionPrompt>> {
  return call("commit_prompt", { operationId, promptId, expectedRevision });
}

export function handOffPrompt(operationId: string, promptId: string, expectedRevision: Revision): Promise<OperationResult<RevisionPrompt>> {
  return call("hand_off_prompt", { operationId, promptId, expectedRevision });
}

export interface VersionInputIpc {
  fileId: string;
  attachmentFileIds: string[];
  author: VersionAuthor | null;
  promptId: string | null;
  note: string | null;
}

export function createHomeworkVersion(
  operationId: string,
  homeworkId: string,
  input: VersionInputIpc,
  expectedRevision: Revision,
): Promise<OperationResult<HomeworkVersion>> {
  return call("create_homework_version", { operationId, homeworkId, input, expectedRevision });
}

export function selectHomeworkVersion(operationId: string, versionId: string, expectedRevision: Revision): Promise<OperationResult<Homework>> {
  return call("select_homework_version", { operationId, versionId, expectedRevision });
}

export function addTeacherComments(
  operationId: string,
  homeworkId: string,
  comments: TeacherCommentInput[],
  expectedRevision: Revision,
): Promise<OperationResult<TeacherComment[]>> {
  return call("add_teacher_comments", {
    operationId,
    homeworkId,
    comments: comments.map((comment) => ({
      source: comment.source,
      date: comment.date,
      question: comment.question,
      answer: comment.answer,
      authority: comment.authority ?? "unknown",
      author: comment.author ?? "",
      context: comment.context ?? "",
      confirmedOfficial: comment.confirmedOfficial ?? false,
    })),
    expectedRevision,
  });
}

export function addTeacherRequirement(
  operationId: string,
  homeworkId: string,
  input: TeacherRequirementInput,
  expectedRevision: Revision,
): Promise<OperationResult<TeacherRequirement>> {
  return call("add_teacher_requirement", {
    operationId,
    homeworkId,
    input: { commentId: input.commentId, text: input.text, origin: input.origin, status: input.status, note: input.note ?? null },
    expectedRevision,
  });
}

export function setCommentAuthority(
  operationId: string,
  commentId: string,
  input: CommentAuthorityInput,
  expectedRevision: Revision,
): Promise<OperationResult<TeacherComment[]>> {
  return call("set_comment_authority", {
    operationId,
    commentId,
    input: { authority: input.authority, author: input.author, context: input.context, confirmedOfficial: input.confirmedOfficial },
    expectedRevision,
  });
}

export function withdrawTeacherRequirement(operationId: string, requirementId: string, expectedRevision: Revision): Promise<OperationResult<TeacherRequirement>> {
  return call("withdraw_teacher_requirement", { operationId, requirementId, expectedRevision });
}

export function resolveTeacherRequirement(
  operationId: string,
  requirementId: string,
  status: "done" | "not_done" | "na",
  note: string,
  expectedRevision: Revision,
): Promise<OperationResult<TeacherRequirement>> {
  return call("resolve_teacher_requirement", { operationId, requirementId, status, note, expectedRevision });
}

export function confirmStyleDivergence(operationId: string, reviewRunId: string, expectedRevision: Revision): Promise<OperationResult<StyleConfirmation>> {
  return call("confirm_style_divergence", { operationId, reviewRunId, expectedRevision });
}

export function markReadyToSubmit(
  operationId: string,
  homeworkId: string,
  noTeacherComments: boolean,
  expectedRevision: Revision,
): Promise<OperationResult<Homework>> {
  return call("mark_ready_to_submit", { operationId, homeworkId, noTeacherComments, expectedRevision });
}

export function transitionWorkflow(
  operationId: string,
  homeworkId: string,
  to: HomeworkWorkflowStage,
  reason: string | null,
  expectedRevision: Revision,
): Promise<OperationResult<Homework>> {
  return call("transition_workflow", { operationId, homeworkId, to, reason, expectedRevision });
}

// ───────────── Этап 9: провайдеры и задания передачи ─────────────

export function getProviderStatuses(): Promise<ProviderStatus[]> {
  return call("get_provider_statuses");
}

export function checkProvider(provider: ProviderId): Promise<ProviderStatus> {
  return call("check_provider", { provider });
}

export function runAiIsolationSelfTest(): Promise<IsolationReport> {
  return call("run_ai_isolation_self_test");
}

export function connectProvider(provider: ProviderId): Promise<ProviderStatus> {
  return call("connect_provider", { provider });
}

export function disconnectProvider(provider: ProviderId): Promise<ProviderStatus> {
  return call("disconnect_provider", { provider });
}

export function openProviderApp(provider: ProviderId): Promise<void> {
  return call("open_provider_app", { provider });
}

export function listAiJobs(homeworkId: string): Promise<AiJob[]> {
  return call("list_ai_jobs", { homeworkId });
}

export function getAiJob(jobId: string): Promise<AiJob> {
  return call("get_ai_job", { jobId });
}

export function prepareAiJob(runId: string, task: AiTask, transport: TransportPreference): Promise<AiJob> {
  return call("prepare_ai_job", { runId, task, transport });
}

export function sendAiJob(jobId: string): Promise<AiJob> {
  return call("send_ai_job", { jobId });
}

export function cancelAiJob(jobId: string): Promise<boolean> {
  return call("cancel_ai_job", { jobId });
}

export function submitAiResponse(jobId: string, text: string): Promise<AiJob> {
  return call("submit_ai_response", { jobId, text });
}

export function importAiResponseFile(jobId: string): Promise<AiJob | null> {
  return call("import_ai_response_file", { jobId });
}

export function discardAiJob(jobId: string): Promise<AiJob> {
  return call("discard_ai_job", { jobId });
}

export function confirmAiResult(
  operationId: string,
  jobId: string,
  supportingJobId: string | null,
  input: AiConfirmInput,
  expectedRevision: Revision,
): Promise<OperationResult<AiConfirmation>> {
  return call("confirm_ai_result", { operationId, jobId, supportingJobId, input: confirmInputIpc(input), expectedRevision });
}

/** Точная форма подтверждения: необязательные поля ручного ввода заполняются явно. */
function confirmInputIpc(input: AiConfirmInput): AiConfirmInput {
  if (input.kind === "analysis") return { kind: "analysis", value: { summary: input.value.summary, promptText: input.value.promptText } };
  if (input.kind === "prompt") return { kind: "prompt", value: { text: input.value.text } };
  const v = input.value;
  return {
    kind: "review",
    value: {
      verdict: v.verdict,
      summary: v.summary,
      correctness: v.correctness,
      taskFit: v.taskFit,
      courseGrounding: v.courseGrounding,
      teacher: v.teacher,
      styleScore: v.styleScore,
      styleReasons: v.styleReasons ?? [],
      critical: (v.critical ?? []).map((f) => ({ criterion: f.criterion, text: f.text, location: f.location, evidence: f.evidence ?? [] })),
      remarks: (v.remarks ?? []).map((f) => ({ criterion: f.criterion, text: f.text, location: f.location, evidence: f.evidence ?? [] })),
      unsupportedClaims: v.unsupportedClaims ?? [],
      missing: v.missing ?? [],
    },
  };
}

/** Преобразует DroppedItem в аргумент IPC; без sourcePath импорт не начинается. */
export function toImportItems(items: DroppedItem[]): ImportItemIpc[] {
  return items.map((item) => ({ sourcePath: item.sourcePath!, origin: item.origin ?? "drop" }));
}

/** Только для unit-тестов низкоуровневого IPC-клиента. */
export function __setInvokeForTests(invoke: Invoke | null | undefined): void {
  testInvoke = invoke;
}
