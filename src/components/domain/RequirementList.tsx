import { motion } from "motion/react";
import type { RequirementStatus, TeacherComment, TeacherRequirement } from "../../lib/bridge";
import { COMMENT_SOURCE_LABEL, formatDate, REQUIREMENT_STATUS_LABEL } from "../../lib/format";

const TONE: Record<RequirementStatus, string> = { done: "ok", not_done: "danger", manual: "warn", na: "" };

interface Props {
  requirements: TeacherRequirement[];
  comments: TeacherComment[];
  onResolve?: (r: TeacherRequirement, status: "done" | "not_done" | "na") => void;
  locked?: boolean;
}

/** Чек-лист требований преподавателя; у каждого — связь с исходным вопросом/ответом. */
export function RequirementList({ requirements, comments, onResolve, locked }: Props) {
  return (
    <ul className="reqs">
      {requirements.map((r, i) => {
        const c = comments.find((x) => x.id === r.commentId);
        return (
          <motion.li key={r.id} className="req" data-status={r.status} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: i * 0.04 }}>
            <span className={`req__mark req__mark--${r.status}`} aria-hidden />
            <div className="req__main">
              <div className="req__text">{r.text}</div>
              <div className="req__origin faint">
                {c ? `${COMMENT_SOURCE_LABEL[c.source]}, ${formatDate(c.date)} · ` : ""}из: «{r.origin.replace(/…$/, "")}…»
              </div>
              {r.note && <div className="req__note faint">{r.note}</div>}
            </div>
            <span className={`badge ${TONE[r.status] ? `badge--${TONE[r.status]}` : ""}`}>{REQUIREMENT_STATUS_LABEL[r.status]}</span>
            {onResolve && r.status === "manual" && !locked && (
              <div className="req__resolve">
                <button className="btn btn--sm" onClick={() => onResolve(r, "done")}>
                  Выполнено
                </button>
                <button className="btn btn--sm btn--ghost" onClick={() => onResolve(r, "not_done")}>
                  Нет
                </button>
                <button className="btn btn--sm btn--ghost" onClick={() => onResolve(r, "na")}>
                  Неприменимо
                </button>
              </div>
            )}
          </motion.li>
        );
      })}
    </ul>
  );
}
