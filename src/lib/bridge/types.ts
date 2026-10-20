/**
 * Контракт между frontend и desktop-backend «Контура».
 *
 * Frontend никогда не обращается к файловой системе, Obsidian, CLI нейросетей
 * или сайту университета напрямую — только через интерфейс DesktopBridge.
 * Все даты — ISO 8601 строки (UTC или с offset). Все размеры — в байтах.
 * Все идентификаторы — непрозрачные строки, frontend их не парсит.
 */

import type {
  HomeworkIteration,
  HomeworkWorkflowStage,
  Readiness,
  ReviewRun,
  RevisionPrompt,
  RunKind,
  SourceBundle,
  StyleProfile,
  SubmissionConfirmation,
  SubmissionPackage,
  SubmissionReceipt,
  TeacherComment,
  TeacherCommentInput,
  TeacherRequirement,
  VersionAuthor,
  VersionComparison,
  WorkflowRole,
  WorkflowSettings,
  PackageCheckId,
} from "./workflow-types";

export * from "./workflow-types";

// ───────────────────────── Базовые сущности ─────────────────────────

export type ISODate = string;
export type ProviderId = "codex" | "claude";

export type ProviderState =
  | "disconnected" // не подключён
  | "connecting" // идёт подключение (вход выполняется во внешнем официальном клиенте)
  | "connected" // подключён
  | "error" // ошибка подключения или выполнения
  | "reauth_required"; // сессия провайдера истекла, нужен повторный вход

export type MaterialCategory =
  | "lecture"
  | "seminar"
  | "material"
  | "homework_task" // условие ДЗ → 03 ДЗ/01 Условие и данные
  | "homework_data" // данные к ДЗ → 03 ДЗ/01 Условие и данные
  | "homework_draft" // черновик → 03 ДЗ/02 Работа
  | "unknown";

export type FileKind = "pdf" | "doc" | "sheet" | "slides" | "image" | "notebook" | "text" | "archive" | "other";

export interface FileRef {
  id: string;
  name: string; // имя с расширением
  kind: FileKind;
  sizeBytes: number;
  modifiedAt: ISODate;
  /** Путь относительно корня хранилища Obsidian. Только для отображения. */
  relativePath: string;
}

export interface Subject {
  id: string;
  name: string;
  shortName: string;
  /** Номер текущей учебной недели для предмета. */
  currentWeek: number;
  /** Сколько недель уже создано в хранилище. */
  weekCount: number;
}

export interface Week {
  id: string;
  subjectId: string;
  number: number;
  startsAt: ISODate;
  endsAt: ISODate;
  topic: string | null;
  lectures: FileRef[];
  seminars: FileRef[];
  materials: FileRef[];
  homeworkId: string | null;
}

// ───────────────────────── Входящие ─────────────────────────

export type InboxStatus = "new" | "classifying" | "classified" | "ambiguous" | "routed";

export interface Classification {
  subjectId: string | null;
  weekNumber: number | null;
  category: MaterialCategory;
  /** 0…1. Ниже rules.lowConfidenceThreshold → файл считается неоднозначным. */
  confidence: number;
  /** Человекочитаемые причины решения («имя содержит „линал“»). */
  reasons: string[];
  /** Источник правки: автоматическая классификация или ручная. */
  source: "auto" | "manual";
}

export interface InboxFile {
  id: string;
  name: string;
  kind: FileKind;
  sizeBytes: number;
  addedAt: ISODate;
  /** Откуда пришёл файл: папка загрузок, перетаскивание, сайт. */
  origin: "downloads" | "drop" | "lms";
  status: InboxStatus;
  classification: Classification | null;
  /** Если в целевой папке уже есть файл с таким содержимым/именем. */
  duplicate: DuplicateInfo | null;
}

export interface DuplicateInfo {
  existingPath: string;
  sameContent: boolean;
  /** Имя, под которым файл будет сохранён без перезаписи. */
  suggestedName: string;
}

export interface RouteDestination {
  subjectId: string;
  weekNumber: number;
  category: MaterialCategory;
}

export interface RoutePreview {
  fileId: string;
  targetPath: string;
  targetName: string;
  createsWeek: boolean;
  duplicate: DuplicateInfo | null;
}

export interface DroppedItem {
  name: string;
  sizeBytes: number;
  /** Абсолютный путь — только в Tauri (из события drag-drop окна). В браузере отсутствует. */
  path?: string;
}

// ───────────────────────── Домашние задания ─────────────────────────

export type HomeworkStage = "task" | "in_progress" | "ready" | "submitted";

export const HOMEWORK_STAGE_ORDER: readonly HomeworkStage[] = ["task", "in_progress", "ready", "submitted"];

export interface VersionSection {
  id: string;
  title: string;
  /** Хэш содержимого раздела — по нему frontend строит сравнение версий. */
  hash: string;
}

export interface HomeworkVersion {
  id: string;
  homeworkId: string;
  /** Порядковый номер, начинается с 1. Метка — «v01», «v02»… */
  number: number;
  label: string;
  fileName: string;
  sizeBytes: number;
  createdAt: ISODate;
  sourceFileId: string;
  note: string | null;
  sections: VersionSection[];
  author: VersionAuthor;
  /** Промт, по которому создана версия (null — версия пользователя без промта). */
  promptId: string | null;
  sourceBundleId: string | null;
  /** Все файлы версии (основной + приложения). */
  files: FileRef[];
  /** Версии неизменяемы после создания. */
  frozen: true;
}

export interface Homework {
  id: string;
  subjectId: string;
  weekNumber: number;
  title: string;
  deadline: ISODate;
  stage: HomeworkStage;
  taskFiles: FileRef[];
  versions: HomeworkVersion[];
  finalVersionId: string | null;
  /** Имя итогового файла в 03 На сдачу, напр. «Гончарова_Доминика_Неделя7 ДЗ.xlsx». */
  finalFileName: string | null;
  submittedAt: ISODate | null;
  folderPath: string;
  /** Детальный этап рабочего цикла. stage — производная «папочная» стадия. */
  workflowStage: HomeworkWorkflowStage;
  /** Ссылка на страницу задания в системе вуза. */
  assignmentUrl: string | null;
}

// ───────────────────────── Провайдеры ─────────────────────────

export interface ProviderStatus {
  provider: ProviderId;
  state: ProviderState;
  /** Отображаемое имя аккаунта/клиента. Никогда не токен. */
  accountLabel: string | null;
  lastCheckedAt: ISODate;
  errorMessage: string | null;
  /** Через что backend работает с провайдером. «mock» — демонстрационный адаптер. */
  adapter: "cli" | "desktop_app" | "mock";
  /** Роли, назначенные провайдеру в настройках рабочего цикла. */
  roles: WorkflowRole[];
}

/** Синоним для контракта рабочего цикла. */
export type ProviderConnection = ProviderStatus;

// ───────────────────────── Автоматизация ─────────────────────────

export interface SubjectRule {
  subjectId: string;
  keywords: string[];
}

export interface AutomationRules {
  inboxPath: string;
  downloadsWatchPath: string;
  obsidianVaultPath: string;
  studentName: { lastName: string; firstName: string };
  /** Токены: {Фамилия} {Имя} {Неделя} {Предмет} {Версия}. */
  finalNamePattern: string;
  versionPattern: string;
  weekDetection: "calendar" | "filename_then_calendar";
  semesterStart: ISODate;
  subjectRules: SubjectRule[];
  lowConfidenceThreshold: number;
  backupBeforeChange: boolean;
  autoRouteHighConfidence: boolean;
  confirm: {
    lowConfidenceRoute: boolean;
    markFinal: boolean;
    /** Всегда true. Отправка ДЗ никогда не автоматизируется. */
    markSubmitted: true;
    /** Всегда false. Перезапись файлов запрещена контрактом. */
    allowOverwrite: false;
  };
  workflow: WorkflowSettings;
}

// ───────────────────────── Операции и журнал ─────────────────────────

export type OperationKind =
  | "classify"
  | "route"
  | "create_week"
  | "create_version"
  | "mark_final"
  | "mark_submitted"
  | "prepare_sources"
  | "ai_run"
  | "revision_prompt"
  | "send_prompt"
  | "save_teacher_comments"
  | "build_package"
  | "mark_ready"
  | "confirm_submission"
  | "update_rules"
  | "undo";

export type OperationStatus = "running" | "completed" | "failed" | "undone";

export interface Operation {
  id: string;
  kind: OperationKind;
  title: string;
  detail: string | null;
  status: OperationStatus;
  startedAt: ISODate;
  finishedAt: ISODate | null;
  undoable: boolean;
  /** Пути, которые затронула операция (для журнала). */
  paths: string[];
  backupPath: string | null;
  /** ДЗ, к которому относится операция (для запрета отмены после отправки). */
  homeworkId: string | null;
}

export interface OperationResult<T> {
  operation: Operation;
  value: T;
}

// ───────────────────────── Главная ─────────────────────────

export interface Deadline {
  homeworkId: string;
  subjectId: string;
  title: string;
  deadline: ISODate;
  stage: HomeworkStage;
}

export interface Dashboard {
  today: ISODate;
  academicWeek: { number: number; startsAt: ISODate; endsAt: ISODate };
  inbox: { total: number; ambiguous: number; preview: InboxFile[] };
  activeHomework: Homework[];
  deadlines: Deadline[];
  recentFiles: FileRef[];
  automation: { state: "active" | "paused" | "error"; watchedPath: string; lastRunAt: ISODate; pending: number };
  providers: ProviderStatus[];
  weekProgress: { done: number; total: number; items: { label: string; done: boolean }[] };
}

// ───────────────────────── События ─────────────────────────

export interface BridgeEvents {
  "inbox:file-added": { files: InboxFile[] };
  "inbox:classification-updated": { fileId: string; classification: Classification; status: InboxStatus };
  "operation:progress": { operationId: string; kind: OperationKind; progress: number; message: string };
  "operation:completed": { operation: Operation };
  "operation:failed": { operation: Operation; error: BridgeErrorPayload };
  "provider:status-changed": { status: ProviderStatus };
  "homework:version-created": { homeworkId: string; version: HomeworkVersion };
  "workflow:stage-changed": { homeworkId: string; from: HomeworkWorkflowStage; to: HomeworkWorkflowStage; reason: string };
  "review:progress": { runId: string; homeworkId: string; progress: number; step: string };
  "review:completed": { run: ReviewRun };
  "review:failed": { run: ReviewRun; error: BridgeErrorPayload };
  "submission:confirmed": { receipt: SubmissionReceipt };
}

export type BridgeEventName = keyof BridgeEvents;
export type BridgeListener<E extends BridgeEventName> = (payload: BridgeEvents[E]) => void;
export type Unsubscribe = () => void;

// ───────────────────────── Ошибки ─────────────────────────

export type BridgeErrorCode =
  | "not_found"
  | "conflict" // например, версия уже итоговая
  | "duplicate" // файл-дубль; перезапись запрещена
  | "provider_unavailable"
  | "reauth_required"
  | "confirmation_required"
  | "io_error"
  | "not_undoable"
  | "immutable" // отправленный снимок и созданные версии менять нельзя
  | "blocked" // не выполнены обязательные проверки
  | "cancelled"
  | "unknown";

export interface BridgeErrorPayload {
  code: BridgeErrorCode;
  message: string;
}

export class BridgeError extends Error implements BridgeErrorPayload {
  readonly code: BridgeErrorCode;
  constructor(code: BridgeErrorCode, message: string) {
    super(message);
    this.code = code;
    this.name = "BridgeError";
  }
}

// ───────────────────────── Интерфейс моста ─────────────────────────

export interface DesktopBridge {
  readonly kind: "mock" | "tauri";

  getDashboard(): Promise<Dashboard>;
  getSubjects(): Promise<Subject[]>;
  getWeeks(subjectId: string): Promise<Week[]>;
  getWeek(subjectId: string, weekNumber: number): Promise<Week | null>;
  createWeek(subjectId: string, weekNumber: number): Promise<OperationResult<Week>>;

  getInboxFiles(): Promise<InboxFile[]>;
  importDroppedFiles(items: DroppedItem[]): Promise<InboxFile[]>;
  classifyFiles(fileIds: string[]): Promise<OperationResult<InboxFile[]>>;
  updateClassification(fileId: string, patch: Partial<Pick<Classification, "subjectId" | "weekNumber" | "category">>): Promise<InboxFile>;
  previewRoute(fileId: string, destination: RouteDestination): Promise<RoutePreview>;
  /** destination = null → использовать классификацию каждого файла. */
  routeFiles(fileIds: string[], destination: RouteDestination | null): Promise<OperationResult<RoutePreview[]>>;

  getHomeworkList(): Promise<Homework[]>;
  getHomework(homeworkId: string): Promise<Homework | null>;
  getHomeworkCandidates(homeworkId: string): Promise<FileRef[]>;
  createHomeworkVersion(homeworkId: string, fileId: string, meta?: { author?: VersionAuthor; promptId?: string | null }): Promise<OperationResult<HomeworkVersion>>;
  markVersionFinal(versionId: string): Promise<OperationResult<Homework>>;

  // ── Рабочий цикл ДЗ ──
  getStyleProfile(): Promise<StyleProfile | null>;
  /** Собирает набор источников для запуска. Пустые обязательные категории → missing. */
  prepareSourceBundle(homeworkId: string, purpose: RunKind, versionId?: string | null): Promise<SourceBundle>;
  setSourceIncluded(bundleId: string, itemId: string, included: boolean): Promise<SourceBundle>;
  /** Аналитик читает материалы и создаёт первичный промт для автора. */
  runMaterialAnalysis(homeworkId: string, bundleId: string): Promise<ReviewRun>;
  /** Проверяющий проверяет версию по всем критериям. */
  runVersionReview(homeworkId: string, versionId: string, bundleId: string): Promise<ReviewRun>;
  /** Отдельный запуск «создать промт исправлений» по версии (учитывает и замечания, и требования преподавателя). */
  runRevisionPrompt(homeworkId: string, versionId: string, bundleId: string): Promise<ReviewRun>;
  /** Сохраняет промт исправлений из результата запуска (обычной проверки или по комментариям). */
  createRevisionPrompt(reviewRunId: string, editedText?: string): Promise<OperationResult<RevisionPrompt>>;
  /** Передаёт промт роли «автор». Backend отдаёт промт адаптеру; результат придёт новой версией. */
  sendPromptToAuthor(promptId: string): Promise<OperationResult<RevisionPrompt>>;
  updatePromptText(promptId: string, text: string): Promise<RevisionPrompt>;
  getPrompts(homeworkId: string): Promise<RevisionPrompt[]>;
  cancelRun(runId: string): Promise<ReviewRun>;
  getReviewRuns(homeworkId: string): Promise<ReviewRun[]>;
  /** Пользователь закрепляет результат запуска в истории итераций. */
  saveRunAsIteration(runId: string): Promise<ReviewRun>;
  getIterations(homeworkId: string): Promise<HomeworkIteration[]>;
  compareVersions(fromVersionId: string, toVersionId: string): Promise<VersionComparison>;

  getTeacherComments(homeworkId: string): Promise<TeacherComment[]>;
  saveTeacherComments(homeworkId: string, comments: TeacherCommentInput[]): Promise<OperationResult<TeacherComment[]>>;
  /** Разбирает вставленную переписку или файл (md/txt/pdf/изображение) на пары «вопрос — ответ». */
  importTeacherTranscript(homeworkId: string, input: { text?: string; file?: DroppedItem; source: TeacherComment["source"]; date: ISODate }): Promise<OperationResult<TeacherComment[]>>;
  removeTeacherComment(commentId: string): Promise<void>;
  runTeacherReview(homeworkId: string, versionId: string, bundleId: string): Promise<ReviewRun>;
  getTeacherRequirements(homeworkId: string): Promise<TeacherRequirement[]>;
  /** Пользователь вручную закрывает требование со статусом «требует ручной проверки». */
  resolveRequirement(requirementId: string, status: "done" | "not_done" | "na", note: string): Promise<TeacherRequirement>;

  getReadiness(homeworkId: string): Promise<Readiness>;
  /** Только по действию пользователя и только если все проверки readiness пройдены. */
  markReadyToSubmit(homeworkId: string, options: { noTeacherComments: boolean }): Promise<OperationResult<Homework>>;
  buildSubmissionPackage(homeworkId: string): Promise<OperationResult<SubmissionPackage>>;
  getSubmissionPackage(homeworkId: string): Promise<SubmissionPackage | null>;
  setPackageCheck(packageId: string, checkId: PackageCheckId, done: boolean): Promise<SubmissionPackage>;
  /** Ручное подтверждение отправки. Ничего не загружает в систему вуза. Снимок становится неизменяемым. */
  confirmSubmission(homeworkId: string, confirmation: SubmissionConfirmation): Promise<OperationResult<SubmissionReceipt>>;
  getSubmissionReceipt(homeworkId: string): Promise<SubmissionReceipt | null>;

  /** Показать папку в Finder (backend). */
  revealInFinder(relativePath: string): Promise<void>;
  /** Открыть официальное приложение провайдера (backend). Вход — только там. */
  openProviderApp(provider: ProviderId): Promise<void>;

  connectProvider(provider: ProviderId): Promise<ProviderStatus>;
  disconnectProvider(provider: ProviderId): Promise<ProviderStatus>;
  getProviderStatus(provider: ProviderId): Promise<ProviderStatus>;

  getAutomationRules(): Promise<AutomationRules>;
  updateAutomationRules(rules: AutomationRules): Promise<OperationResult<AutomationRules>>;

  getActivityLog(): Promise<Operation[]>;
  undoOperation(operationId: string): Promise<OperationResult<null>>;

  on<E extends BridgeEventName>(event: E, listener: BridgeListener<E>): Unsubscribe;
}
