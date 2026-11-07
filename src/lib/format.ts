import type {
  CommentAuthority,
  CriterionStatus,
  FileKind,
  HomeworkStage,
  HomeworkWorkflowStage,
  MaterialCategory,
  ProviderId,
  ProviderState,
  RequirementStatus,
  RunKind,
  SourceKind,
  StyleReasonKind,
  TeacherCommentSource,
  VersionAuthor,
  WorkflowRole,
} from "./bridge/types";

export function plural(n: number, one: string, few: string, many: string): string {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}

export const providerName = (p: ProviderId): string => (p === "codex" ? "Codex" : "Claude");

export const pad2 = (n: number): string => String(n).padStart(2, "0");

export function formatBytes(b: number): string {
  if (b < 1024) return `${b} Б`;
  if (b < 1024 * 1024) return `${Math.round(b / 1024)} КБ`;
  return `${(b / 1024 / 1024).toFixed(1).replace(".", ",")} МБ`;
}

const dateFmt = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "short" });
const timeFmt = new Intl.DateTimeFormat("ru-RU", { hour: "2-digit", minute: "2-digit" });
const weekdayFmt = new Intl.DateTimeFormat("ru-RU", { weekday: "short" });

export const formatDate = (iso: string): string => dateFmt.format(new Date(iso)).replace(".", "");
export const formatTime = (iso: string): string => timeFmt.format(new Date(iso));
export const formatDateTime = (iso: string): string => `${formatDate(iso)}, ${formatTime(iso)}`;

export function formatRelative(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const min = Math.round(diff / 60_000);
  if (min < 1) return "только что";
  if (min < 60) return `${min} мин назад`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h} ч назад`;
  const d = Math.round(h / 24);
  return `${d} ${plural(d, "день", "дня", "дней")} назад`;
}

/** «через 2 дня», «завтра, 18:00», «просрочено». */
export const UNKNOWN_DEADLINE = "срок не указан";

/** Сортировка по сроку: ДЗ с неизвестным сроком — в конце, порядок стабилен. */
export function compareDeadlines(a: string | null, b: string | null): number {
  if (a === b) return 0;
  if (a === null) return 1;
  if (b === null) return -1;
  return a.localeCompare(b);
}

/** Диапазон дат недели или честное «даты не заданы». */
export function formatWeekRange(startsAt: string | null, endsAt: string | null): string {
  return startsAt && endsAt ? `${formatDate(startsAt)} — ${formatDate(endsAt)}` : "даты недели не заданы";
}

export function formatDeadline(iso: string | null): { text: string; tone: "danger" | "warn" | "calm" | "past" | "unknown" } {
  if (!iso) return { text: UNKNOWN_DEADLINE, tone: "unknown" };
  const t = new Date(iso);
  const now = new Date();
  const ms = t.getTime() - now.getTime();
  const startOf = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((startOf(t) - startOf(now)) / 86_400_000);
  if (ms < 0) return { text: `было ${formatDate(iso)}`, tone: "past" };
  if (days === 0) return { text: `сегодня, ${formatTime(iso)}`, tone: "danger" };
  if (days === 1) return { text: `завтра, ${formatTime(iso)}`, tone: "danger" };
  if (days <= 3) return { text: `${weekdayFmt.format(t)}, ${formatTime(iso)} · через ${days} ${plural(days, "день", "дня", "дней")}`, tone: "warn" };
  return { text: `${formatDate(iso)} · через ${days} ${plural(days, "день", "дня", "дней")}`, tone: "calm" };
}

export const CATEGORY_LABEL: Record<MaterialCategory, string> = {
  lecture: "Лекция",
  seminar: "Семинар",
  material: "Материал",
  homework_task: "Условие ДЗ",
  homework_data: "Данные ДЗ",
  homework_draft: "Черновик ДЗ",
  unknown: "Не определено",
};

export const STAGE_LABEL: Record<HomeworkStage, string> = {
  task: "Условие",
  in_progress: "В работе",
  ready: "На сдачу",
  submitted: "Отправлено",
};

export const STAGE_FOLDER: Record<HomeworkStage, string> = {
  task: "01 Условие и данные",
  in_progress: "02 Работа",
  ready: "03 На сдачу",
  submitted: "04 Отправлено",
};

export const PROVIDER_STATE_LABEL: Record<ProviderState, string> = {
  disconnected: "Не подключён",
  connecting: "Подключение…",
  connected: "Подключён",
  error: "Ошибка",
  reauth_required: "Нужен повторный вход",
};

export function kindFromName(name: string): FileKind {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (ext === "pdf") return "pdf";
  if (["doc", "docx", "rtf", "odt", "pages"].includes(ext)) return "doc";
  if (["xls", "xlsx", "csv", "numbers"].includes(ext)) return "sheet";
  if (["ppt", "pptx", "key"].includes(ext)) return "slides";
  if (["png", "jpg", "jpeg", "heic", "webp"].includes(ext)) return "image";
  if (ext === "ipynb") return "notebook";
  if (["md", "txt", "tex"].includes(ext)) return "text";
  if (["zip", "rar", "7z"].includes(ext)) return "archive";
  return "other";
}

export const WORKFLOW_STAGE_LABEL: Record<HomeworkWorkflowStage, string> = {
  materials_collected: "Материалы собраны",
  codex_analysis: "Анализ материалов",
  prompt_ready: "Промт готов",
  claude_solving: "Создаётся решение",
  codex_review: "Проверка версии",
  needs_fixes: "Требуются исправления",
  teacher_review: "Комментарии преподавателя",
  ready_to_submit: "Готово к сдаче",
  submitted: "Отправлено",
};

export const ROLE_LABEL: Record<WorkflowRole, string> = { analyst: "Анализ материалов", author: "Решение", reviewer: "Проверка" };

export const SOURCE_KIND_LABEL: Record<SourceKind, string> = {
  lecture: "Лекции",
  seminar: "Семинары",
  task: "Условие",
  data: "Данные",
  extra: "Дополнительные материалы",
  style: "Мой стиль",
  previous_work: "Предыдущие работы",
  current_version: "Текущая версия",
  previous_version: "Предыдущая версия",
  teacher_comments: "Комментарии преподавателя",
};

export const RUN_KIND_LABEL: Record<RunKind, string> = {
  analyze_materials: "Проанализировать материалы и создать промт",
  review_version: "Проверить текущую версию",
  revision_prompt: "Создать промт исправлений",
  review_teacher: "Проверить комментарии преподавателя",
};

export const CRITERION_STATUS_LABEL: Record<CriterionStatus, string> = { pass: "в порядке", warn: "есть замечания", fail: "не пройдено", manual: "нужна ручная проверка", na: "неприменимо" };

export const REQUIREMENT_STATUS_LABEL: Record<RequirementStatus, string> = { done: "Выполнено", not_done: "Не выполнено", manual: "Требует ручной проверки", na: "Неприменимо" };

export const STYLE_REASON_LABEL: Record<StyleReasonKind, string> = {
  template: "Слишком шаблонные формулировки",
  uncharacteristic_words: "Нехарактерные слова",
  formal_tone: "Слишком формальный тон",
  repetition: "Повторяющиеся конструкции",
  unsupported: "Утверждения без ссылок на материалы",
};

export const COMMENT_SOURCE_LABEL: Record<TeacherCommentSource, string> = { chat: "Чат", email: "Почта", lms: "Система вуза", in_person: "Устно", other: "Другое" };

/** Этап 7: кто дал ответ. */
export const COMMENT_AUTHORITY_LABEL: Record<CommentAuthority, string> = {
  teacher_official: "Преподаватель",
  teaching_assistant: "Ассистент",
  student: "Студент",
  group_chat_unverified: "Общий чат, не проверено",
  user_note: "Моя заметка",
  system: "Система",
  unknown: "Автор неизвестен",
};

export const authorName = (a: VersionAuthor): string => (a === "user" ? "Вы" : providerName(a));

/** Сводное состояние провайдера (этап 9). */
export const AVAILABILITY_LABEL: Record<"connected" | "manual" | "unavailable" | "error", string> = {
  connected: "CLI подключён",
  manual: "ручная передача",
  unavailable: "CLI недоступен",
  error: "ошибка CLI",
};

/**
 * Этап 12: честный режим аккаунта провайдера. «Не авторизован» — только когда
 * CLI сам сообщил, что нужен вход; отсутствие CLI и ошибки проверки — отдельные
 * состояния, а не «нет авторизации».
 */
export function providerAccountMode(status: { availability: "connected" | "manual" | "unavailable" | "error"; state: ProviderState }): { label: string; tone: "ok" | "info" | "warn" | "danger" } {
  if (status.availability === "connected" && status.state === "connected") return { label: "подписка аккаунта", tone: "ok" };
  if (status.state === "reauth_required") return { label: "не авторизован", tone: "warn" };
  if (status.availability === "manual") return { label: "ручная передача", tone: "info" };
  if (status.availability === "unavailable") return { label: "CLI недоступен — ручная передача", tone: "info" };
  return { label: "ошибка проверки CLI", tone: "danger" };
}

export const PARSE_STATUS_LABEL: Record<"valid" | "partial" | "corrupt" | "empty", string> = {
  valid: "ответ распознан полностью",
  partial: "ответ распознан частично",
  corrupt: "ответ не распознан",
  empty: "пустой ответ",
};

export const JOB_STATUS_LABEL: Record<"prepared" | "running" | "response_ready" | "failed" | "cancelled" | "confirmed" | "discarded", string> = {
  prepared: "передача подготовлена",
  running: "выполняется",
  response_ready: "ответ ждёт проверки",
  failed: "ошибка",
  cancelled: "остановлено",
  confirmed: "подтверждено",
  discarded: "отклонено",
};

/** «1:05» из миллисекунд. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
