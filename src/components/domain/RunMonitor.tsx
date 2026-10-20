import { AnimatePresence, motion } from "motion/react";
import { X } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type ReviewRun } from "../../lib/bridge";
import { providerName, ROLE_LABEL } from "../../lib/format";
import { useBridgeEvent } from "../../lib/hooks/useBridgeEvent";
import { AiWaveform } from "./AiWaveform";

/** Активный запуск: спокойная волна, текущий шаг, отмена. Прогресс — из события review:progress. */
export function RunMonitor({ run }: { run: ReviewRun | null }) {
  const { reportError } = useOps();
  const [live, setLive] = useState<{ id: string; progress: number; step: string } | null>(null);
  useBridgeEvent("review:progress", (p) => {
    if (run && p.runId === run.id) setLive({ id: p.runId, progress: p.progress, step: p.step });
  });
  const active = run && (run.status === "running" || run.status === "queued") ? run : null;
  const progress = live && run && live.id === run.id ? live.progress : (run?.progress ?? 0);
  const step = live && run && live.id === run.id ? live.step : (run?.step ?? "");
  return (
    <AnimatePresence>
      {run && (
        <motion.div className="runmon" data-status={run.status} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -4 }}>
          <div className="runmon__head">
            <span className="dot dot--accent" />
            <span className="runmon__provider">
              {providerName(run.provider)} · {ROLE_LABEL[run.role].toLowerCase()}
            </span>
            <span className="faint runmon__meta">
              {step || (run.status === "queued" ? "В очереди" : run.status === "completed" ? "Завершено" : run.status === "failed" ? "Ошибка" : "Отменено")} · <span className="num">{Math.round(progress * 100)}%</span>
            </span>
            {active && (
              <button className="btn btn--sm btn--ghost" onClick={() => void bridge.cancelRun(active.id).catch((e: unknown) => reportError(e, "Отмена не выполнена"))}>
                <X /> Отменить
              </button>
            )}
          </div>
          {active && <AiWaveform active progress={progress} label="Прогресс запуска" />}
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
