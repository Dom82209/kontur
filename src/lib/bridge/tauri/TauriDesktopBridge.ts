import type { BridgeEventName, BridgeListener, DesktopBridge, Unsubscribe } from "../types";
import { BridgeError } from "../types";
import { listenToBridgeEvent } from "./events";
import { unsupportedBackend, unsupportedBackendSync } from "./errors";
import * as ipc from "./ipc";

export interface TauriReadIpc {
  getSubjects: typeof ipc.getSubjects;
  getWeeks: typeof ipc.getWeeks;
  getWeek: typeof ipc.getWeek;
  getInboxFiles: typeof ipc.getInboxFiles;
  getHomeworkList: typeof ipc.getHomeworkList;
  getHomework: typeof ipc.getHomework;
  getAutomationRules: typeof ipc.getAutomationRules;
  getActivityLog: typeof ipc.getActivityLog;
}

type EventSubscriber = typeof listenToBridgeEvent;

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
    private readonly readIpc: TauriReadIpc = ipc,
    private readonly subscribe: EventSubscriber = listenToBridgeEvent,
  ) {}

  getSubjects: DesktopBridge["getSubjects"] = () => this.readIpc.getSubjects();
  getWeeks: DesktopBridge["getWeeks"] = (subjectId) => this.readIpc.getWeeks(subjectId);
  getWeek: DesktopBridge["getWeek"] = (subjectId, weekNumber) =>
    nullWhenNotFound(() => this.readIpc.getWeek(subjectId, weekNumber));
  getInboxFiles: DesktopBridge["getInboxFiles"] = () => this.readIpc.getInboxFiles();
  getHomeworkList: DesktopBridge["getHomeworkList"] = () => this.readIpc.getHomeworkList();
  getHomework: DesktopBridge["getHomework"] = (homeworkId) =>
    nullWhenNotFound(() => this.readIpc.getHomework(homeworkId));
  getAutomationRules: DesktopBridge["getAutomationRules"] = () => this.readIpc.getAutomationRules();
  getActivityLog: DesktopBridge["getActivityLog"] = () => this.readIpc.getActivityLog();

  on<E extends BridgeEventName>(event: E, listener: BridgeListener<E>): Unsubscribe {
    return this.subscribe(event, listener);
  }

  getDashboard: DesktopBridge["getDashboard"] = () => unsupportedBackend("Главная");
  createWeek: DesktopBridge["createWeek"] = () => unsupportedBackend("Создание недели");
  pickFiles: DesktopBridge["pickFiles"] = () => unsupportedBackend("Выбор файлов");
  onNativeFileDrop: DesktopBridge["onNativeFileDrop"] = () => unsupportedBackendSync("Native drag-and-drop");
  importDroppedFiles: DesktopBridge["importDroppedFiles"] = () => unsupportedBackend("Импорт файлов");
  classifyFiles: DesktopBridge["classifyFiles"] = () => unsupportedBackend("Классификация файлов");
  updateClassification: DesktopBridge["updateClassification"] = () => unsupportedBackend("Изменение классификации");
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
  revealInFinder: DesktopBridge["revealInFinder"] = () => unsupportedBackend("Показ в Finder");
  openProviderApp: DesktopBridge["openProviderApp"] = () => unsupportedBackend("Открытие приложения провайдера");
  connectProvider: DesktopBridge["connectProvider"] = () => unsupportedBackend("Подключение провайдера");
  disconnectProvider: DesktopBridge["disconnectProvider"] = () => unsupportedBackend("Отключение провайдера");
  getProviderStatus: DesktopBridge["getProviderStatus"] = () => unsupportedBackend("Статус провайдера");
  updateAutomationRules: DesktopBridge["updateAutomationRules"] = () => unsupportedBackend("Сохранение правил автоматизации");
  undoOperation: DesktopBridge["undoOperation"] = () => unsupportedBackend("Отмена операции");
}
