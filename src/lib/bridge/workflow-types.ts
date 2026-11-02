/**
 * Рабочий цикл домашней работы: материалы → анализ → промт → решение → проверка →
 * исправления → комментарии преподавателя → комплект → ручная отметка отправки.
 *
 * Компоненты не зависят от конкретного провайдера: в данных фигурируют роли
 * (аналитик, автор, проверяющий), а сопоставление роль → провайдер задаётся в
 * AutomationRules.workflow и может быть изменено пользователем.
 */
import type { FileRef, ISODate, MaterialRef, ProviderId } from "./types";

// ───────────── Этапы ─────────────

export type HomeworkWorkflowStage =
  | "materials_collected" // 1. Материалы собраны
  | "codex_analysis" // 2. Анализ материалов (роль «аналитик», по умолчанию Codex)
  | "prompt_ready" // 3. Промт для автора готов
  | "claude_solving" // 4. Автор создаёт решение (по умолчанию Claude)
  | "codex_review" // 5. Проверка версии (роль «проверяющий»)
  | "needs_fixes" // 6. Требуются исправления
  | "teacher_review" // 7. Проверка по комментариям преподавателя
  | "ready_to_submit" // 8. Готово к сдаче — только явным действием пользователя
  | "submitted"; // 9. Отправлено — только ручным подтверждением

export const WORKFLOW_STAGE_ORDER: readonly HomeworkWorkflowStage[] = [
  "materials_collected",
  "codex_analysis",
  "prompt_ready",
  "claude_solving",
  "codex_review",
  "needs_fixes",
  "teacher_review",
  "ready_to_submit",
  "submitted",
];

export type WorkflowRole = "analyst" | "author" | "reviewer";

/** Кто создал версию. */
export type VersionAuthor = ProviderId | "user";

// ───────────── Источники ─────────────

export type SourceKind =
  | "lecture"
  | "seminar"
  | "task"
  | "data"
  | "extra"
  | "style"
  | "previous_work"
  | "current_version"
  | "previous_version"
  | "teacher_comments";

export interface SourceItem {
  id: string;
  kind: SourceKind;
  /** Снимок файла на момент создания набора (имя, путь, размер). */
  file: FileRef;
  required: boolean;
  /** Пользователь может исключить любой необязательный источник. */
  included: boolean;
  /** Материал подборки этапа 7, из которого сделан снимок. */
  material: MaterialRef;
  /** SHA-256 содержимого в момент снимка; null только у legacy-наборов. */
  sha256: string | null;
  extractionId: string | null;
  /** Версия правил извлечения текста, по которым получен корпус. */
  extractorVersion: number | null;
  textChars: number;
  /** Коды предупреждений извлечения. */
  warnings: string[];
}

export interface BundleWarning {
  code: string;
  message: string;
  materialId: string | null;
}

export interface MissingSource {
  kind: SourceKind;
  label: string;
  required: boolean;
  hint: string;
}

export type RunKind = "analyze_materials" | "review_version" | "revision_prompt" | "review_teacher";

/**
 * Неизменяемый снимок подтверждённой подборки материалов для конкретного запуска.
 * Изменение подборки после создания набора не меняет уже созданный набор.
 */
export interface SourceBundle {
  id: string;
  homeworkId: string;
  purpose: RunKind;
  versionId: string | null;
  items: SourceItem[];
  missing: MissingSource[];
  /** false, если не хватает обязательных материалов — запуск заблокирован. */
  complete: boolean;
  createdAt: ISODate;
  /** corpus_selection — снимок подборки; legacy — набор схемы до этапа 8. */
  origin: "legacy" | "corpus_selection";
  selectionId: string | null;
  extractorVersion: number | null;
  /** Предупреждения корпуса: частичное извлечение, обрезанный текст, превышение лимита. */
  warnings: BundleWarning[];
  totalChars: number;
  limitChars: number;
}

export interface StyleProfile {
  id: string;
  fileName: string;
  relativePath: string;
  updatedAt: ISODate;
  /** Краткие черты стиля из файла — только для показа. */
  traits: string[];
}

// ───────────── Проверка ─────────────

export type CriterionId = "correctness" | "task_fit" | "course_grounding" | "style" | "teacher" | "completeness";
export type CriterionStatus = "pass" | "warn" | "fail" | "manual" | "na";

export interface ReviewCriterion {
  id: CriterionId;
  label: string;
  status: CriterionStatus;
  note: string;
}

/** Ссылка на фрагмент исходного материала, на который опирается вывод. */
export interface EvidenceReference {
  sourceId: string;
  fileName: string;
  /** «слайд 14», «стр. 3», «лист Данные, B2:B40», «вопрос 2» */
  locator: string;
}

export type FindingSeverity = "critical" | "normal";

export interface ReviewFinding {
  id: string;
  severity: FindingSeverity;
  criterion: CriterionId;
  text: string;
  location: string | null;
  evidence: EvidenceReference[];
  /** Версия, в которой замечание появилось впервые. */
  introducedIn: string;
  /** Версия, в которой замечание исправлено (null — открыто). */
  fixedIn: string | null;
}

export interface UnsupportedClaim {
  id: string;
  text: string;
  location: string;
}

export type StyleReasonKind = "template" | "uncharacteristic_words" | "formal_tone" | "repetition" | "unsupported";

/**
 * Расхождение с личным стилем, 0…10. Это сравнение с профилем стиля и прошлыми
 * работами пользователя, а не определение авторства текста.
 */
export interface StyleDivergence {
  score: number;
  threshold: number;
  reasons: { kind: StyleReasonKind; text: string; example: string | null }[];
}

export type ReviewVerdict = "pass" | "needs_fixes" | "needs_teacher_review" | "blocked";

export type RecommendedActionKind = "send_prompt_to_author" | "create_revision_prompt" | "add_teacher_comments" | "mark_ready" | "complete_materials";

/** Отсутствующий файл или ссылка, найденные проверкой. */
export interface ReviewMissingItem {
  kind: "file" | "link";
  label: string;
  required: boolean;
}

export interface ReviewResult {
  verdict: ReviewVerdict;
  summary: string;
  criteria: ReviewCriterion[];
  critical: ReviewFinding[];
  remarks: ReviewFinding[];
  unsupportedClaims: UnsupportedClaim[];
  style: StyleDivergence | null;
  teacherRequirements: TeacherRequirement[];
  recommendedAction: { kind: RecommendedActionKind; label: string };
  /** Обязательные файлы и ссылки, которых не хватает. */
  missing: ReviewMissingItem[];
  /** Готовый промт: для анализа — первичный, для проверки — промт исправлений. */
  prompt: RevisionPrompt | null;
}

/**
 * Состояние запуска. waiting_for_provider — роль назначена провайдеру, но AI-адаптера
 * ещё нет; manual_handoff — шаг выполняется вручную, результат сохраняет пользователь.
 */
export type ReviewRunStatus = "queued" | "waiting_for_provider" | "manual_handoff" | "running" | "completed" | "failed" | "cancelled";

/** Происхождение результата: manual — сохранён пользователем, provider — AI-адаптер, legacy — до этапа 8. */
export type Provenance = "legacy" | "manual" | "provider";

/** Отдельно сохранённые критерии завершённой проверки — по ним считается готовность. */
export interface ReviewAssessment {
  verdict: ReviewVerdict;
  correctness: CriterionStatus;
  taskFit: CriterionStatus;
  courseGrounding: CriterionStatus;
  teacher: CriterionStatus;
  styleScore: number | null;
  criticalCount: number;
  unsupportedCount: number;
  missingRequired: number;
}

export interface ReviewRun {
  id: string;
  homeworkId: string;
  kind: RunKind;
  role: WorkflowRole;
  provider: ProviderId;
  versionId: string | null;
  /** null только у запусков схемы v1. */
  sourceBundleId: string | null;
  /** Полный список переданных источников — сохраняется всегда. */
  sourcesUsed: EvidenceReference[];
  status: ReviewRunStatus;
  progress: number;
  step: string;
  startedAt: ISODate;
  finishedAt: ISODate | null;
  /** Пользователь закрепил результат в истории итераций. */
  savedAsIteration: boolean;
  result: ReviewResult | null;
  errorMessage: string | null;
  provenance: Provenance;
  /** Порог стиля, применённый в момент запуска; не меняется вместе с настройкой. */
  styleThreshold: number;
  assessment: ReviewAssessment | null;
  /** Отпечаток входных данных на момент запуска; null — запуск до этапа 7. */
  inputsFingerprint: string | null;
  /** Входные данные изменились после запуска: в истории, но не действует. */
  stale: boolean;
}

export type PromptKind = "initial" | "revision" | "teacher";

export interface RevisionPrompt {
  id: string;
  homeworkId: string;
  kind: PromptKind;
  text: string;
  createdAt: ISODate;
  createdBy: ProviderId | "user";
  /** Для какой версии и по какой проверке создан. */
  basedOnVersionId: string | null;
  reviewRunId: string | null;
  /** Какие замечания/требования промт должен закрыть. */
  addresses: string[];
  sources: EvidenceReference[];
  /** Передан автору (вручную или адаптером). */
  sentAt: ISODate | null;
  /** draft — можно редактировать; committed — зафиксирован и неизменяем. */
  status: PromptStatus;
  committedAt: ISODate | null;
  provenance: Provenance;
  updatedAt: ISODate;
  inputsFingerprint: string | null;
  /** Составлен по устаревшим входным данным — передать автору нельзя. */
  stale: boolean;
}

export type PromptStatus = "draft" | "committed";

// ───────────── Преподаватель ─────────────

export type TeacherCommentSource = "chat" | "email" | "lms" | "in_person" | "other";

/**
 * Этап 7: кто дал ответ. Официальным может стать только ответ преподавателя или
 * ассистента — и только после явного подтверждения пользователем.
 */
export type CommentAuthority = "teacher_official" | "teaching_assistant" | "student" | "group_chat_unverified" | "user_note" | "system" | "unknown";

export const OFFICIAL_AUTHORITIES: readonly CommentAuthority[] = ["teacher_official", "teaching_assistant"];

export interface CommentAuthorityInput {
  authority: CommentAuthority;
  author: string;
  /** Ссылка на сообщение или цитата с контекстом. */
  context: string;
  confirmedOfficial: boolean;
}

export interface TeacherCommentInput extends Partial<CommentAuthorityInput> {
  source: TeacherCommentSource;
  date: ISODate;
  question: string;
  answer: string;
}

export interface TeacherComment {
  id: string;
  homeworkId: string;
  source: TeacherCommentSource;
  date: ISODate;
  question: string;
  answer: string;
  addedAt: ISODate;
  attachment: FileRef | null;
  authority: CommentAuthority;
  author: string;
  context: string;
  /** Преподаватель/ассистент + подтверждение пользователя. */
  official: boolean;
  officialConfirmedAt: ISODate | null;
}

export type RequirementStatus = "done" | "not_done" | "manual" | "na";

export interface TeacherRequirement {
  id: string;
  text: string;
  /** null — legacy-требование без комментария. */
  commentId: string | null;
  /** Короткий фрагмент вопроса/ответа, из которого выведено требование. */
  origin: string;
  status: RequirementStatus;
  note: string;
  checkedVersionId: string | null;
  /** Отозвано (ответ перестал быть официальным или удалено) — хранится для истории, не учитывается. */
  withdrawnAt: ISODate | null;
}

export interface TeacherRequirementInput {
  commentId: string;
  text: string;
  origin: string;
  status: RequirementStatus;
  note?: string;
}

// ───────────── Итерации ─────────────

export interface HomeworkIteration {
  versionId: string;
  label: string;
  author: VersionAuthor;
  createdAt: ISODate;
  files: FileRef[];
  /** Промт, по которому создана версия. */
  prompt: RevisionPrompt | null;
  sourceBundleId: string | null;
  /** Последняя закреплённая проверка версии. */
  review: ReviewRun | null;
  teacherReview: ReviewRun | null;
  /** Промт исправлений, созданный по проверке. */
  revisionPrompt: RevisionPrompt | null;
  nextVersionId: string | null;
  fixedFindings: ReviewFinding[];
  openFindings: ReviewFinding[];
  status: "draft" | "reviewing" | "failed" | "passed" | "final" | "submitted";
}

export interface VersionComparison {
  fromVersionId: string;
  toVersionId: string;
  fixed: ReviewFinding[];
  introduced: ReviewFinding[];
  sections: { title: string; diff: "same" | "changed" | "added" | "removed" }[];
  style: { from: number | null; to: number | null };
  grounding: { from: CriterionStatus | null; to: CriterionStatus | null; unsupportedFrom: number; unsupportedTo: number };
  teacher: { from: { done: number; total: number } | null; to: { done: number; total: number } | null };
}

// ───────────── Готовность и сдача ─────────────

export type ReadinessCheckId =
  | "materials"
  | "review_current"
  | "review_passed"
  | "no_critical"
  | "task_coverage"
  | "teacher"
  | "attachments"
  | "style"
  | "sources_fresh"
  | "final_version"
  | "inputs_fresh";

export interface ReadinessCheck {
  id: ReadinessCheckId;
  label: string;
  ok: boolean;
  detail: string;
}

/** Объяснение, почему работа может или не может стать «Готово к сдаче». */
export interface Readiness {
  homeworkId: string;
  ready: boolean;
  checks: ReadinessCheck[];
  /** Проверяемая версия и проверка, по которым посчитана готовность. */
  versionId: string | null;
  reviewRunId: string | null;
  /** Ревизия ДЗ, на которой посчитан ответ. */
  revision: number;
  /** Отпечаток входных данных, по которым посчитана готовность (этап 7). */
  inputsFingerprint: string;
}

/** Этап 7: отпечатки входных данных ДЗ (SHA-256). */
export interface InputsFingerprint {
  materials: string;
  task: string;
  version: string;
  style: string;
  requirements: string;
  combined: string;
}

/** Явное подтверждение пользователем расхождения со стилем выше порога. */
export interface StyleConfirmation {
  id: string;
  homeworkId: string;
  versionId: string;
  reviewRunId: string;
  score: number;
  threshold: number;
  confirmedBy: "user";
  confirmedAt: ISODate;
}

export type PackageCheckId = "final_selected" | "name_checked" | "attachments" | "links" | "teacher" | "manual_review" | "inputs_fresh";

/** Этап 8: состояние файла, ссылки или пункта комплекта. */
export type PackageStatus = "verified" | "missing" | "stale" | "manual_required" | "inaccessible" | "mismatch" | "not_applicable";

export interface PackageCheck {
  id: PackageCheckId;
  label: string;
  /** Выполнен: проверен, неприменим или подтверждён вручную. */
  done: boolean;
  /** true — проверяется приложением; false — отмечает пользователь. */
  auto: boolean;
  status: PackageStatus;
  detail: string;
  manualConfirmed: boolean;
}

export interface PackageItem {
  id: string;
  role: "main" | "attachment" | "extra" | "cloud";
  file: FileRef | null;
  /** Итоговое имя в «03 На сдачу». */
  finalName: string;
  required: boolean;
  present: boolean;
  status: PackageStatus;
  sha256: string | null;
  detail: string;
}

export interface PackageLink {
  id: string;
  label: string;
  url: string;
  required: boolean;
  /** Сетевой проверки нет: всегда false, доступность подтверждает пользователь. */
  verified: boolean;
  status: PackageStatus;
}

export interface PackageRequirement {
  id: string;
  text: string;
  status: RequirementStatus;
}

export interface PackageRevisionSummary {
  id: string;
  revision: number;
  versionId: string;
  builtAt: ISODate;
  stale: boolean;
  submitted: boolean;
}

export interface SubmissionPackage {
  id: string;
  homeworkId: string;
  versionId: string;
  folderPath: string;
  items: PackageItem[];
  links: PackageLink[];
  warnings: string[];
  checklist: PackageCheck[];
  /** Что прикрепить в систему вуза — в порядке загрузки. */
  uploadList: string[];
  builtAt: ISODate;
  inputsFingerprint: string | null;
  /** Собран по устаревшим входным данным или есть более новая ревизия — подтвердить отправку нельзя. */
  stale: boolean;
  /** Этап 8: номер неизменяемой ревизии, требования и отпечатки входных данных на момент сборки. */
  revision: number;
  requirements: PackageRequirement[];
  inputs: InputsFingerprint | null;
  submitted: boolean;
}

export interface SubmissionConfirmation {
  submittedAt: ISODate;
  assignmentUrl: string;
  proof: { name: string; sizeBytes: number; path?: string } | null;
  /** Должно быть true — пользователь подтвердил, что отправил сам. */
  userConfirmed: true;
}

export interface SubmissionReceipt {
  id: string;
  homeworkId: string;
  versionId: string;
  packageId: string;
  submittedAt: ISODate;
  confirmedAt: ISODate;
  assignmentUrl: string;
  proof: FileRef | null;
  snapshotPath: string;
  files: FileRef[];
  /** Хэш снимка «04 Отправлено» — снимок неизменяем. */
  snapshotHash: string;
}

// ───────────── Провайдеры ─────────────

export interface WorkflowSettings {
  roles: Record<WorkflowRole, ProviderId>;
  /** Этап 9: провайдер каждой AI-задачи — независимо от имён этапов. */
  tasks: Record<import("./ai-types").AiTask, ProviderId>;
  /** Порог расхождения с личным стилем, 0…10. По умолчанию 4. */
  styleThreshold: number;
  styleProfilePath: string;
  requireTeacherReview: boolean;
}

// ───────────── Агрегат рабочего цикла (этап 8) ─────────────

/** Как выполнить AI-шаг: provider — ждать адаптер роли; manual — выполнить вручную. */
export type RunMode = "provider" | "manual";

/** Опции любой мутации рабочего цикла: идемпотентность и optimistic revision. */
export interface WorkflowCommandOptions {
  /** UUID v4. Повтор с тем же id возвращает сохранённый итог без повторных действий. */
  operationId?: string;
  /** Ревизия ДЗ, которую видел клиент. Расхождение — BridgeError("conflict"). */
  expectedRevision?: number;
}

export interface TransitionOption {
  to: HomeworkWorkflowStage;
  /** Решение backend state machine — кнопки блокируются по нему. */
  allowed: boolean;
  reason: string;
}

/** Ручной итог анализа материалов и текст первичного промта (provenance manual). */
export interface ManualAnalysisInput {
  summary: string;
  promptText: string;
}

export interface FindingInput {
  criterion: CriterionId;
  text: string;
  location: string | null;
  evidence?: EvidenceReference[];
}

/** Ручной результат проверки: каждое измерение задаётся отдельно. */
export interface ManualReviewInput {
  verdict: ReviewVerdict;
  summary: string;
  correctness: CriterionStatus;
  taskFit: CriterionStatus;
  courseGrounding: CriterionStatus;
  teacher: CriterionStatus;
  /** Расхождение с личным стилем 0…10 — эвристика, а не определение авторства. */
  styleScore: number | null;
  styleReasons?: { kind: StyleReasonKind; text: string; example: string | null }[];
  critical?: FindingInput[];
  remarks?: FindingInput[];
  unsupportedClaims?: { text: string; location: string }[];
  missing?: ReviewMissingItem[];
}

/** Полное состояние рабочего цикла одного ДЗ, прочитанное одной командой. */
export interface HomeworkWorkflow {
  homework: import("./types").Homework;
  revision: number;
  stage: HomeworkWorkflowStage;
  transitions: TransitionOption[];
  bundles: SourceBundle[];
  runs: ReviewRun[];
  prompts: RevisionPrompt[];
  teacherComments: TeacherComment[];
  teacherRequirements: TeacherRequirement[];
  styleConfirmations: StyleConfirmation[];
  readiness: Readiness;
  iterations: HomeworkIteration[];
  submissionPackage: SubmissionPackage | null;
  submissionReceipt: SubmissionReceipt | null;
  /** Этап 7: текущие отпечатки входных данных. */
  inputs: InputsFingerprint;
  /** Этап 8: все ревизии комплекта, новые сверху. */
  packageHistory: PackageRevisionSummary[];
  /** «Готово к сдаче» отмечено по другим входным данным — отметка не действует. */
  readyStale: boolean;
  /** Задания передачи провайдерам (этап 9) — без текстов промта и ответа. */
  aiJobs: import("./ai-types").AiJob[];
  /** ai_jobs — AI-шаги идут через задания передачи с подтверждением результата пользователем (этап 9). */
  providerMode: "ai_jobs";
}
