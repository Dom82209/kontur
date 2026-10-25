import { ClipboardPen, Send } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, TASK_FOR_RUN, type AiDraft, type AiJob, type AiTask, type AutomationRules, type ProviderStatus, type ReviewRun } from "../../lib/bridge";
import { providerName } from "../../lib/format";
import type { useWorkflowActions } from "../../lib/hooks/useWorkflowActions";
import { AiHandoffDialog } from "./AiHandoffDialog";
import { ManualResultDialog } from "./ManualResultDialog";

type Actions = ReturnType<typeof useWorkflowActions>;

interface Props {
  run: ReviewRun;
  /** Задача; по умолчанию — основная задача запуска (анализ, проверка, преподаватель). */
  task?: AiTask;
  jobs: AiJob[];
  rules: AutomationRules | null;
  providers: ProviderStatus[] | null;
  actions: Actions;
  /** Результат сохранён (вручную или подтверждением ответа провайдера). */
  onSaved?: (run: ReviewRun) => void;
  /** Только кнопка передачи (без ручного ввода) — для промта исправлений. */
  handoffOnly?: boolean;
  label?: string;
}

/**
 * Шаг рабочего цикла, ожидающий результата: передать провайдеру (официальный CLI
 * после явного подключения или ручная передача) либо ввести результат вручную.
 * Ответ провайдера применяется только после проверки и подтверждения в форме.
 */
export function AiStepActions({ run, task, jobs, rules, providers, actions, onSaved, handoffOnly, label }: Props) {
  const { reportError } = useOps();
  const [handoff, setHandoff] = useState(false);
  const [currentTask, setCurrentTask] = useState<AiTask>(task ?? TASK_FOR_RUN[run.kind]);
  const [manual, setManual] = useState(false);
  const [review, setReview] = useState<{ job: AiJob; supporting: AiJob | null; draft: AiDraft } | null>(null);

  const mainTask = task ?? TASK_FOR_RUN[run.kind];
  const provider = rules?.workflow.tasks[mainTask];
  const connected = providers?.find((p) => p.provider === provider)?.availability === "connected";

  const openReview = async (job: AiJob, supporting: AiJob | null) => {
    try {
      const [full, extra] = await Promise.all([bridge.getAiJob(job.id), supporting ? bridge.getAiJob(supporting.id) : Promise.resolve(null)]);
      const draft: AiDraft = { ...(full.draft ?? {}) };
      if (extra?.draft?.promptText) draft.promptText = extra.draft.promptText;
      setReview({ job: full, supporting: extra, draft });
      setHandoff(false);
    } catch (e) {
      reportError(e, "Ответ не открыт");
    }
  };

  const closeAll = (saved: ReviewRun | null) => {
    setReview(null);
    setManual(false);
    if (saved) onSaved?.(saved);
  };

  return (
    <>
      <button className="btn btn--sm btn--primary" disabled={!!actions.busy} onClick={() => setHandoff(true)}>
        <Send /> {label ?? `Передать ${provider ? providerName(provider) : "провайдеру"}`}
        <span className="faint">{connected ? " · CLI" : " · вручную"}</span>
      </button>
      {!handoffOnly && (
        <button className="btn btn--sm" disabled={!!actions.busy} onClick={() => setManual(true)}>
          <ClipboardPen /> Ввести результат вручную
        </button>
      )}

      <AiHandoffDialog
        open={handoff}
        onClose={() => setHandoff(false)}
        run={run}
        task={currentTask}
        onTaskChange={run.kind === "analyze_materials" ? setCurrentTask : undefined}
        jobs={jobs}
        rules={rules}
        providers={providers}
        actions={actions}
        onReview={(job, supporting) => void openReview(job, supporting)}
        onConfirmed={() => onSaved?.(run)}
      />

      <ManualResultDialog
        run={manual || review ? run : null}
        busy={actions.busy === "manual" || actions.busy === "ai-confirm"}
        initial={review?.draft ?? null}
        confirming={review ? { job: review.job, supporting: review.supporting } : null}
        onClose={() => closeAll(null)}
        onAnalysis={(input) => {
          if (review) {
            void actions.confirmAi(review.job.id, review.job.task, { kind: "analysis", value: input }, review.supporting?.id ?? null).then((r) => r && closeAll(r.run));
          } else void actions.saveAnalysis(run.id, input).then((r) => r && closeAll(r));
        }}
        onReview={(input) => {
          if (review) {
            void actions.confirmAi(review.job.id, review.job.task, { kind: "review", value: input }).then((r) => r && closeAll(r.run));
          } else void actions.saveReview(run.id, input).then((r) => r && closeAll(r));
        }}
      />
    </>
  );
}
