import { assertNativeSourcePaths, filesFromImportReport, newOperationId } from "../intake";
import type { BridgeEventName, BridgeListener, DesktopBridge, DroppedItem, Unsubscribe } from "../types";
import { BridgeError } from "../types";
import { listenToBridgeEvent, listenToIntakeEvent } from "./events";
import { unsupportedBackend } from "./errors";
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
  checkManagedRoots: typeof ipc.checkManagedRoots;
  chooseManagedRoot: typeof ipc.chooseManagedRoot;
  prepareInboxRoot: typeof ipc.prepareInboxRoot;
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

  getDashboard: DesktopBridge["getDashboard"] = () => unsupportedBackend("Главная");
  createWeek: DesktopBridge["createWeek"] = () => unsupportedBackend("Создание недели");
  classifyFiles: DesktopBridge["classifyFiles"] = () => unsupportedBackend("Классификация файлов");
  previewRoute: DesktopBridge["previewRoute"] = () => unsupportedBackend("Предпросмотр раскладки");
  routeFiles: DesktopBridge["routeFiles"] = () => unsupportedBackend("Раскладка файлов");
  getHomeworkCandidates: DesktopBridge["getHomeworkCandidates"] = () => unsupportedBackend("Кандидаты версий ДЗ");
  createHomeworkVersion: DesktopBridge["createHomeworkVersion"] = () => unsupportedBackend("Создание версии ДЗ");
  markVersionFinal: DesktopBridge["markVersionFinal"] = () => unsupportedBackend("Выбор итоговой версии");
  getStyleProfile: DesktopBridge["getStyleProfile"] = () => unsupportedBackend("Профиль стиля");
  prepareSourceBundle: DesktopBridge["prepareSourceBundle"] = () => unsupportedBackend("Подготовка источников");
  setSourceIncluded: DesktopBridge["setSourceIncluded"] = () => unsupportedBackend("Изменение состава источников");
  runMaterialAnalysis: DesktopBridge["runMaterialAnalysis"] = () => unsupportedBackend("Анализ материалов");
  runVersionReview: DesktopBridge["runVersionReview"] = () => unsupportedBackend("Проверка версии");
  runRevisionPrompt: DesktopBridge["runRevisionPrompt"] = () => unsupportedBackend("Создание промта исправлений");
  createRevisionPrompt: DesktopBridge["createRevisionPrompt"] = () => unsupportedBackend("Сохранение промта исправлений");
  sendPromptToAuthor: DesktopBridge["sendPromptToAuthor"] = () => unsupportedBackend("Передача промта автору");
  updatePromptText: DesktopBridge["updatePromptText"] = () => unsupportedBackend("Редактирование промта");
  getPrompts: DesktopBridge["getPrompts"] = () => unsupportedBackend("Чтение промтов");
  cancelRun: DesktopBridge["cancelRun"] = () => unsupportedBackend("Отмена AI-запуска");
  getReviewRuns: DesktopBridge["getReviewRuns"] = () => unsupportedBackend("История проверок");
  saveRunAsIteration: DesktopBridge["saveRunAsIteration"] = () => unsupportedBackend("Сохранение итерации");
  getIterations: DesktopBridge["getIterations"] = () => unsupportedBackend("История итераций");
  compareVersions: DesktopBridge["compareVersions"] = () => unsupportedBackend("Сравнение версий");
  getTeacherComments: DesktopBridge["getTeacherComments"] = () => unsupportedBackend("Комментарии преподавателя");
  saveTeacherComments: DesktopBridge["saveTeacherComments"] = () => unsupportedBackend("Сохранение комментариев преподавателя");
  importTeacherTranscript: DesktopBridge["importTeacherTranscript"] = () => unsupportedBackend("Импорт переписки преподавателя");
  removeTeacherComment: DesktopBridge["removeTeacherComment"] = () => unsupportedBackend("Удаление комментария преподавателя");
  runTeacherReview: DesktopBridge["runTeacherReview"] = () => unsupportedBackend("Проверка по комментариям преподавателя");
  getTeacherRequirements: DesktopBridge["getTeacherRequirements"] = () => unsupportedBackend("Требования преподавателя");
  resolveRequirement: DesktopBridge["resolveRequirement"] = () => unsupportedBackend("Проверка требования преподавателя");
  getReadiness: DesktopBridge["getReadiness"] = () => unsupportedBackend("Проверка готовности");
  markReadyToSubmit: DesktopBridge["markReadyToSubmit"] = () => unsupportedBackend("Отметка готовности к сдаче");
  buildSubmissionPackage: DesktopBridge["buildSubmissionPackage"] = () => unsupportedBackend("Сборка комплекта на сдачу");
  getSubmissionPackage: DesktopBridge["getSubmissionPackage"] = () => unsupportedBackend("Комплект на сдачу");
  setPackageCheck: DesktopBridge["setPackageCheck"] = () => unsupportedBackend("Чек-лист комплекта");
  confirmSubmission: DesktopBridge["confirmSubmission"] = () => unsupportedBackend("Подтверждение отправки");
  getSubmissionReceipt: DesktopBridge["getSubmissionReceipt"] = () => unsupportedBackend("Снимок отправленной работы");
  openProviderApp: DesktopBridge["openProviderApp"] = () => unsupportedBackend("Открытие приложения провайдера");
  connectProvider: DesktopBridge["connectProvider"] = () => unsupportedBackend("Подключение провайдера");
  disconnectProvider: DesktopBridge["disconnectProvider"] = () => unsupportedBackend("Отключение провайдера");
  getProviderStatus: DesktopBridge["getProviderStatus"] = () => unsupportedBackend("Статус провайдера");
  undoOperation: DesktopBridge["undoOperation"] = () => unsupportedBackend("Отмена операции");
}
