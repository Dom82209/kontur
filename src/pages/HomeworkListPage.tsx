import { LayoutGroup, motion } from "motion/react";
import { FileCheck2 } from "lucide-react";
import { Link } from "react-router-dom";
import { HomeworkRail } from "../components/domain/HomeworkRail";
import { EmptyState, ErrorState, SkeletonRows } from "../components/ui/States";
import { bridge, HOMEWORK_STAGE_ORDER } from "../lib/bridge";
import { compareDeadlines, formatDeadline, formatDateTime, STAGE_FOLDER, STAGE_LABEL, WORKFLOW_STAGE_LABEL } from "../lib/format";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
import { useSubjectNames } from "../lib/hooks/useSubjectNames";

/** Workflow-доска: карточка ДЗ перетекает между колонками при смене этапа (layoutId). */
export function HomeworkListPage() {
  const names = useSubjectNames();
  const q = useBridgeQuery(() => bridge.getHomeworkList(), [], ["operation:completed"]);

  return (
    <div className="hwlist">
      <header className="pagehead">
        <div>
          <span className="eyebrow">Домашние задания</span>
          <h1 className="pagehead__title">Условие → В работе → На сдачу → Отправлено</h1>
          <p className="muted">Колонки — папки ДЗ. Детальный этап рабочего цикла — на карточке; «Готово к сдаче» и «Отправлено» ставятся только вами.</p>
        </div>
      </header>
      {q.error && !q.data ? (
        <ErrorState message={q.error} onRetry={() => void q.reload()} />
      ) : !q.data ? (
        <SkeletonRows count={6} />
      ) : q.data.length === 0 ? (
        <EmptyState icon={<FileCheck2 />} title="Домашних заданий пока нет" text="Они появятся, когда во «Входящие» попадёт условие ДЗ." />
      ) : (
        <LayoutGroup id="hwboard">
          <div className="board">
            {HOMEWORK_STAGE_ORDER.map((stage, i) => {
              const items = q.data!.filter((h) => h.stage === stage).sort((a, b) => compareDeadlines(a.deadline, b.deadline));
              return (
                <section key={stage} className="board__col" aria-label={STAGE_LABEL[stage]}>
                  <div className="board__head">
                    <span className="board__idx num">0{i + 1}</span>
                    <span className="board__title">{STAGE_LABEL[stage]}</span>
                    <span className="board__count num">{items.length}</span>
                  </div>
                  <div className="board__folder mono">{STAGE_FOLDER[stage]}</div>
                  <ul className="board__list">
                    {items.length === 0 && <li className="board__empty">Пусто</li>}
                    {items.map((h) => {
                      const dl = formatDeadline(h.deadline);
                      return (
                        <motion.li key={h.id} layoutId={`hwcard-${h.id}`} className="hwcard" transition={{ type: "spring", stiffness: 300, damping: 32 }}>
                          <Link to={`/homework/${h.id}`} className="hwcard__link">
                            <div className="hwcard__subject">{names(h.subjectId)}</div>
                            <div className="hwcard__title">{h.title}</div>
                            <div className="hwcard__wf">{WORKFLOW_STAGE_LABEL[h.workflowStage]}</div>
                            <div className="hwcard__meta">
                              <span className="num">Нед. {h.weekNumber}</span>
                              <span>·</span>
                              <span>{h.versions.length ? `v${String(h.versions.length).padStart(2, "0")}` : "без версий"}</span>
                            </div>
                            <HomeworkRail homeworkId={h.id} stage={h.stage} compact />
                            <div className="hwcard__deadline" data-tone={h.stage === "submitted" ? "past" : dl.tone}>
                              {h.stage === "submitted" && h.submittedAt ? `Отправлено ${formatDateTime(h.submittedAt)}` : dl.text}
                            </div>
                          </Link>
                        </motion.li>
                      );
                    })}
                  </ul>
                </section>
              );
            })}
          </div>
        </LayoutGroup>
      )}
    </div>
  );
}
