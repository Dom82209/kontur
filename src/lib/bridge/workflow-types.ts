/**
 * Рабочий цикл домашней работы: материалы → анализ → промт → решение → проверка →
 * исправления → комментарии преподавателя → комплект → ручная отметка отправки.
 *
 * Компоненты не зависят от конкретного провайдера: в данных фигурируют роли
 * (аналитик, автор, проверяющий), а сопоставление роль → провайдер задаётся в
 * AutomationRules.workflow и может быть изменено пользователем.
 */
import type { FileRef, ISODate, ProviderId } from "./types";

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
  file: FileRef;
  required: boolean;
  /** Пользователь может исключить любой необязательный источник. */
  included: boolean;
}

export interface MissingSource {
  kind: SourceKind;
  label: string;
  required: boolean;
  hint: string;
}

export type RunKind = "analyze_materials" | "review_version" | "revision_prompt" | "review_teacher";

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
  /** Готовый промт: для анализа — первичный, для проверки — промт исправлений. */
  prompt: RevisionPrompt | null;
}

export type ReviewRunStatus = "queued" | "running" | "completed" | "failed" | "cancelled";

export interface ReviewRun {
  id: string;
  homeworkId: string;
  kind: RunKind;
  role: WorkflowRole;
  provider: ProviderId;
  versionId: string | null;
  sourceBundleId: string;
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
  /** Передан автору (backend отдал промт адаптеру). */
  sentAt: ISODate | null;
}

// ───────────── Преподаватель ─────────────

export type TeacherCommentSource = "chat" | "email" | "lms" | "in_person" | "other";

export interface TeacherCommentInput {
  source: TeacherCommentSource;
  date: ISODate;
  question: string;
  answer: string;
}

export interface TeacherComment extends TeacherCommentInput {
  id: string;
  homeworkId: string;
  addedAt: ISODate;
  attachment: FileRef | null;
}

export type RequirementStatus = "done" | "not_done" | "manual" | "na";

export interface TeacherRequirement {
  id: string;
  text: string;
  commentId: string;
  /** Короткий фрагмент вопроса/ответа, из которого выведено требование. */
  origin: string;
  status: RequirementStatus;
  note: string;
  checkedVersionId: string;
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

export interface ReadinessCheck {
  id: "materials" | "review_passed" | "no_critical" | "style" | "teacher" | "final_version";
  label: string;
  ok: boolean;
  detail: string;
}

/** Объяснение, почему работа может или не может стать «Готово к сдаче». */
export interface Readiness {
  homeworkId: string;
  ready: boolean;
  checks: ReadinessCheck[];
}

export type PackageCheckId = "final_selected" | "name_checked" | "attachments" | "links" | "teacher" | "manual_review";

export interface PackageCheck {
  id: PackageCheckId;
  label: string;
  done: boolean;
  /** true — проверяется приложением; false — отмечает пользователь. */
  auto: boolean;
}

export interface PackageItem {
  id: string;
  role: "main" | "attachment" | "extra" | "cloud";
  file: FileRef | null;
  /** Итоговое имя в «03 На сдачу». */
  finalName: string;
  required: boolean;
  present: boolean;
}

export interface PackageLink {
  id: string;
  label: string;
  url: string;
  required: boolean;
  verified: boolean;
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
  /** Порог расхождения с личным стилем, 0…10. По умолчанию 4. */
  styleThreshold: number;
  styleProfilePath: string;
  requireTeacherReview: boolean;
}
