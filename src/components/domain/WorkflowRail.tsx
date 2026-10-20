import { motion } from "motion/react";
import { Check } from "lucide-react";
import { WORKFLOW_STAGE_ORDER, type HomeworkWorkflowStage } from "../../lib/bridge";
import { WORKFLOW_STAGE_LABEL } from "../../lib/format";

interface Props {
  homeworkId: string;
  stage: HomeworkWorkflowStage;
  /** Номер текущего цикла исправлений (1 — первая версия). */
  cycle?: number;
  compact?: boolean;
}

const N = WORKFLOW_STAGE_ORDER.length;
const x = (i: number) => (i / (N - 1)) * 100;
const LOOP_FROM = WORKFLOW_STAGE_ORDER.indexOf("needs_fixes");
const LOOP_TO = WORKFLOW_STAGE_ORDER.indexOf("prompt_ready");

/**
 * Девять этапов рабочего цикла. Петля «Требуются исправления → Промт готов»
 * подсвечивается, когда работа идёт на повторный круг; маркер этапа перетекает (layoutId).
 */
export function WorkflowRail({ homeworkId, stage, cycle = 1, compact }: Props) {
  const index = WORKFLOW_STAGE_ORDER.indexOf(stage);
  const looping = cycle > 1 && index >= LOOP_TO && index <= LOOP_FROM;
  return (
    <div className={`wfrail${compact ? " wfrail--compact" : ""}`} role="img" aria-label={`Этап ${index + 1} из ${N}: ${WORKFLOW_STAGE_LABEL[stage]}${cycle > 1 ? `, цикл исправлений ${cycle}` : ""}`}>
      {!compact && (
        <svg className="wfrail__loop" viewBox="0 0 100 12" preserveAspectRatio="none" aria-hidden>
          <motion.path
            d={`M ${x(LOOP_FROM)} 12 C ${x(LOOP_FROM)} 1, ${x(LOOP_TO)} 1, ${x(LOOP_TO)} 12`}
            className="wfrail__looppath"
            data-active={looping}
            initial={false}
            animate={{ pathLength: looping ? 1 : 0.999, opacity: looping ? 1 : 0.5 }}
            transition={{ duration: 0.5 }}
          />
        </svg>
      )}
      <div className="wfrail__line">
        <motion.div className="wfrail__fill" initial={false} animate={{ scaleX: index / (N - 1) }} transition={{ type: "spring", stiffness: 160, damping: 26 }} />
      </div>
      {WORKFLOW_STAGE_ORDER.map((s, i) => {
        const done = i < index || stage === "submitted";
        const current = i === index;
        return (
          <div key={s} className="wfrail__stage" data-done={done} data-current={current} data-loop={i === LOOP_FROM || i === LOOP_TO} style={{ left: `${x(i)}%` }}>
            <span className="wfrail__node">
              {current && <motion.span layoutId={`wf-marker-${homeworkId}`} className="wfrail__marker" transition={{ type: "spring", stiffness: 280, damping: 30 }} />}
              {done && !compact && <Check className="wfrail__check" />}
            </span>
            {!compact && (
              <span className="wfrail__label">
                <span className="wfrail__n num">{i + 1}</span>
                {WORKFLOW_STAGE_LABEL[s]}
              </span>
            )}
          </div>
        );
      })}
      {!compact && cycle > 1 && <span className="wfrail__cycle badge badge--accent">цикл {cycle}</span>}
    </div>
  );
}
