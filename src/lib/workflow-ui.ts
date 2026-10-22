import type { HomeworkWorkflowStage } from "./bridge";

export type HomeworkTab = "materials" | "cycle" | "review" | "teacher" | "package";

export interface WorkflowPresentation {
  tab: HomeworkTab;
  action: string;
}

/** Единое правило навигации для Главной и карточки ДЗ. */
export function getWorkflowPresentation(stage: HomeworkWorkflowStage, readinessReady = false): WorkflowPresentation {
  switch (stage) {
    case "materials_collected":
      return { tab: "materials", action: "Проверить материалы и запустить анализ" };
    case "codex_analysis":
      return { tab: "materials", action: "Дождаться результата анализа" };
    case "prompt_ready":
      return { tab: "cycle", action: "Передать промт автору" };
    case "claude_solving":
      return { tab: "cycle", action: "Дождаться новой версии" };
    case "codex_review":
      return { tab: "review", action: "Проверить последнюю версию" };
    case "needs_fixes":
      return { tab: "review", action: "Подготовить промт исправлений" };
    case "teacher_review":
      return readinessReady
        ? { tab: "package", action: "Собрать комплект на сдачу" }
        : { tab: "teacher", action: "Сверить комментарии преподавателя" };
    case "ready_to_submit":
      return { tab: "package", action: "Отправить вручную и подтвердить" };
    case "submitted":
      return { tab: "package", action: "Открыть неизменяемый снимок" };
  }
}

/**
 * Конфликт устаревшего состояния: другая команда успела изменить ДЗ. UI в этом
 * случае перечитывает агрегат и предлагает повторить действие, а не показывает
 * его как обычную ошибку.
 */
export function isStaleConflict(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const candidate = error as { code?: unknown; message?: unknown };
  return candidate.code === "conflict" && typeof candidate.message === "string" && /перечитайте|изменил/i.test(candidate.message);
}
