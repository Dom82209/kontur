import { motion } from "motion/react";
import { Check } from "lucide-react";
import { HOMEWORK_STAGE_ORDER, type HomeworkStage } from "../../lib/bridge";
import { STAGE_FOLDER, STAGE_LABEL } from "../../lib/format";

interface Props {
  homeworkId: string;
  stage: HomeworkStage;
  compact?: boolean;
}

/**
 * Рельс статуса ДЗ: «Условие → В работе → На сдачу → Отправлено».
 * При смене этапа рельс перестраивается: заполнение растягивается пружиной,
 * маркер текущего этапа перетекает к новому узлу (layoutId).
 */
export function HomeworkRail({ homeworkId, stage, compact }: Props) {
  const index = HOMEWORK_STAGE_ORDER.indexOf(stage);
  const last = HOMEWORK_STAGE_ORDER.length - 1;
  return (
    <div className={`hwrail${compact ? " hwrail--compact" : ""}`} role="img" aria-label={`Этап: ${STAGE_LABEL[stage]}, ${index + 1} из ${last + 1}`}>
      <div className="hwrail__line">
        <motion.div className="hwrail__fill" initial={false} animate={{ scaleX: index / last }} transition={{ type: "spring", stiffness: 180, damping: 26 }} />
      </div>
      {HOMEWORK_STAGE_ORDER.map((s, i) => {
        const done = i < index || stage === "submitted";
        const current = i === index;
        return (
          <div key={s} className="hwrail__stage" data-done={done} data-current={current} style={{ left: `${(i / last) * 100}%` }}>
            <span className="hwrail__node">
              {current && <motion.span layoutId={`hwrail-marker-${homeworkId}`} className="hwrail__marker" transition={{ type: "spring", stiffness: 300, damping: 30 }} />}
              {done && <Check className="hwrail__check" />}
            </span>
            {!compact && (
              <span className="hwrail__label">
                <span>{STAGE_LABEL[s]}</span>
                <span className="hwrail__folder mono">{STAGE_FOLDER[s]}</span>
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}
