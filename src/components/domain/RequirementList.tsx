import { motion } from "motion/react";
import type { RequirementStatus, TeacherComment, TeacherRequirement } from "../../lib/bridge";
import { COMMENT_SOURCE_LABEL, formatDate, REQUIREMENT_STATUS_LABEL } from "../../lib/format";

const TONE: Record<RequirementStatus, string> = { done: "ok", not_done: "danger", manual: "warn", na: "" };

interface Props {
  requirements: TeacherRequirement[];
  comments: TeacherComment[];
  onResolve?: (r: TeacherRequirement, status: "done" | "not_done" | "na") => void;
  locked?: boolean;
  /** Статусы, которые можно закрыть вручную. По умолчанию — только «требует ручной проверки». */
  resolvable?: RequirementStatus[];
  /** Этап 7: удалить требование (остаётся в истории как отозванное). */
  onWithdraw?: (r: TeacherRequirement) => void;
}

/** Чек-лист требований преподавателя; у каждого — связь с исходным вопросом/ответом. */
export function RequirementList({ requirements, comments, onResolve, locked, resolvable = ["manual"], onWithdraw }: Props) {
  return (
    <ul className="reqs">
      {requirements.map((r, i) => {
        const c = comments.find((x) => x.id === r.commentId);
        return (
          <motion.li key={r.id} className="req" data-status={r.status} data-withdrawn={r.withdrawnAt ? "" : undefined} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: i * 0.04 }}>
            <span className={`req__mark req__mark--${r.status}`} aria-hidden />
            <div className="req__main">
              <div className="req__text">{r.text}</div>
              <div className="req__origin faint">
                {c ? `${COMMENT_SOURCE_LABEL[c.source]}, ${formatDate(c.date)} · ` : ""}из: «{r.origin.replace(/…$/, "")}…»
              </div>
              {r.note && <div className="req__note faint">{r.note}</div>}
              {r.withdrawnAt && <div className="req__note faint">Отозвано {formatDate(r.withdrawnAt)} — хранится для истории и не учитывается</div>}
              {c && !c.official && !r.withdrawnAt && <div className="req__note faint">Источник не подтверждён как официальный — проверьте его</div>}
            </div>
            <span className={`badge ${r.withdrawnAt ? "" : TONE[r.status] ? `badge--${TONE[r.status]}` : ""}`}>{r.withdrawnAt ? "Отозвано" : REQUIREMENT_STATUS_LABEL[r.status]}</span>
            {onWithdraw && !r.withdrawnAt && !locked && (
              <button className="btn btn--sm btn--ghost" onClick={() => onWithdraw(r)}>
                Удалить
              </button>
            )}
            {onResolve && !r.withdrawnAt && resolvable.includes(r.status) && !locked && (
              <div className="req__resolve">
                <button className="btn btn--sm" onClick={() => onResolve(r, "done")}>
                  Выполнено
                </button>
                {r.status !== "not_done" && (
                  <button className="btn btn--sm btn--ghost" onClick={() => onResolve(r, "not_done")}>
                    Нет
                  </button>
                )}
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
