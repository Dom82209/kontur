import { bridge, type BridgeEventName, type HomeworkWorkflow } from "../bridge";
import { useBridgeQuery, type QueryState } from "./useBridgeQuery";

const EVENTS: BridgeEventName[] = [
  "operation:completed",
  "operation:failed",
  "review:completed",
  "review:failed",
  "review:progress",
  "workflow:stage-changed",
  "homework:version-created",
  "homework:updated",
  "homework:readiness-updated",
  "homework:revision-prompt-created",
  "submission:confirmed",
  "ai-job:updated",
];

/**
 * Все данные рабочего цикла одного ДЗ — один запрос агрегата (`getWorkflow`).
 * Перечитывается по событиям backend, поэтому новый промт, версия или проверка
 * появляются без перезагрузки приложения.
 */
export function useHomeworkWorkflow(homeworkId: string) {
  const workflow = useBridgeQuery(() => bridge.getWorkflow(homeworkId), [homeworkId], EVENTS);
  const part = <T,>(pick: (value: HomeworkWorkflow) => T): QueryState<T> => ({
    data: workflow.data ? pick(workflow.data) : null,
    error: workflow.error,
    errorCode: workflow.errorCode,
    loading: workflow.loading,
    reload: workflow.reload,
    setData: () => undefined,
  });
  return {
    workflow,
    homework: part((w) => w.homework),
    iterations: part((w) => w.iterations),
    runs: part((w) => w.runs),
    prompts: part((w) => [...w.prompts].sort((a, b) => a.createdAt.localeCompare(b.createdAt))),
    comments: part((w) => w.teacherComments),
    requirements: part((w) => w.teacherRequirements),
    readiness: part((w) => w.readiness),
    pkg: part((w) => w.submissionPackage),
    receipt: part((w) => w.submissionReceipt),
    revision: workflow.data?.revision,
    /** Задания передачи провайдерам (этап 9), без текстов. */
    aiJobs: part((w) => w.aiJobs),
    reloadAll: () => workflow.reload(),
  };
}

export type HomeworkWorkflowData = ReturnType<typeof useHomeworkWorkflow>;
