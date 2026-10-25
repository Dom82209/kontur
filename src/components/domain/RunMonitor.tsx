import { AnimatePresence, motion } from "motion/react";
import { Square, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useOps } from "../../app/operations";
import { AI_TASK_LABEL, bridge, type AiJob, type ReviewRun, type ReviewRunStatus } from "../../lib/bridge";
import { formatBytes, formatElapsed, JOB_STATUS_LABEL, PARSE_STATUS_LABEL, providerName, ROLE_LABEL } from "../../lib/format";
import { useBridgeEvent } from "../../lib/hooks/useBridgeEvent";
import { AiWaveform } from "./AiWaveform";

const STATUS_TEXT: Record<ReviewRunStatus, string> = {
  queued: "В очереди",
  waiting_for_provider: "Ждёт передачи провайдеру",
  manual_handoff: "Ручная передача",
  running: "Провайдер обрабатывает",
  completed: "Завершено",
  failed: "Ошибка",
  cancelled: "Отменено",
};

/** Открытый (незавершённый) запуск: в очереди, выполняется или ждёт результата. */
export const isOpenRun = (run: ReviewRun): boolean =>
  run.status === "queued" || run.status === "running" || run.status === "waiting_for_provider" || run.status === "manual_handoff";

interface Props {
  run: ReviewRun | null;
  /** Последнее задание передачи этого запуска (этап 9), без текстов. */
  job?: AiJob | null;
  /** Действия: передать провайдеру, ввести результат вручную и т. п. */
  handoff?: ReactNode;
  /** Своя отмена (с expectedRevision); без неё — прямая отмена через bridge. */
  onCancel?: (runId: string) => void;
  onCancelJob?: (jobId: string) => void;
}

/**
 * Активный запуск. Пока работает официальный CLI, показываются прошедшее время и
 * объём вывода (из `ai-job:progress`) — процентов нет, потому что CLI их не сообщает.
 * Ожидающий запуск честно говорит, что результата ещё нет.
 */
export function RunMonitor({ run, job, handoff, onCancel, onCancelJob }: Props) {
  const { reportError } = useOps();
  const [live, setLive] = useState<{ jobId: string; elapsedMs: number; outputBytes: number } | null>(null);
  useBridgeEvent("ai-job:progress", (p) => {
    if (run && p.runId === run.id) setLive({ jobId: p.jobId, elapsedMs: p.elapsedMs, outputBytes: p.outputBytes });
  });
  const open = run && isOpenRun(run) ? run : null;
  const jobRunning = job?.status === "running";
  const waiting = !jobRunning && (run?.status === "waiting_for_provider" || run?.status === "manual_handoff");
  const progress = live && job && live.jobId === job.id ? live : null;
  const cancel = (id: string) => (onCancel ? onCancel(id) : void bridge.cancelRun(id).catch((e: unknown) => reportError(e, "Отмена не выполнена")));
  const cancelJob = (id: string) => (onCancelJob ? onCancelJob(id) : void bridge.cancelAiJob(id).catch((e: unknown) => reportError(e, "Остановка не выполнена")));
  return (
    <AnimatePresence>
      {run && (
        <motion.div className="runmon" data-status={jobRunning ? "running" : run.status} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -4 }}>
          <div className="runmon__head">
            <span className={`dot ${waiting ? "dot--warn" : "dot--accent"}`} />
            <span className="runmon__provider">
              {providerName(run.provider)} · {ROLE_LABEL[run.role].toLowerCase()}
            </span>
            <span className="faint runmon__meta">
              {jobRunning ? (
                <>
                  {job.adapterLabel ?? "CLI"} · <span className="num">{formatElapsed(progress?.elapsedMs ?? 0)}</span>
                  {progress && progress.outputBytes > 0 && <> · вывод {formatBytes(progress.outputBytes)}</>}
                </>
              ) : (
                run.step || STATUS_TEXT[run.status]
              )}
            </span>
            {jobRunning ? (
              <button className="btn btn--sm btn--ghost" onClick={() => cancelJob(job.id)}>
                <Square /> Остановить
              </button>
            ) : (
              open && (
                <button className="btn btn--sm btn--ghost" onClick={() => cancel(open.id)}>
                  <X /> Отменить шаг
                </button>
              )
            )}
          </div>
          {jobRunning && <AiWaveform active progress={0.5} label="Официальный CLI обрабатывает материалы" />}
          {job && !jobRunning && job.status !== "confirmed" && job.status !== "discarded" && (
            <div className="runmon__job faint">
              {AI_TASK_LABEL[job.task]}: {JOB_STATUS_LABEL[job.status]}
              {job.parseStatus && <> · {PARSE_STATUS_LABEL[job.parseStatus]}</>}
              {job.errorMessage && <> · {job.errorMessage}</>}
            </div>
          )}
          {waiting && (
            <div className="runmon__handoff">
              <p>
                {run.status === "waiting_for_provider"
                  ? "Результата ещё нет. Передайте материалы провайдеру — вы увидите весь текст до отправки — или введите результат сами. Ничего не сохранится без вашего подтверждения."
                  : "Ручная передача: скопируйте промт, получите ответ в официальном приложении и вставьте его, — или введите результат сами."}
              </p>
              {handoff}
            </div>
          )}
          {run.errorMessage && (
            <div className="runmon__error" role="alert">
              {run.errorMessage}
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
