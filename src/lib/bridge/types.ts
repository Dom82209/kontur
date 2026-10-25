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
  HomeworkWorkflow,
  HomeworkWorkflowStage,
  ManualAnalysisInput,
  ManualReviewInput,
  RunMode,
  StyleConfirmation,
  TeacherRequirementInput,
  WorkflowCommandOptions,
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
import type {
  AiConfirmation,
  AiConfirmInput,
  AiJob,
  AiJobStatus,
  AiParseStatus,
  AiTask,
  ProviderAvailability,
  ProviderCli,
  ProviderHandoff,
  ProviderIsolation,
  IsolationReport,
  TransportPreference,
} from "./ai-types";

export * from "./workflow-types";
export * from "./ai-types";

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
  /** Необязательная ссылка на курс в LMS (только https://my.centraluniversity.ru). */
  courseUrl: string | null;
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

export interface ClassificationPatch {
  subjectId?: string | null;
  weekNumber?: number | null;
  /** null очищает категорию до unknown. */
  category?: MaterialCategory | null;
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
  /** Происхождение (этап 6). null — файл импортирован до появления журнала происхождения. */
  provenance: InboxProvenance | null;
}

/** Откуда файл попал во «Входящие»: без абсолютных путей, только имя архива и путь внутри него. */
export interface InboxProvenance {
  source: "manual" | "watcher" | "cu_export";
  via: "picker" | "drop" | "watcher";
  archiveName: string | null;
  entryPath: string | null;
  courseUrl: string | null;
  assignmentUrl: string | null;
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
  /** Абсолютный исходный путь. Обязателен для native Tauri intake, отсутствует у HTML5 File. */
  sourcePath?: string;
  /** Как файл получен: системный выбор или перетаскивание. По умолчанию — drop. */
  origin?: "picker" | "drop";
}

// ───────────────────────── Управляемые каталоги и импорт ─────────────────────────

export type ManagedRootKind = "vault" | "inbox" | "backup" | "downloads";
/** Корень, который пользователь выбирает системным диалогом. «Входящие» живут внутри хранилища. */
export type SelectableRootKind = Exclude<ManagedRootKind, "inbox">;
export type ManagedRootState = "ok" | "not_configured" | "missing" | "unsafe";

export interface ManagedRootStatus {
  kind: ManagedRootKind;
  state: ManagedRootState;
  /** Только для показа: домашний каталог сокращён до «~». В моделях не используется. */
  displayPath: string | null;
  message: string | null;
}

export interface ManagedRoots {
  vault: ManagedRootStatus;
  inbox: ManagedRootStatus;
  backup: ManagedRootStatus;
  downloads: ManagedRootStatus;
  /** Хранилище доступно, импорт во «Входящие» возможен. */
  importReady: boolean;
}

export interface RootSelection {
  /** Пользователь закрыл системный диалог; настройки не изменились. */
  cancelled: boolean;
  roots: ManagedRoots;
}

export type ImportItemReport =
  | { status: "imported"; name: string; file: InboxFile; renamedFrom: string | null; recovered: boolean }
  | { status: "duplicate"; name: string; file: InboxFile; restored: boolean }
  | { status: "failed"; name: string; error: BridgeErrorPayload };

export interface InboxImportReport {
  operationId: string;
  items: ImportItemReport[];
  imported: number;
  duplicates: number;
  failed: number;
  /** Записи экспорта ЦУ, которые не выбраны или не могут быть импортированы. */
  skipped: number;
  /** Повтор той же команды: итог прочитан из журнала, файлы не копировались повторно. */
  replayed: boolean;
}

// ───────────────────────── Загрузки и экспорт ЦУ (этап 6) ─────────────────────────

/** Кандидат на импорт: найден watcher в папке загрузок или выбран кнопкой «Импортировать экспорт ЦУ». */
export interface DownloadCandidate {
  id: string;
  name: string;
  sizeBytes: number;
  detectedAt: ISODate;
  origin: "watcher" | "manual";
  kind: "zip" | "pdf" | "file";
}

export type DownloadsWatcherState = "disabled" | "no_folder" | "watching" | "error";

export interface DownloadsWatcher {
  /** Включён пользователем в Настройках. По умолчанию false. */
  enabled: boolean;
  state: DownloadsWatcherState;
  message: string | null;
  lastScanAt: ISODate | null;
  candidates: DownloadCandidate[];
}

export interface ExportEntryPreview {
  entryId: string;
  /** Путь внутри архива (относительный). */
  path: string;
  name: string;
  /** Имя, под которым файл появится во «Входящих». */
  inboxName: string;
  sizeBytes: number;
  kind: FileKind;
  classification: Classification;
  status: "classified" | "ambiguous";
  /** false — вложенный архив или слишком большой файл: импорт невозможен. */
  selectable: boolean;
  selected: boolean;
  warnings: string[];
}

export interface ExportPreview {
  candidateId: string;
  archiveName: string;
  /** Отпечаток предпросмотра: импорт отклоняется, если файл изменился. */
  archiveSha256: string;
  format: "zip" | "single";
  courseTitle: string | null;
  entries: ExportEntryPreview[];
  ignored: string[];
  warnings: string[];
  totalFiles: number;
  ambiguousFiles: number;
  totalBytes: number;
}

export interface ExportImportEntry {
  entryId: string;
  subjectId: string | null;
  weekNumber: number | null;
  category: MaterialCategory;
  /** Пользователь исправил классификацию в предпросмотре. */
  edited: boolean;
}

export interface ExportImportRequest {
  candidateId: string;
  archiveSha256: string;
  /** Только выбранные записи. */
  entries: ExportImportEntry[];
  courseUrl?: string | null;
  assignmentUrl?: string | null;
}

export type CuImportPhase = "inspect" | "extract" | "import" | "cleanup";

/** Цель команд «Открыть курс» / «Открыть задание». */
export type CourseLinkTarget = { subjectId: string } | { fileId: string };
export type AssignmentLinkTarget = { homeworkId: string } | { fileId: string };

export interface OperationRecord {
  operation: Operation;
  importReport: InboxImportReport | null;
}

// ───────────────────────── Извлечение текста (этап 7) ─────────────────────────

/** Материал для извлечения: файл «Входящих» или файл недели/ДЗ. Идентификаторы непрозрачны. */
export interface MaterialRef {
  kind: "inbox" | "file";
  id: string;
}

export type ExtractionFormat = "pdf" | "docx" | "xlsx" | "pptx" | "text" | "markdown" | "csv" | "tsv" | "notebook" | "image" | "unknown";

/**
 * Итог извлечения. complete — всё поддерживаемое извлечено; partial — часть пропущена
 * (см. warnings); needs_ocr — текстового слоя нет (скан/изображение), метаданные сохранены;
 * rejected — отклонён по соображениям безопасности. timeout/failed не кэшируются.
 */
export type ExtractionStatus =
  | "complete"
  | "partial"
  | "needs_ocr"
  | "unsupported"
  | "encrypted"
  | "corrupt"
  | "rejected"
  | "too_large"
  | "timeout"
  | "failed";

export type ExtractionWarningCode =
  | "text_truncated"
  | "blocks_truncated"
  | "pages_truncated"
  | "page_without_text"
  | "page_unreadable"
  | "sheets_truncated"
  | "cells_truncated"
  | "rows_truncated"
  | "slides_truncated"
  | "macros_ignored"
  | "embedded_objects_skipped"
  | "images_skipped"
  | "charts_skipped"
  | "external_links_ignored"
  | "formulas_not_evaluated"
  | "parts_skipped"
  | "part_unreadable"
  | "encoding_fallback"
  | "extension_mismatch"
  | "hidden_content"
  | "unknown_entity"
  | "tracked_changes_skipped"
  | "outputs_skipped"
  | "ocr_disabled"
  | "ocr_unavailable"
  | "ocr_failed"
  | "empty_document";

export interface ExtractionWarning {
  code: ExtractionWarningCode;
  message: string;
  /** «стр. 3», «лист «Данные»», «слайд 4» — без путей файловой системы. */
  location: string | null;
}

export interface ExtractionMetadata {
  title: string | null;
  author: string | null;
  subject: string | null;
  application: string | null;
  producer: string | null;
  created: string | null;
  modified: string | null;
  pageCount: number | null;
  sheetCount: number | null;
  slideCount: number | null;
  wordCount: number | null;
  pdfVersion: string | null;
  imageFormat: string | null;
  imageWidth: number | null;
  imageHeight: number | null;
  encoding: string | null;
  delimiter: string | null;
  rowCount: number | null;
  columnCount: number | null;
  language: string | null;
}

export interface SheetCell {
  ref: string;
  value: string;
  /** Текст формулы; формулы не вычисляются. */
  formula: string | null;
}

export type ExtractedBlock =
  | { type: "heading"; level: number; text: string }
  | { type: "paragraph"; text: string }
  | { type: "list_item"; level: number; ordered: boolean; text: string }
  | { type: "table"; rows: string[][]; truncated: boolean }
  | { type: "page"; number: number; text: string }
  | { type: "slide"; number: number; title: string | null; body: string[]; notes: string | null; hidden: boolean }
  | {
      type: "sheet";
      name: string;
      hidden: boolean;
      declaredRange: string | null;
      usedRange: string | null;
      rowCount: number;
      columnCount: number;
      cellCount: number;
      formulaCount: number;
      cells: SheetCell[];
      truncated: boolean;
    }
  | { type: "code"; language: string | null; text: string };

export interface ExtractionSummary {
  sha256: string;
  format: ExtractionFormat | null;
  status: ExtractionStatus | null;
  statusMessage: string | null;
  textChars: number;
  truncated: boolean;
  warningCount: number;
  analyzedAt: ISODate;
  /** Файл изменился после анализа. */
  stale: boolean;
  ocrUsed: boolean;
}

export interface MaterialExtractionItem {
  material: MaterialRef;
  name: string;
  fileKind: FileKind;
  sizeBytes: number;
  /** «Входящие» или «ОБА · Неделя 7 · Лекции». */
  context: string;
  extraction: ExtractionSummary | null;
}

export interface MaterialExtraction {
  material: MaterialRef;
  name: string;
  context: string;
  sha256: string;
  extractorVersion: number;
  format: ExtractionFormat;
  status: ExtractionStatus;
  statusMessage: string | null;
  metadata: ExtractionMetadata;
  blocks: ExtractedBlock[];
  blocksTruncated: boolean;
  text: string;
  textChars: number;
  truncated: boolean;
  warnings: ExtractionWarning[];
  ocrUsed: boolean;
  sourceSizeBytes: number;
  durationMs: number;
  extractedAt: ISODate;
  stale: boolean;
  /** Взято из cache по SHA-256 без повторного разбора. */
  cached: boolean;
}

export type ExtractionItemOutcome = "extracted" | "cached" | "failed" | "cancelled";

export interface ExtractionBatchItem {
  material: MaterialRef;
  name: string;
  outcome: ExtractionItemOutcome;
  status: ExtractionStatus | null;
  textChars: number;
  error: BridgeErrorPayload | null;
}

export interface ExtractionBatchReport {
  operationId: string;
  items: ExtractionBatchItem[];
  extracted: number;
  cached: number;
  failed: number;
  cancelled: number;
  stoppedByUser: boolean;
  replayed: boolean;
}

export interface OcrStatus {
  available: boolean;
  engine: string | null;
  languages: string | null;
  message: string;
}

export interface ExtractionLimitsInfo {
  maxInputBytes: number;
  maxTextChars: number;
  timeLimitSeconds: number;
  maxPdfPages: number;
  maxSheets: number;
  maxCellsPerSheet: number;
  maxSlides: number;
  maxCorpusChars: number;
}

export interface ExtractionSettings {
  /** Локальный OCR изображений. По умолчанию выключен. */
  ocrEnabled: boolean;
  ocr: OcrStatus;
  limits: ExtractionLimitsInfo;
}

export interface CorpusItem {
  material: MaterialRef;
  name: string;
  context: string;
  sha256: string;
  status: ExtractionStatus | null;
  textChars: number;
  warningCount: number;
  /** Материал на месте, содержимое не менялось, текст извлечён. */
  ready: boolean;
  problem: string | null;
}

/** Явно выбранный пользователем набор материалов — вход будущего SourceBundle. */
export interface CorpusSelection {
  homeworkId: string | null;
  updatedAt: ISODate | null;
  items: CorpusItem[];
  totalChars: number;
  limitChars: number;
}

export interface CorpusDocument {
  material: MaterialRef;
  name: string;
  sha256: string;
  format: ExtractionFormat;
  status: ExtractionStatus;
  text: string;
  textChars: number;
  truncated: boolean;
  warnings: ExtractionWarning[];
}

/** Ровно то, что получит будущий анализ. Сам анализ не запускается. */
export interface CorpusPreview {
  homeworkId: string | null;
  documents: CorpusDocument[];
  excluded: { material: MaterialRef; name: string; reason: string }[];
  totalChars: number;
  limitChars: number;
  truncated: boolean;
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
  /** Optimistic revision: меняется при каждой мутации рабочего цикла. */
  revision: number;
}

// ───────────────────────── Провайдеры ─────────────────────────

export interface ProviderStatus {
  provider: ProviderId;
  state: ProviderState;
  /** Отображаемая метка клиента («Codex CLI · 0.47.0»). Никогда не аккаунт и не токен. */
  accountLabel: string | null;
  lastCheckedAt: ISODate;
  errorMessage: string | null;
  /**
   * Через что backend работает с провайдером: «cli» — официальный CLI, явно подключённый
   * пользователем; «manual_handoff» — ручная передача (копировать промт, вставить ответ);
   * «unavailable» — CLI подключён, но сейчас не работает; «mock» — демонстрационный адаптер.
   */
  adapter: "cli" | "desktop_app" | "mock" | "manual_handoff" | "unavailable";
  /** Роли, назначенные провайдеру в настройках рабочего цикла. */
  roles: WorkflowRole[];
  /** Этап 9: сводное состояние — подключён / ручная передача / недоступен / ошибка. */
  availability: ProviderAvailability;
  /** AI-задачи, назначенные провайдеру. */
  tasks: AiTask[];
  cli: ProviderCli;
  isolation: ProviderIsolation;
  handoff: ProviderHandoff;
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
  | "import"
  | "extract"
  | "workflow_transition"
  | "confirm_style"
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
  /** Список «Входящих» изменился; клиент перечитывает getInboxFiles. */
  "inbox:updated": { reason: "import"; fileIds: string[] };
  /** Состояние watcher загрузок и список кандидатов изменились. */
  "downloads:watcher-updated": { watcher: DownloadsWatcher };
  /** Фаза импорта экспорта ЦУ: проверка → распаковка во временную папку → импорт → удаление временных файлов. */
  "cu-import:progress": { operationId: string; phase: CuImportPhase; completed: number; total: number; entryName: string | null };
  /** Прогресс пакетного извлечения текста (этап 7). */
  "extraction:progress": { operationId: string; completed: number; total: number; material: MaterialRef | null; name: string | null; outcome: ExtractionItemOutcome | null };
  /** Результаты извлечения изменились: анализ, перестроение или очистка cache. */
  "extraction:updated": { reason: "analyzed" | "cleared"; materials: MaterialRef[] };
  /** Агрегат ДЗ изменился (новая ревизия) — клиент перечитывает getWorkflow. */
  "homework:updated": { homeworkId: string; revision: number; stage: HomeworkWorkflowStage; reason: string; operationId: string };
  /** Создан черновик промта (первичного или исправлений). */
  "homework:revision-prompt-created": { homeworkId: string; promptId: string; kind: RevisionPrompt["kind"]; status: RevisionPrompt["status"]; reviewRunId: string | null; basedOnVersionId: string | null };
  /** Готовность к сдаче пересчитана после мутации. */
  "homework:readiness-updated": { homeworkId: string; ready: boolean; blockers: Readiness["checks"][number]["id"][]; revision: number };
  /** Этап 9: прогресс выполняющегося задания (фаза, время, объём вывода — без содержимого). */
  "ai-job:progress": { jobId: string; runId: string; homeworkId: string; phase: "started" | "running" | "parsing"; elapsedMs: number; outputBytes: number };
  /** Этап 9: задание изменилось (подготовлено, отправлено, ответ получен, ошибка, отмена, подтверждение). */
  "ai-job:updated": { jobId: string; runId: string; homeworkId: string; status: AiJobStatus; parseStatus: AiParseStatus | null };
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
  | "database"
  | "backend_unavailable"
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
  /** Системный диалог выбора файлов. Реальный Tauri-адаптер возвращает sourcePath для каждого файла. */
  pickFiles(): Promise<DroppedItem[]>;
  /** Native drag-and-drop окна. Mock/browser возвращает пустую отписку и использует HTML5 drop. */
  onNativeFileDrop(listener: (items: DroppedItem[]) => void): Unsubscribe;
  /** Файлы тащат над окном (native). Mock/browser — пустая отписка. */
  onNativeDragState(listener: (active: boolean) => void): Unsubscribe;
  importDroppedFiles(items: DroppedItem[]): Promise<InboxFile[]>;
  /**
   * Импорт во «Входящие» с отчётом по каждому файлу: импортирован, дубль или ошибка.
   * operationId (UUID v4) задаёт клиент, чтобы сопоставить operation:progress и
   * безопасно повторить команду: повтор с тем же id возвращает сохранённый итог.
   */
  importFiles(items: DroppedItem[], operationId?: string): Promise<OperationResult<InboxImportReport>>;
  getOperationResult(operationId: string): Promise<OperationRecord | null>;
  openInboxFile(fileId: string): Promise<void>;
  revealInboxFile(fileId: string): Promise<void>;

  getManagedRoots(): Promise<ManagedRoots>;
  /** Системный выбор папки (backend). Отмена — cancelled: true. */
  chooseManagedRoot(kind: SelectableRootKind): Promise<RootSelection>;
  /** Создать «Входящие» внутри хранилища, если папки ещё нет. */
  prepareInboxRoot(): Promise<ManagedRoots>;
  // ── Загрузки и экспорт ЦУ (этап 6). Вход в LMS и отправка работ не автоматизируются. ──
  getDownloadsWatcher(): Promise<DownloadsWatcher>;
  /** Включение — только явным действием пользователя в Настройках. */
  setDownloadsWatcher(enabled: boolean): Promise<DownloadsWatcher>;
  dismissDownloadCandidate(candidateId: string): Promise<DownloadsWatcher>;
  /** Системный выбор ZIP/PDF экспорта CU LMS Enhancer. Отмена — null. */
  pickCuExport(): Promise<DownloadCandidate | null>;
  /** Предпросмотр без распаковки и без записи на диск. */
  previewCuExport(candidateId: string): Promise<ExportPreview>;
  /** Импорт подтверждённых записей во «Входящие». Ничего не раскладывается автоматически. */
  importCuExport(request: ExportImportRequest, operationId?: string): Promise<OperationResult<InboxImportReport>>;
  setSubjectCourseUrl(subjectId: string, url: string | null): Promise<Subject>;
  setHomeworkAssignmentUrl(homeworkId: string, url: string | null): Promise<Homework>;
  /** Открывает ссылку во внешнем браузере после проверки allowlist HTTPS-домена LMS. */
  openCourse(target: CourseLinkTarget): Promise<void>;
  openAssignment(target: AssignmentLinkTarget): Promise<void>;

  // ── Локальное извлечение текста (этап 7). Codex/Claude не вызываются. ──
  listMaterialExtractions(): Promise<MaterialExtractionItem[]>;
  /** Сохранённый результат; null — текст из материала ещё не извлекался. */
  getMaterialExtraction(material: MaterialRef): Promise<MaterialExtraction | null>;
  /** Анализ одного материала. Тот же SHA-256 берётся из cache; force перестраивает. */
  analyzeMaterial(material: MaterialRef, options?: { force?: boolean }): Promise<MaterialExtraction>;
  /** Пакет с прогрессом extraction:progress; отмена — cancelExtraction(operationId). */
  analyzeMaterials(materials: MaterialRef[], options?: { force?: boolean; operationId?: string }): Promise<OperationResult<ExtractionBatchReport>>;
  /** false — пакет уже завершён. */
  cancelExtraction(operationId: string): Promise<boolean>;
  clearExtractionCache(): Promise<{ removed: number }>;
  getExtractionSettings(): Promise<ExtractionSettings>;
  setExtractionSettings(patch: { ocrEnabled: boolean }): Promise<ExtractionSettings>;
  /** homeworkId = null — общая подборка без ДЗ. */
  getCorpusSelection(homeworkId: string | null): Promise<CorpusSelection>;
  setCorpusSelection(homeworkId: string | null, materials: MaterialRef[]): Promise<CorpusSelection>;
  previewCorpus(homeworkId: string | null): Promise<CorpusPreview>;

  classifyFiles(fileIds: string[]): Promise<OperationResult<InboxFile[]>>;
  updateClassification(fileId: string, patch: ClassificationPatch): Promise<InboxFile>;
  previewRoute(fileId: string, destination: RouteDestination): Promise<RoutePreview>;
  /** destination = null → использовать классификацию каждого файла. */
  routeFiles(fileIds: string[], destination: RouteDestination | null): Promise<OperationResult<RoutePreview[]>>;

  getHomeworkList(): Promise<Homework[]>;
  getHomework(homeworkId: string): Promise<Homework | null>;
  getHomeworkCandidates(homeworkId: string): Promise<FileRef[]>;
  /** Новая append-only версия; номер назначает backend. attachmentFileIds — приложения той же версии. */
  createHomeworkVersion(
    homeworkId: string,
    fileId: string,
    meta?: { author?: VersionAuthor; promptId?: string | null; attachmentFileIds?: string[]; note?: string | null } & WorkflowCommandOptions,
  ): Promise<OperationResult<HomeworkVersion>>;
  /** Выбор текущей проверяемой версии (она же итоговая до снимка этапа 10). */
  markVersionFinal(versionId: string, options?: WorkflowCommandOptions): Promise<OperationResult<Homework>>;

  // ── Рабочий цикл ДЗ ──
  /** Весь рабочий цикл ДЗ одним запросом: этапы, наборы, запуски, промты, готовность, переходы. null — ДЗ нет. */
  getWorkflow(homeworkId: string): Promise<HomeworkWorkflow | null>;
  getStyleProfile(): Promise<StyleProfile | null>;
  /** Неизменяемый снимок подтверждённой подборки материалов для запуска. Пустые обязательные категории → missing. */
  prepareSourceBundle(homeworkId: string, purpose: RunKind, versionId?: string | null, options?: WorkflowCommandOptions): Promise<SourceBundle>;
  setSourceIncluded(bundleId: string, itemId: string, included: boolean): Promise<SourceBundle>;
  /** Аналитик читает материалы и создаёт первичный промт для автора. Без адаптера — waiting_for_provider/manual_handoff. */
  runMaterialAnalysis(homeworkId: string, bundleId: string, options?: WorkflowCommandOptions & { mode?: RunMode }): Promise<ReviewRun>;
  /** Проверяющий проверяет версию по всем критериям. */
  runVersionReview(homeworkId: string, versionId: string, bundleId: string, options?: WorkflowCommandOptions & { mode?: RunMode }): Promise<ReviewRun>;
  /** Отдельный запуск «создать промт исправлений» по версии (учитывает и замечания, и требования преподавателя). */
  runRevisionPrompt(homeworkId: string, versionId: string, bundleId: string, options?: WorkflowCommandOptions & { mode?: RunMode }): Promise<ReviewRun>;
  /** Ручной итог анализа материалов: запуск завершается с provenance manual, создаётся черновик первичного промта. */
  saveManualAnalysis(runId: string, input: ManualAnalysisInput, options?: WorkflowCommandOptions): Promise<OperationResult<ReviewRun>>;
  /** Ручной результат проверки версии или проверки по комментариям преподавателя (provenance manual). */
  saveManualReviewResult(runId: string, input: ManualReviewInput, options?: WorkflowCommandOptions): Promise<OperationResult<ReviewRun>>;
  /** Черновик промта исправлений по проверке: только выявленные проблемы. */
  createRevisionPrompt(reviewRunId: string, editedText?: string, options?: WorkflowCommandOptions): Promise<OperationResult<RevisionPrompt>>;
  /** Фиксирует черновик: после этого промт неизменяем. */
  commitPrompt(promptId: string, options?: WorkflowCommandOptions): Promise<OperationResult<RevisionPrompt>>;
  /** Передаёт зафиксированный промт роли «автор». Без адаптера — отметка ручной передачи. */
  sendPromptToAuthor(promptId: string, options?: WorkflowCommandOptions): Promise<OperationResult<RevisionPrompt>>;
  /** Меняет только черновик. */
  updatePromptText(promptId: string, text: string, options?: WorkflowCommandOptions): Promise<RevisionPrompt>;
  getPrompts(homeworkId: string): Promise<RevisionPrompt[]>;
  cancelRun(runId: string, options?: WorkflowCommandOptions): Promise<ReviewRun>;
  getReviewRuns(homeworkId: string): Promise<ReviewRun[]>;
  /** Пользователь закрепляет результат запуска в истории итераций. */
  saveRunAsIteration(runId: string, options?: WorkflowCommandOptions): Promise<ReviewRun>;
  getIterations(homeworkId: string): Promise<HomeworkIteration[]>;
  compareVersions(fromVersionId: string, toVersionId: string): Promise<VersionComparison>;
  /** Явное подтверждение пользователем расхождения со стилем выше применённого порога. */
  confirmStyleDivergence(reviewRunId: string, options?: WorkflowCommandOptions): Promise<OperationResult<StyleConfirmation>>;
  /** Допустимый переход этапа (возвраты и решения пользователя) — проверяет backend state machine. */
  transitionWorkflow(homeworkId: string, to: HomeworkWorkflowStage, options?: WorkflowCommandOptions & { reason?: string }): Promise<OperationResult<Homework>>;

  getTeacherComments(homeworkId: string): Promise<TeacherComment[]>;
  saveTeacherComments(homeworkId: string, comments: TeacherCommentInput[], options?: WorkflowCommandOptions): Promise<OperationResult<TeacherComment[]>>;
  /** Ручное добавление требования, выведенного из комментария преподавателя. */
  addTeacherRequirement(homeworkId: string, input: TeacherRequirementInput, options?: WorkflowCommandOptions): Promise<OperationResult<TeacherRequirement>>;
  /** Разбирает вставленную переписку или файл (md/txt/pdf/изображение) на пары «вопрос — ответ». */
  importTeacherTranscript(homeworkId: string, input: { text?: string; file?: DroppedItem; source: TeacherComment["source"]; date: ISODate }): Promise<OperationResult<TeacherComment[]>>;
  removeTeacherComment(commentId: string): Promise<void>;
  runTeacherReview(homeworkId: string, versionId: string, bundleId: string, options?: WorkflowCommandOptions & { mode?: RunMode }): Promise<ReviewRun>;
  getTeacherRequirements(homeworkId: string): Promise<TeacherRequirement[]>;
  /** Пользователь вручную закрывает требование со статусом «требует ручной проверки». */
  resolveRequirement(requirementId: string, status: "done" | "not_done" | "na", note: string, options?: WorkflowCommandOptions): Promise<TeacherRequirement>;

  getReadiness(homeworkId: string): Promise<Readiness>;
  /** Только по действию пользователя и только если все проверки readiness пройдены. */
  markReadyToSubmit(homeworkId: string, options: { noTeacherComments: boolean } & WorkflowCommandOptions): Promise<OperationResult<Homework>>;
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

  /** Явное подключение официального CLI. Найденный CLI сам не включается; пароли и токены не запрашиваются. */
  connectProvider(provider: ProviderId): Promise<ProviderStatus>;
  disconnectProvider(provider: ProviderId): Promise<ProviderStatus>;
  getProviderStatus(provider: ProviderId): Promise<ProviderStatus>;

  // ── Провайдеры и задания передачи (этап 9) ──
  getProviderStatuses(): Promise<ProviderStatus[]>;
  /** Повторная проверка CLI: версия, флаги неинтерактивного режима, вход (только код выхода). */
  checkProvider(provider: ProviderId): Promise<ProviderStatus>;
  /** OS-level self-test; automatic mode stays blocked until it passes. */
  runAiIsolationSelfTest(): Promise<IsolationReport>;
  /** Задания ДЗ без текстов. */
  listAiJobs(homeworkId: string): Promise<AiJob[]>;
  /** Задание целиком: полный текст передачи, ответ, черновик, журнал. */
  getAiJob(jobId: string): Promise<AiJob>;
  /** Подготовка передачи: ровно тот текст, который получит провайдер. Ничего не отправляет. */
  prepareAiJob(runId: string, task: AiTask, transport?: TransportPreference): Promise<AiJob>;
  /** Явная отправка подготовленного задания в подключённый официальный CLI. */
  sendAiJob(jobId: string): Promise<AiJob>;
  /** false — задание не выполняется. */
  cancelAiJob(jobId: string): Promise<boolean>;
  /** Ответ, вставленный пользователем после ручной передачи. */
  submitAiResponse(jobId: string, text: string): Promise<AiJob>;
  /** Системный выбор файла ответа (txt/md/json, ≤ 2 МБ). Отмена — null. */
  importAiResponseFile(jobId: string): Promise<AiJob | null>;
  discardAiJob(jobId: string): Promise<AiJob>;
  /** Подтверждение проверенного (и, возможно, исправленного) результата. Только оно меняет рабочий цикл. */
  confirmAiResult(
    jobId: string,
    input: AiConfirmInput,
    options?: WorkflowCommandOptions & { supportingJobId?: string | null },
  ): Promise<OperationResult<AiConfirmation>>;

  getAutomationRules(): Promise<AutomationRules>;
  updateAutomationRules(rules: AutomationRules): Promise<OperationResult<AutomationRules>>;

  getActivityLog(): Promise<Operation[]>;
  undoOperation(operationId: string): Promise<OperationResult<null>>;

  on<E extends BridgeEventName>(event: E, listener: BridgeListener<E>): Unsubscribe;
}
