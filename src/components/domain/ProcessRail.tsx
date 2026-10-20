import { motion } from "motion/react";

export const PROCESS_STAGES = ["Материалы", "Анализ", "Промт", "Решение", "Проверка", "Исправления", "Преподаватель", "К сдаче", "Отправлено"] as const;

interface Props {
  /** Значения под каждым этапом (например, сколько ДЗ на этапе). */
  counts?: number[];
  /** Индекс этапа, до которого рельс «заполнен». */
  reached?: number;
  /** intro — рисуется из одной точки, по шагам. */
  variant?: "intro" | "static";
  layoutId?: string;
  onStageClick?: (index: number) => void;
}

const X = PROCESS_STAGES.map((_, i) => ((i + 0.5) / PROCESS_STAGES.length) * 100);

/** Линия учебного процесса. В intro-режиме — из точки в линию, затем узлы по порядку. */
export function ProcessRail({ counts, reached = -1, variant = "static", layoutId, onStageClick }: Props) {
  const intro = variant === "intro";
  const fillTo = reached < 0 ? 0 : X[Math.min(reached, X.length - 1)]!;
  return (
    <motion.div layoutId={layoutId} className={`rail rail--${variant}`} transition={{ type: "spring", stiffness: 260, damping: 34 }}>
      <svg className="rail__svg" viewBox="0 0 100 10" preserveAspectRatio="none" aria-hidden>
        <motion.path
          d={`M${X[0]} 5 H${X[X.length - 1]}`}
          className="rail__track"
          initial={intro ? { pathLength: 0, opacity: 0 } : false}
          animate={{ pathLength: 1, opacity: 1 }}
          transition={intro ? { delay: 0.55, duration: 0.7, ease: [0.6, 0, 0.2, 1] } : { duration: 0 }}
        />
        {reached >= 0 && (
          <motion.path
            key={fillTo}
            d={`M10 5 H${fillTo}`}
            className="rail__fill"
            initial={{ pathLength: 0 }}
            animate={{ pathLength: 1 }}
            transition={{ delay: intro ? 1.3 : 0.15, duration: 0.6, ease: [0.2, 0.8, 0.2, 1] }}
          />
        )}
      </svg>
      {intro && (
        // Стартовая точка: появляется в центре и «растягивается» в линию.
        <motion.span
          className="rail__seed"
          initial={{ scale: 0, opacity: 0, left: "50%" }}
          animate={{ scale: [0, 1.25, 1, 1], opacity: [0, 1, 1, 0], left: ["50%", "50%", "50%", `${X[0]}%`] }}
          transition={{ duration: 1.0, times: [0, 0.3, 0.55, 1], ease: "easeOut" }}
        />
      )}
      <ol className="rail__nodes">
        {PROCESS_STAGES.map((label, i) => {
          const done = i <= reached;
          const content = (
            <>
              <motion.span
                className="rail__node"
                data-done={done}
                initial={intro ? { scale: 0, opacity: 0 } : false}
                animate={{ scale: 1, opacity: 1 }}
                transition={intro ? { delay: 0.85 + i * 0.09, type: "spring", stiffness: 520, damping: 26 } : { duration: 0 }}
              />
              <motion.span
                className="rail__label"
                initial={intro ? { opacity: 0, y: 4 } : false}
                animate={{ opacity: 1, y: 0 }}
                transition={intro ? { delay: 1.0 + i * 0.09, duration: 0.26 } : { duration: 0 }}
              >
                {label}
                {counts && <span className="rail__count num">{counts[i] ?? 0}</span>}
              </motion.span>
            </>
          );
          return (
            <li key={label} className="rail__stage">
              {onStageClick ? (
                <button className="rail__btn" onClick={() => onStageClick(i)}>
                  {content}
                </button>
              ) : (
                content
              )}
            </li>
          );
        })}
      </ol>
    </motion.div>
  );
}
