import type { HomeworkWorkflowStage } from "./workflow-types";

/**
 * Таблица допустимых переходов рабочего цикла — зеркало backend state machine
 * (`src-tauri/src/workflow/state_machine.rs`). В Tauri-режиме решение всегда
 * принимает backend (`HomeworkWorkflow.transitions`); mock использует таблицу,
 * чтобы поддерживать тот же контракт.
 */
export const WORKFLOW_TRANSITIONS: readonly (readonly [HomeworkWorkflowStage, HomeworkWorkflowStage])[] = [
  ["materials_collected", "codex_analysis"],
  ["codex_analysis", "materials_collected"],
  ["codex_analysis", "prompt_ready"],
  ["prompt_ready", "codex_analysis"],
  ["prompt_ready", "claude_solving"],
  ["claude_solving", "prompt_ready"],
  ["claude_solving", "codex_review"],
  ["codex_review", "claude_solving"],
  ["codex_review", "needs_fixes"],
  ["codex_review", "teacher_review"],
  ["codex_review", "ready_to_submit"],
  ["needs_fixes", "claude_solving"],
  ["teacher_review", "codex_review"],
  ["teacher_review", "needs_fixes"],
  ["teacher_review", "ready_to_submit"],
  ["ready_to_submit", "needs_fixes"],
  ["ready_to_submit", "teacher_review"],
  ["ready_to_submit", "submitted"],
];

export function isAllowedTransition(from: HomeworkWorkflowStage, to: HomeworkWorkflowStage): boolean {
  return WORKFLOW_TRANSITIONS.some(([a, b]) => a === from && b === to);
}

export function transitionsFrom(from: HomeworkWorkflowStage): HomeworkWorkflowStage[] {
  return WORKFLOW_TRANSITIONS.filter(([a]) => a === from).map(([, b]) => b);
}
