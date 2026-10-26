import { assertNativeSourcePaths, filesFromImportReport, newOperationId } from "../intake";
import type {
  BridgeEventName,
  BridgeListener,
  DesktopBridge,
  DroppedItem,
  HomeworkWorkflow,
  ProviderStatus,
  RunKind,
  RunMode,
  Unsubscribe,
  WorkflowCommandOptions,
} from "../types";
import { BridgeError } from "../types";
import { listenToBridgeEvent, listenToIntakeEvent } from "./events";
import * as ipc from "./ipc";

export interface TauriIpcClient {
  getSubjects: typeof ipc.getSubjects;
  getWeeks: typeof ipc.getWeeks;
  getWeek: typeof ipc.getWeek;
  getInboxFiles: typeof ipc.getInboxFiles;
  getHomeworkList: typeof ipc.getHomeworkList;
  getHomework: typeof ipc.getHomework;
  getAutomationRules: typeof ipc.getAutomationRules;
  getActivityLog: typeof ipc.getActivityLog;
  updateAutomationRules: typeof ipc.updateAutomationRules;
  updateClassification: typeof ipc.updateClassification;
  createWeek: typeof ipc.createWeek;
  classifyFiles: typeof ipc.classifyFiles;
  previewRoute: typeof ipc.previewRoute;
  routeFiles: typeof ipc.routeFiles;
  undoOperation: typeof ipc.undoOperation;
  getStyleProfile: typeof ipc.getStyleProfile;
  importTeacherTranscript: typeof ipc.importTeacherTranscript;
  previewTeacherTranscript: typeof ipc.previewTeacherTranscript;
  removeTeacherComment: typeof ipc.removeTeacherComment;
  buildSubmissionPackage: typeof ipc.buildSubmissionPackage;
  setPackageCheck: typeof ipc.setPackageCheck;
  confirmSubmission: typeof ipc.confirmSubmission;
  checkManagedRoots: typeof ipc.checkManagedRoots;
  chooseManagedRoot: typeof ipc.chooseManagedRoot;
  prepareInboxRoot: typeof ipc.prepareInboxRoot;
  createDemoWorkspace: typeof ipc.createDemoWorkspace;
  pickImportFiles: typeof ipc.pickImportFiles;
  importInboxFiles: typeof ipc.importInboxFiles;
  getOperationResult: typeof ipc.getOperationResult;
  openInboxFile: typeof ipc.openInboxFile;
  revealInboxFile: typeof ipc.revealInboxFile;
  revealInFinder: typeof ipc.revealInFinder;
  getDownloadsWatcher: typeof ipc.getDownloadsWatcher;
  setDownloadsWatcher: typeof ipc.setDownloadsWatcher;
  dismissDownloadCandidate: typeof ipc.dismissDownloadCandidate;
  pickCuExport: typeof ipc.pickCuExport;
  previewCuExport: typeof ipc.previewCuExport;
  importCuExport: typeof ipc.importCuExport;
  setSubjectCourseUrl: typeof ipc.setSubjectCourseUrl;
  setHomeworkAssignmentUrl: typeof ipc.setHomeworkAssignmentUrl;
  openCourse: typeof ipc.openCourse;
  openAssignment: typeof ipc.openAssignment;
  listMaterialExtractions: typeof ipc.listMaterialExtractions;
  getMaterialExtraction: typeof ipc.getMaterialExtraction;
  analyzeMaterial: typeof ipc.analyzeMaterial;
  analyzeMaterials: typeof ipc.analyzeMaterials;
  cancelExtraction: typeof ipc.cancelExtraction;
  clearExtractionCache: typeof ipc.clearExtractionCache;
  getExtractionSettings: typeof ipc.getExtractionSettings;
  setExtractionSettings: typeof ipc.setExtractionSettings;
  getCorpusSelection: typeof ipc.getCorpusSelection;
  setCorpusSelection: typeof ipc.setCorpusSelection;
  previewCorpus: typeof ipc.previewCorpus;
  getWorkflow: typeof ipc.getWorkflow;
  getDashboard: typeof ipc.getDashboard;
  getIterations: typeof ipc.getIterations;
  compareVersions: typeof ipc.compareVersions;
  getHomeworkCandidates: typeof ipc.getHomeworkCandidates;
  createSourceBundle: typeof ipc.createSourceBundle;
  startWorkflowRun: typeof ipc.startWorkflowRun;
  cancelWorkflowRun: typeof ipc.cancelWorkflowRun;
  pinWorkflowRun: typeof ipc.pinWorkflowRun;
  saveManualAnalysis: typeof ipc.saveManualAnalysis;
  saveManualReview: typeof ipc.saveManualReview;
  createRevisionPrompt: typeof ipc.createRevisionPrompt;
  updatePromptDraft: typeof ipc.updatePromptDraft;
  commitPrompt: typeof ipc.commitPrompt;
  handOffPrompt: typeof ipc.handOffPrompt;
  createHomeworkVersion: typeof ipc.createHomeworkVersion;
  selectHomeworkVersion: typeof ipc.selectHomeworkVersion;
  addTeacherComments: typeof ipc.addTeacherComments;
  addTeacherRequirement: typeof ipc.addTeacherRequirement;
  resolveTeacherRequirement: typeof ipc.resolveTeacherRequirement;
  confirmStyleDivergence: typeof ipc.confirmStyleDivergence;
  markReadyToSubmit: typeof ipc.markReadyToSubmit;
  transitionWorkflow: typeof ipc.transitionWorkflow;
  getProviderStatuses: typeof ipc.getProviderStatuses;
  checkProvider: typeof ipc.checkProvider;
  runAiIsolationSelfTest: typeof ipc.runAiIsolationSelfTest;
  connectProvider: typeof ipc.connectProvider;
  disconnectProvider: typeof ipc.disconnectProvider;
  openProviderApp: typeof ipc.openProviderApp;
  listAiJobs: typeof ipc.listAiJobs;
  getAiJob: typeof ipc.getAiJob;
  prepareAiJob: typeof ipc.prepareAiJob;
  sendAiJob: typeof ipc.sendAiJob;
  cancelAiJob: typeof ipc.cancelAiJob;
  submitAiResponse: typeof ipc.submitAiResponse;
  importAiResponseFile: typeof ipc.importAiResponseFile;
  discardAiJob: typeof ipc.discardAiJob;
  confirmAiResult: typeof ipc.confirmAiResult;
}

/** Опции команды → аргументы IPC: новый UUID v4, если клиент не задал свой. */
function command(options?: WorkflowCommandOptions): { operationId: string; revision: number | null } {
  return { operationId: options?.operationId ?? newOperationId(), revision: options?.expectedRevision ?? null };
}

type EventSubscriber = typeof listenToBridgeEvent;
type IntakeSubscriber = typeof listenToIntakeEvent;

async function nullWhenNotFound<T>(read: () => Promise<T>): Promise<T | null> {
  try {
    return await read();
  } catch (error) {
    if (error instanceof BridgeError && error.code === "not_found") return null;
    throw error;
  }
}

export class TauriDesktopBridge implements DesktopBridge {
  readonly kind = "tauri" as const;

  constructor(
    private readonly ipcClient: TauriIpcClient = ipc,
    private readonly subscribe: EventSubscriber = listenToBridgeEvent,
    private readonly subscribeIntake: IntakeSubscriber = listenToIntakeEvent,
  ) {}

  getSubjects: DesktopBridge["getSubjects"] = () => this.ipcClient.getSubjects();
  getWeeks: DesktopBridge["getWeeks"] = (subjectId) => this.ipcClient.getWeeks(subjectId);
  getWeek: DesktopBridge["getWeek"] = (subjectId, weekNumber) =>
    nullWhenNotFound(() => this.ipcClient.getWeek(subjectId, weekNumber));
  getInboxFiles: DesktopBridge["getInboxFiles"] = () => this.ipcClient.getInboxFiles();
  getHomeworkList: DesktopBridge["getHomeworkList"] = () => this.ipcClient.getHomeworkList();
  getHomework: DesktopBridge["getHomework"] = (homeworkId) =>
    nullWhenNotFound(() => this.ipcClient.getHomework(homeworkId));
  getAutomationRules: DesktopBridge["getAutomationRules"] = () => this.ipcClient.getAutomationRules();
  getActivityLog: DesktopBridge["getActivityLog"] = () => this.ipcClient.getActivityLog();
  updateAutomationRules: DesktopBridge["updateAutomationRules"] = (rules) =>
    this.ipcClient.updateAutomationRules(rules);
  updateClassification: DesktopBridge["updateClassification"] = (fileId, patch) =>
    this.ipcClient.updateClassification(fileId, patch);

  on<E extends BridgeEventName>(event: E, listener: BridgeListener<E>): Unsubscribe {
    return this.subscribe(event, listener);
  }

  // ── Native intake и импорт во «Входящие» ──

  pickFiles: DesktopBridge["pickFiles"] = async () => {
    const items = await this.ipcClient.pickImportFiles();
    return items.map((item): DroppedItem => ({ ...item, origin: "picker" }));
  };

  onNativeFileDrop: DesktopBridge["onNativeFileDrop"] = (listener) =>
    this.subscribeIntake("intake:native-drop", ({ items }) => {
      if (items.length > 0) listener(items.map((item): DroppedItem => ({ ...item, origin: "drop" })));
    });

  onNativeDragState: DesktopBridge["onNativeDragState"] = (listener) =>
    this.subscribeIntake("intake:drag-state", ({ state }) => listener(state === "enter"));

  importFiles: DesktopBridge["importFiles"] = async (items, operationId = newOperationId()) => {
    assertNativeSourcePaths(items);
    return this.ipcClient.importInboxFiles(operationId, ipc.toImportItems(items));
  };

  importDroppedFiles: DesktopBridge["importDroppedFiles"] = async (items) => {
    const result = await this.importFiles(items);
    return filesFromImportReport(result.value);
  };

  getOperationResult: DesktopBridge["getOperationResult"] = (operationId) =>
    nullWhenNotFound(() => this.ipcClient.getOperationResult(operationId));
  openInboxFile: DesktopBridge["openInboxFile"] = (fileId) => this.ipcClient.openInboxFile(fileId);
  revealInboxFile: DesktopBridge["revealInboxFile"] = (fileId) => this.ipcClient.revealInboxFile(fileId);
  revealInFinder: DesktopBridge["revealInFinder"] = (relativePath) => this.ipcClient.revealInFinder(relativePath);

  getManagedRoots: DesktopBridge["getManagedRoots"] = () => this.ipcClient.checkManagedRoots();
  chooseManagedRoot: DesktopBridge["chooseManagedRoot"] = (kind) => this.ipcClient.chooseManagedRoot(kind);
  prepareInboxRoot: DesktopBridge["prepareInboxRoot"] = () => this.ipcClient.prepareInboxRoot();
  createDemoWorkspace: DesktopBridge["createDemoWorkspace"] = () => this.ipcClient.createDemoWorkspace();

  // ── Загрузки и экспорт ЦУ ──

  getDownloadsWatcher: DesktopBridge["getDownloadsWatcher"] = () => this.ipcClient.getDownloadsWatcher();
  setDownloadsWatcher: DesktopBridge["setDownloadsWatcher"] = (enabled) => this.ipcClient.setDownloadsWatcher(enabled);
  dismissDownloadCandidate: DesktopBridge["dismissDownloadCandidate"] = (candidateId) =>
    this.ipcClient.dismissDownloadCandidate(candidateId);
  pickCuExport: DesktopBridge["pickCuExport"] = () => this.ipcClient.pickCuExport();
  previewCuExport: DesktopBridge["previewCuExport"] = (candidateId) => this.ipcClient.previewCuExport(candidateId);
  importCuExport: DesktopBridge["importCuExport"] = (request, operationId = newOperationId()) =>
    this.ipcClient.importCuExport(operationId, request);
  setSubjectCourseUrl: DesktopBridge["setSubjectCourseUrl"] = (subjectId, url) =>
    this.ipcClient.setSubjectCourseUrl(subjectId, url);
  setHomeworkAssignmentUrl: DesktopBridge["setHomeworkAssignmentUrl"] = (homeworkId, url) =>
    this.ipcClient.setHomeworkAssignmentUrl(homeworkId, url);
  openCourse: DesktopBridge["openCourse"] = (target) => this.ipcClient.openCourse(target);
  openAssignment: DesktopBridge["openAssignment"] = (target) => this.ipcClient.openAssignment(target);

  // ── Извлечение текста (этап 7) ──

  listMaterialExtractions: DesktopBridge["listMaterialExtractions"] = () => this.ipcClient.listMaterialExtractions();
  getMaterialExtraction: DesktopBridge["getMaterialExtraction"] = (material) =>
    nullWhenNotFound(() => this.ipcClient.getMaterialExtraction(material));
  analyzeMaterial: DesktopBridge["analyzeMaterial"] = (material, options) =>
    this.ipcClient.analyzeMaterial(material, options?.force ?? false);
  analyzeMaterials: DesktopBridge["analyzeMaterials"] = (materials, options) =>
    this.ipcClient.analyzeMaterials(options?.operationId ?? newOperationId(), materials, options?.force ?? false);
  cancelExtraction: DesktopBridge["cancelExtraction"] = (operationId) => this.ipcClient.cancelExtraction(operationId);
  clearExtractionCache: DesktopBridge["clearExtractionCache"] = () => this.ipcClient.clearExtractionCache();
  getExtractionSettings: DesktopBridge["getExtractionSettings"] = () => this.ipcClient.getExtractionSettings();
  setExtractionSettings: DesktopBridge["setExtractionSettings"] = (patch) =>
    this.ipcClient.setExtractionSettings(patch.ocrEnabled);
  getCorpusSelection: DesktopBridge["getCorpusSelection"] = (homeworkId) => this.ipcClient.getCorpusSelection(homeworkId);
  setCorpusSelection: DesktopBridge["setCorpusSelection"] = (homeworkId, materials) =>
    this.ipcClient.setCorpusSelection(homeworkId, materials);
  previewCorpus: DesktopBridge["previewCorpus"] = (homeworkId) => this.ipcClient.previewCorpus(homeworkId);

  // ── Рабочий цикл ДЗ (этап 8). Без mock fallback: ошибки backend пробрасываются как есть. ──

  getWorkflow: DesktopBridge["getWorkflow"] = (homeworkId) => nullWhenNotFound(() => this.ipcClient.getWorkflow(homeworkId));

  /** Чтение части агрегата: отсутствующее ДЗ — not_found (списки не превращаются в пустые). */
  private async workflowOf(homeworkId: string): Promise<HomeworkWorkflow> {
    return this.ipcClient.getWorkflow(homeworkId);
  }

  private async startRun(homeworkId: string, kind: RunKind, versionId: string | null, bundleId: string, options?: WorkflowCommandOptions & { mode?: RunMode }) {
    const { operationId, revision } = command(options);
    const result = await this.ipcClient.startWorkflowRun(operationId, { homeworkId, kind, versionId, bundleId, mode: options?.mode ?? "provider" }, revision);
    return result.value;
  }

  getDashboard: DesktopBridge["getDashboard"] = () => this.ipcClient.getDashboard();
  getHomeworkCandidates: DesktopBridge["getHomeworkCandidates"] = (homeworkId) => this.ipcClient.getHomeworkCandidates(homeworkId);
  createHomeworkVersion: DesktopBridge["createHomeworkVersion"] = (homeworkId, fileId, meta) => {
    const { operationId, revision } = command(meta);
    return this.ipcClient.createHomeworkVersion(
      operationId,
      homeworkId,
      { fileId, attachmentFileIds: meta?.attachmentFileIds ?? [], author: meta?.author ?? null, promptId: meta?.promptId ?? null, note: meta?.note ?? null },
      revision,
    );
  };
  markVersionFinal: DesktopBridge["markVersionFinal"] = (versionId, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.selectHomeworkVersion(operationId, versionId, revision);
  };
  getStyleProfile: DesktopBridge["getStyleProfile"] = () => this.ipcClient.getStyleProfile();
  prepareSourceBundle: DesktopBridge["prepareSourceBundle"] = async (homeworkId, purpose, versionId = null, options) => {
    const { operationId, revision } = command(options);
    return (await this.ipcClient.createSourceBundle(operationId, homeworkId, purpose, versionId, revision)).value;
  };
  setSourceIncluded: DesktopBridge["setSourceIncluded"] = () =>
    Promise.reject(new BridgeError("immutable", "Набор источников — неизменяемый снимок: измените подборку материалов и создайте новый набор"));
  runMaterialAnalysis: DesktopBridge["runMaterialAnalysis"] = (homeworkId, bundleId, options) => this.startRun(homeworkId, "analyze_materials", null, bundleId, options);
  runVersionReview: DesktopBridge["runVersionReview"] = (homeworkId, versionId, bundleId, options) => this.startRun(homeworkId, "review_version", versionId, bundleId, options);
  runRevisionPrompt: DesktopBridge["runRevisionPrompt"] = (homeworkId, versionId, bundleId, options) => this.startRun(homeworkId, "revision_prompt", versionId, bundleId, options);
  runTeacherReview: DesktopBridge["runTeacherReview"] = (homeworkId, versionId, bundleId, options) => this.startRun(homeworkId, "review_teacher", versionId, bundleId, options);
  saveManualAnalysis: DesktopBridge["saveManualAnalysis"] = (runId, input, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.saveManualAnalysis(operationId, runId, input, revision);
  };
  saveManualReviewResult: DesktopBridge["saveManualReviewResult"] = (runId, input, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.saveManualReview(operationId, runId, input, revision);
  };
  createRevisionPrompt: DesktopBridge["createRevisionPrompt"] = (reviewRunId, editedText, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.createRevisionPrompt(operationId, reviewRunId, editedText ?? null, revision);
  };
  commitPrompt: DesktopBridge["commitPrompt"] = (promptId, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.commitPrompt(operationId, promptId, revision);
  };
  /** Адаптера автора нет: команда фиксирует ручную передачу промта и переводит ДЗ к решению. */
  sendPromptToAuthor: DesktopBridge["sendPromptToAuthor"] = (promptId, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.handOffPrompt(operationId, promptId, revision);
  };
  updatePromptText: DesktopBridge["updatePromptText"] = async (promptId, text, options) => {
    const { operationId, revision } = command(options);
    return (await this.ipcClient.updatePromptDraft(operationId, promptId, text, revision)).value;
  };
  getPrompts: DesktopBridge["getPrompts"] = async (homeworkId) => (await this.workflowOf(homeworkId)).prompts;
  cancelRun: DesktopBridge["cancelRun"] = async (runId, options) => {
    const { operationId, revision } = command(options);
    return (await this.ipcClient.cancelWorkflowRun(operationId, runId, revision)).value;
  };
  getReviewRuns: DesktopBridge["getReviewRuns"] = async (homeworkId) => (await this.workflowOf(homeworkId)).runs;
  saveRunAsIteration: DesktopBridge["saveRunAsIteration"] = async (runId, options) => {
    const { operationId, revision } = command(options);
    return (await this.ipcClient.pinWorkflowRun(operationId, runId, revision)).value;
  };
  getIterations: DesktopBridge["getIterations"] = (homeworkId) => this.ipcClient.getIterations(homeworkId);
  compareVersions: DesktopBridge["compareVersions"] = (fromVersionId, toVersionId) => this.ipcClient.compareVersions(fromVersionId, toVersionId);
  confirmStyleDivergence: DesktopBridge["confirmStyleDivergence"] = (reviewRunId, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.confirmStyleDivergence(operationId, reviewRunId, revision);
  };
  transitionWorkflow: DesktopBridge["transitionWorkflow"] = (homeworkId, to, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.transitionWorkflow(operationId, homeworkId, to, options?.reason ?? null, revision);
  };
  getTeacherComments: DesktopBridge["getTeacherComments"] = async (homeworkId) => (await this.workflowOf(homeworkId)).teacherComments;
  saveTeacherComments: DesktopBridge["saveTeacherComments"] = (homeworkId, comments, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.addTeacherComments(operationId, homeworkId, comments, revision);
  };
  addTeacherRequirement: DesktopBridge["addTeacherRequirement"] = (homeworkId, input, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.addTeacherRequirement(operationId, homeworkId, input, revision);
  };
  importTeacherTranscript: DesktopBridge["importTeacherTranscript"] = (homeworkId, input) => this.ipcClient.importTeacherTranscript(homeworkId, input);
  previewTeacherTranscript: DesktopBridge["previewTeacherTranscript"] = (input) => this.ipcClient.previewTeacherTranscript(input);
  removeTeacherComment: DesktopBridge["removeTeacherComment"] = (commentId) => this.ipcClient.removeTeacherComment(commentId);
  getTeacherRequirements: DesktopBridge["getTeacherRequirements"] = async (homeworkId) => (await this.workflowOf(homeworkId)).teacherRequirements;
  resolveRequirement: DesktopBridge["resolveRequirement"] = async (requirementId, status, note, options) => {
    const { operationId, revision } = command(options);
    return (await this.ipcClient.resolveTeacherRequirement(operationId, requirementId, status, note, revision)).value;
  };
  getReadiness: DesktopBridge["getReadiness"] = async (homeworkId) => (await this.workflowOf(homeworkId)).readiness;
  markReadyToSubmit: DesktopBridge["markReadyToSubmit"] = (homeworkId, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.markReadyToSubmit(operationId, homeworkId, options.noTeacherComments, revision);
  };
  buildSubmissionPackage: DesktopBridge["buildSubmissionPackage"] = (homeworkId) => this.ipcClient.buildSubmissionPackage(homeworkId);
  getSubmissionPackage: DesktopBridge["getSubmissionPackage"] = async (homeworkId) => (await this.workflowOf(homeworkId)).submissionPackage;
  setPackageCheck: DesktopBridge["setPackageCheck"] = (packageId, checkId, done) => this.ipcClient.setPackageCheck(packageId, checkId, done);
  confirmSubmission: DesktopBridge["confirmSubmission"] = (homeworkId, confirmation) => this.ipcClient.confirmSubmission(homeworkId, confirmation);
  getSubmissionReceipt: DesktopBridge["getSubmissionReceipt"] = async (homeworkId) => (await this.workflowOf(homeworkId)).submissionReceipt;

  // ── Реальная раскладка «Входящих» по предметам. ──

  createWeek: DesktopBridge["createWeek"] = (subjectId, weekNumber) => this.ipcClient.createWeek(subjectId, weekNumber);
  classifyFiles: DesktopBridge["classifyFiles"] = (fileIds) => this.ipcClient.classifyFiles(fileIds);
  previewRoute: DesktopBridge["previewRoute"] = (fileId, destination) => this.ipcClient.previewRoute(fileId, destination);
  routeFiles: DesktopBridge["routeFiles"] = (fileIds, destination) => this.ipcClient.routeFiles(fileIds, destination);

  // ── Провайдеры и задания передачи (этап 9). Учётные данные не запрашиваются и не передаются. ──

  getProviderStatuses: DesktopBridge["getProviderStatuses"] = () => this.ipcClient.getProviderStatuses();
  getProviderStatus: DesktopBridge["getProviderStatus"] = async (provider) => {
    const status = (await this.ipcClient.getProviderStatuses()).find((item: ProviderStatus) => item.provider === provider);
    if (!status) throw new BridgeError("not_found", "Провайдер не найден");
    return status;
  };
  checkProvider: DesktopBridge["checkProvider"] = (provider) => this.ipcClient.checkProvider(provider);
  runAiIsolationSelfTest: DesktopBridge["runAiIsolationSelfTest"] = () => this.ipcClient.runAiIsolationSelfTest();
  connectProvider: DesktopBridge["connectProvider"] = (provider) => this.ipcClient.connectProvider(provider);
  disconnectProvider: DesktopBridge["disconnectProvider"] = (provider) => this.ipcClient.disconnectProvider(provider);
  openProviderApp: DesktopBridge["openProviderApp"] = (provider) => this.ipcClient.openProviderApp(provider);
  listAiJobs: DesktopBridge["listAiJobs"] = (homeworkId) => this.ipcClient.listAiJobs(homeworkId);
  getAiJob: DesktopBridge["getAiJob"] = (jobId) => this.ipcClient.getAiJob(jobId);
  prepareAiJob: DesktopBridge["prepareAiJob"] = (runId, task, transport = "auto") => this.ipcClient.prepareAiJob(runId, task, transport);
  sendAiJob: DesktopBridge["sendAiJob"] = (jobId) => this.ipcClient.sendAiJob(jobId);
  cancelAiJob: DesktopBridge["cancelAiJob"] = (jobId) => this.ipcClient.cancelAiJob(jobId);
  submitAiResponse: DesktopBridge["submitAiResponse"] = (jobId, text) => this.ipcClient.submitAiResponse(jobId, text);
  importAiResponseFile: DesktopBridge["importAiResponseFile"] = (jobId) => this.ipcClient.importAiResponseFile(jobId);
  discardAiJob: DesktopBridge["discardAiJob"] = (jobId) => this.ipcClient.discardAiJob(jobId);
  confirmAiResult: DesktopBridge["confirmAiResult"] = (jobId, input, options) => {
    const { operationId, revision } = command(options);
    return this.ipcClient.confirmAiResult(operationId, jobId, options?.supportingJobId ?? null, input, revision);
  };
  undoOperation: DesktopBridge["undoOperation"] = (operationId) => this.ipcClient.undoOperation(operationId);
}
