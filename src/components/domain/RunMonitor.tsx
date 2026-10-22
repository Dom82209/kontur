import { AnimatePresence, motion } from "motion/react";
import { X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useOps } from "../../app/operations";
import { bridge, type ReviewRun, type ReviewRunStatus } from "../../lib/bridge";
import { providerName, ROLE_LABEL } from "../../lib/format";
import { useBridgeEvent } from "../../lib/hooks/useBridgeEvent";
import { AiWaveform } from "./AiWaveform";

const STATUS_TEXT: Record<ReviewRunStatus, string> = {
  queued: "В очереди",
  waiting_for_provider: "Ждёт провайдера",
  manual_handoff: "Выполняется вручную",
  running: "Выполняется",
  completed: "Завершено",
  failed: "Ошибка",
  cancelled: "Отменено",
};

/** Открытый (незавершённый) запуск: в очереди, выполняется или ждёт ручного результата. */
export const isOpenRun = (run: ReviewRun): boolean =>
  run.status === "queued" || run.status === "running" || run.status === "waiting_for_provider" || run.status === "manual_handoff";

interface Props {
  run: ReviewRun | null;
  /** Действия ручного режима: ввести результат, открыть приложение и т. п. */
  handoff?: ReactNode;
  /** Своя отмена (с expectedRevision); без неё — прямая отмена через bridge. */
  onCancel?: (runId: string) => void;
}

/**
 * Активный запуск: спокойная волна, текущий шаг, отмена. Прогресс — из события
 * review:progress. Без AI-адаптера запуск честно показывает «ждёт провайдера» или
 * «выполняется вручную» и не изображает прогресс AI.
 */
export function RunMonitor({ run, handoff, onCancel }: Props) {
  const { reportError } = useOps();
  const [live, setLive] = useState<{ id: string; progress: number; step: string } | null>(null);
  useBridgeEvent("review:progress", (p) => {
    if (run && p.runId === run.id) setLive({ id: p.runId, progress: p.progress, step: p.step });
  });
  const open = run && isOpenRun(run) ? run : null;
  const waiting = run?.status === "waiting_for_provider" || run?.status === "manual_handoff";
  const progress = live && run && live.id === run.id ? live.progress : (run?.progress ?? 0);
  const step = live && run && live.id === run.id ? live.step : (run?.step ?? "");
  const cancel = (id: string) => (onCancel ? onCancel(id) : void bridge.cancelRun(id).catch((e: unknown) => reportError(e, "Отмена не выполнена")));
  return (
    <AnimatePresence>
      {run && (
        <motion.div className="runmon" data-status={run.status} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -4 }}>
          <div className="runmon__head">
            <span className={`dot ${waiting ? "dot--warn" : "dot--accent"}`} />
            <span className="runmon__provider">
              {waiting ? "Вручную" : providerName(run.provider)} · {ROLE_LABEL[run.role].toLowerCase()}
            </span>
            <span className="faint runmon__meta">
              {step || STATUS_TEXT[run.status]}
              {!waiting && (
                <>
                  {" "}
                  · <span className="num">{Math.round(progress * 100)}%</span>
                </>
              )}
            </span>
            {open && (
              <button className="btn btn--sm btn--ghost" onClick={() => cancel(open.id)}>
                <X /> Отменить
              </button>
            )}
          </div>
          {open && !waiting && <AiWaveform active progress={progress} label="Прогресс запуска" />}
          {waiting && (
            <div className="runmon__handoff">
              <p>
                {run.status === "waiting_for_provider"
                  ? `${providerName(run.provider)} недоступен: AI-адаптер ещё не подключён. Выполните шаг сами и сохраните результат вручную.`
                  : "Шаг передан на ручное выполнение. Перенесите итог — он сохранится с отметкой «вручную»."}
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
