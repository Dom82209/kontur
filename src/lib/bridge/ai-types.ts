/**
 * Этап 9: задания передачи материалов провайдерам AI.
 *
 * Бизнес-логика опирается на задачи (`AiTask`), а не на исторические имена
 * этапов `codex_analysis`/`claude_solving`: провайдер каждой задачи
 * назначается отдельно в настройках рабочего цикла.
 *
 * Ответ провайдера ничего не меняет сам: он разбирается в черновик и
 * применяется только после проверки и подтверждения пользователем.
 */
import type { ISODate, ProviderId } from "./types";
import type { ManualAnalysisInput, ManualReviewInput, ReviewRun, RevisionPrompt } from "./workflow-types";

export type AiTask = "material_analysis" | "initial_prompt" | "solution_review" | "revision_prompt" | "teacher_review";

export const AI_TASKS: readonly AiTask[] = ["material_analysis", "initial_prompt", "solution_review", "revision_prompt", "teacher_review"];

export const AI_TASK_LABEL: Record<AiTask, string> = {
  material_analysis: "Анализ материалов",
  initial_prompt: "Первичный промт",
  solution_review: "Проверка решения",
  revision_prompt: "Промт исправлений",
  teacher_review: "Проверка по комментариям преподавателя",
};

/** Официальный CLI (после явного подключения) или ручная передача через буфер обмена. */
export type AiTransport = "cli" | "manual_handoff";
/** auto — CLI, если провайдер подключён, иначе ручная передача. */
export type TransportPreference = "auto" | AiTransport;

export type AiJobStatus = "prepared" | "running" | "response_ready" | "failed" | "cancelled" | "confirmed" | "discarded";
export type AiParseStatus = "valid" | "partial" | "corrupt" | "empty";
export type AiResponseSource = "cli" | "paste" | "file";

/** Сводное состояние провайдера. */
export type ProviderAvailability = "connected" | "manual" | "unavailable" | "error";

/** Источник, вошедший в передаваемый текст. Путей нет — только имя и объём. */
export interface AiContextSource {
  label: string;
  sourceId: string | null;
  name: string;
  kind: string;
  sha256: string | null;
  charsTotal: number;
  charsIncluded: number;
  truncated: boolean;
}

export interface AiIssue {
  code: string;
  message: string;
}

export interface AiLogEntry {
  at: ISODate;
  level: "info" | "warn" | "error";
  /** Без секретов, адресов почты и абсолютных путей (редакция в backend). */
  message: string;
}

/** Черновик ответа — те же поля, что у ручного ввода; любые могут отсутствовать. */
export type AiDraft = Partial<ManualAnalysisInput> & Partial<ManualReviewInput> & { text?: string };

export interface AiJob {
  id: string;
  homeworkId: string;
  runId: string;
  task: AiTask;
  provider: ProviderId;
  transport: AiTransport;
  adapterLabel: string | null;
  status: AiJobStatus;
  /** Полный текст передачи. В списках (агрегат ДЗ) — null. */
  promptText: string | null;
  promptChars: number;
  promptSha256: string;
  context: AiContextSource[];
  contextTruncated: boolean;
  limitChars: number;
  /** Ответ провайдера. В списках — null. */
  responseText: string | null;
  responseChars: number;
  responseSource: AiResponseSource | null;
  /** Только имя импортированного файла, без пути. */
  responseFileName: string | null;
  responseTruncated: boolean;
  parseStatus: AiParseStatus | null;
  /** Черновик результата. В списках — null. */
  draft: AiDraft | null;
  issues: AiIssue[];
  log: AiLogEntry[];
  errorCode: string | null;
  errorMessage: string | null;
  exitCode: number | null;
  durationMs: number | null;
  createdAt: ISODate;
  sentAt: ISODate | null;
  finishedAt: ISODate | null;
  confirmedAt: ISODate | null;
  /** Пользователь поправил черновик перед подтверждением. */
  edited: boolean | null;
  promptId: string | null;
}

/** Что подтверждает пользователь: те же поля, что у ручного ввода. */
export type AiConfirmInput =
  | { kind: "analysis"; value: ManualAnalysisInput }
  | { kind: "review"; value: ManualReviewInput }
  | { kind: "prompt"; value: { text: string } };

export interface AiConfirmation {
  job: AiJob;
  run: ReviewRun;
  prompt: RevisionPrompt | null;
}

/** Состояние официального CLI. Абсолютных путей и учётных данных нет. */
export interface ProviderCli {
  /** Пользователь явно подключил CLI. */
  enabled: boolean;
  detected: boolean;
  executable: string | null;
  version: string | null;
  supported: boolean | null;
  missingFlags: string[];
  loggedIn: boolean | null;
  checkedAt: ISODate | null;
}

export interface ProviderHandoff {
  appInstalled: boolean;
  webUrl: string;
}

export const AI_JOB_OPEN: readonly AiJobStatus[] = ["prepared", "running", "response_ready"];
export const isOpenJob = (job: Pick<AiJob, "status">): boolean => AI_JOB_OPEN.includes(job.status);

/** Задача, ответ на которую подтверждается для запуска данного вида. */
export const TASK_FOR_RUN: Record<ReviewRun["kind"], AiTask> = {
  analyze_materials: "material_analysis",
  review_version: "solution_review",
  revision_prompt: "solution_review",
  review_teacher: "teacher_review",
};
