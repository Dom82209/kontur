import { bridge, type BridgeEventName } from "../bridge";
import { useBridgeQuery } from "./useBridgeQuery";

const EVENTS: BridgeEventName[] = ["operation:completed", "review:completed", "review:failed", "workflow:stage-changed", "homework:version-created", "submission:confirmed"];

/** Все данные рабочего цикла одного ДЗ. Перезагружаются по событиям backend. */
export function useHomeworkWorkflow(homeworkId: string) {
  const homework = useBridgeQuery(() => bridge.getHomework(homeworkId), [homeworkId], EVENTS);
  const iterations = useBridgeQuery(() => bridge.getIterations(homeworkId), [homeworkId], EVENTS);
  const runs = useBridgeQuery(() => bridge.getReviewRuns(homeworkId), [homeworkId], [...EVENTS, "review:progress"]);
  const prompts = useBridgeQuery(() => bridge.getPrompts(homeworkId), [homeworkId], EVENTS);
  const comments = useBridgeQuery(() => bridge.getTeacherComments(homeworkId), [homeworkId], EVENTS);
  const requirements = useBridgeQuery(() => bridge.getTeacherRequirements(homeworkId), [homeworkId], EVENTS);
  const readiness = useBridgeQuery(() => bridge.getReadiness(homeworkId), [homeworkId], EVENTS);
  const pkg = useBridgeQuery(() => bridge.getSubmissionPackage(homeworkId), [homeworkId], EVENTS);
  const receipt = useBridgeQuery(() => bridge.getSubmissionReceipt(homeworkId), [homeworkId], EVENTS);
  const reloadAll = () => Promise.all([homework, iterations, runs, prompts, comments, requirements, readiness, pkg, receipt].map((q) => q.reload())).then(() => undefined);
  return { homework, iterations, runs, prompts, comments, requirements, readiness, pkg, receipt, reloadAll };
}

export type HomeworkWorkflowData = ReturnType<typeof useHomeworkWorkflow>;
