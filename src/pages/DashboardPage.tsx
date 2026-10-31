import { motion } from "motion/react";
import { ArrowRight, Clock, FolderSync, Inbox } from "lucide-react";
import { Link, useNavigate } from "react-router-dom";
import { useIntro } from "../app/intro";
import { ProcessRail } from "../components/domain/ProcessRail";
import { ProviderCard } from "../components/domain/ProviderCard";
import { Confidence } from "../components/ui/Confidence";
import { FileIcon } from "../components/ui/FileIcon";
import { ErrorState, Skeleton, SkeletonRows, EmptyState } from "../components/ui/States";
import { bridge, WORKFLOW_STAGE_ORDER, type Homework } from "../lib/bridge";
import { CATEGORY_LABEL, formatDate, formatDeadline, formatRelative, pad2, plural, WORKFLOW_STAGE_LABEL } from "../lib/format";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
import { useSubjectNames } from "../lib/hooks/useSubjectNames";
import { getWorkflowPresentation } from "../lib/workflow-ui";

const stagger = { animate: { transition: { staggerChildren: 0.045, delayChildren: 0.05 } } };
const rise = { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0, transition: { duration: 0.28, ease: [0.2, 0.8, 0.2, 1] as const } } };

function railCounts(homework: Homework[]): { counts: number[]; reached: number } {
  const counts = WORKFLOW_STAGE_ORDER.map((stage) => homework.filter((h) => h.workflowStage === stage).length);
  const reached = Math.max(0, ...homework.map((h) => WORKFLOW_STAGE_ORDER.indexOf(h.workflowStage)));
  return { counts, reached };
}

export function DashboardPage() {
  const { done: introDone } = useIntro();
  const navigate = useNavigate();
  const names = useSubjectNames();
  const q = useBridgeQuery(() => bridge.getDashboard(), [], ["operation:completed", "inbox:file-added", "provider:status-changed", "homework:updated", "workflow:stage-changed"]);
  const homework = useBridgeQuery(() => bridge.getHomeworkList(), [], ["operation:completed", "workflow:stage-changed", "homework:version-created"]);
  const d = q.data;

  if (q.error && !d) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;

  const next = d?.deadlines[0];
  const nextDl = next ? formatDeadline(next.deadline) : null;
  const allHomework = homework.data ?? [];
  const summary = {
    active: allHomework.filter((h) => h.workflowStage !== "ready_to_submit" && h.workflowStage !== "submitted").length,
    ready: allHomework.filter((h) => h.workflowStage === "ready_to_submit").length,
    submitted: allHomework.filter((h) => h.workflowStage === "submitted").length,
  };
  const rail = homework.data ? railCounts(homework.data) : null;

  return (
    <motion.div className="dash" variants={stagger} initial="initial" animate="animate">
      <motion.header className="dash__hero" variants={rise}>
        <div className="dash__week">
          <span className="eyebrow">Учебная неделя</span>
          <span className="dash__weeknum num">{d?.academicWeek ? pad2(d.academicWeek.number) : "··"}</span>
          {d && (
            <span className="faint">
              {d.academicWeek ? (
                `${formatDate(d.academicWeek.startsAt)} — ${formatDate(d.academicWeek.endsAt)}`
              ) : (
                <Link to="/settings">семестр не настроен</Link>
              )}
            </span>
          )}
        </div>
        <div className="dash__summary">
          {d ? (
            <>
              <h1 className="dash__headline">
                {summary.active} {plural(summary.active, "работа", "работы", "работ")} в процессе
                {next && nextDl && (
                  <>
                    <span className="faint">, ближайшая — </span>
                    <Link to={`/homework/${next.homeworkId}`} className="dash__next" data-tone={nextDl.tone}>
                      {names(next.subjectId)}, {nextDl.text}
                    </Link>
                  </>
                )}
              </h1>
              <p className="muted">
                {d.inbox.total > 0
                  ? `Во «Входящих» ${d.inbox.total} ${plural(d.inbox.total, "файл", "файла", "файлов")}${d.inbox.ambiguous ? `, ${d.inbox.ambiguous} требуют уточнения` : " — все распознаны"}.`
                  : "Входящие разобраны."}{" "}
                Прогресс недели: {d.weekProgress.done} из {d.weekProgress.total}.
              </p>
            </>
          ) : (
            <div style={{ display: "grid", gap: 10 }}>
              <Skeleton h={26} w="70%" />
              <Skeleton h={14} w="50%" />
            </div>
          )}
        </div>
        <div className="dash__cta">
          <button className="btn btn--primary btn--lg" onClick={() => navigate("/inbox")} disabled={!d}>
            <Inbox /> Разобрать входящие
            {d && d.inbox.total > 0 && <span className="dash__ctacount num">{d.inbox.total}</span>}
          </button>
        </div>
      </motion.header>

      <motion.section className="dash__rail" variants={rise} aria-label="Учебный процесс недели">
        {introDone && rail ? <ProcessRail layoutId="process-rail" counts={rail.counts} reached={rail.reached} onStageClick={(i) => navigate(i === 1 || i === 4 ? "/ai" : "/homework")} /> : <div style={{ height: 64 }} />}
      </motion.section>

      <motion.div className="dash__workflow-summary" variants={rise} aria-label="Сводка домашних работ">
        <Link to="/homework" className="dash-stat"><span className="dash-stat__value num">{summary.active}</span><span>активных</span></Link>
        <Link to="/homework" className="dash-stat"><span className="dash-stat__value num">{summary.ready}</span><span>готово к сдаче</span></Link>
        <Link to="/homework" className="dash-stat"><span className="dash-stat__value num">{summary.submitted}</span><span>отправлено</span></Link>
      </motion.div>

      <div className="dash__grid">
        <div className="dash__col">
          <motion.section className="panel" variants={rise}>
            <div className="panel__head">
              <h2 className="panel__title">Ближайшие дедлайны</h2>
              <Link to="/homework" className="panel__meta">
                Все ДЗ <ArrowRight size={13} />
              </Link>
            </div>
            <div className="panel__body">
              {!d ? (
                <SkeletonRows count={4} />
              ) : d.deadlines.length === 0 ? (
                <EmptyState icon={<Clock />} title="Дедлайнов нет" text="Все работы отправлены. Новые появятся после загрузки условий." />
              ) : (
                <ul className="deadlines">
                  {d.deadlines.map((dl) => {
                    const f = formatDeadline(dl.deadline);
                    const hw = d.activeHomework.find((h) => h.id === dl.homeworkId);
                    const presentation = hw ? getWorkflowPresentation(hw.workflowStage) : null;
                    return (
                      <li key={dl.homeworkId}>
                        <Link to={`/homework/${dl.homeworkId}${presentation ? `?tab=${presentation.tab}` : ""}`} className="deadline" data-tone={f.tone}>
                          <span className="deadline__when">{f.text}</span>
                          <span className="deadline__what">
                            <span className="deadline__subject">{names(dl.subjectId)}</span>
                            <span className="deadline__title">
                              Неделя {hw?.weekNumber} · {dl.title}
                            </span>
                          </span>
                          {hw && (
                            <span className="deadline__workflow">
                              <span className="deadline__stage">{WORKFLOW_STAGE_LABEL[hw.workflowStage]}</span>
                              <span className="deadline__action">{presentation?.action}</span>
                            </span>
                          )}
                          <ArrowRight size={14} className="faint" />
                        </Link>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          </motion.section>

          <motion.section className="panel" variants={rise}>
            <div className="panel__head">
              <h2 className="panel__title">Недавно изменённые</h2>
            </div>
            <div className="panel__body">
              {!d ? (
                <SkeletonRows count={3} />
              ) : (
                <ul className="files-compact">
                  {d.recentFiles.map((f) => (
                    <li key={f.id} className="row">
                      <FileIcon kind={f.kind} />
                      <div className="row__main">
                        <div className="row__title">{f.name}</div>
                        <div className="row__sub mono">{f.relativePath.split("/").slice(0, -1).join(" / ")}</div>
                      </div>
                      <span className="faint num" style={{ fontSize: 12 }}>
                        {formatRelative(f.modifiedAt)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </motion.section>
        </div>

        <div className="dash__col dash__col--side">
          <motion.section className="panel" variants={rise}>
            <div className="panel__head">
              <h2 className="panel__title">Входящие</h2>
              {d && d.inbox.ambiguous > 0 && <span className="badge badge--warn panel__meta">{d.inbox.ambiguous} неясно</span>}
            </div>
            <div className="panel__body">
              {!d ? (
                <SkeletonRows count={3} />
              ) : d.inbox.preview.length === 0 ? (
                <EmptyState icon={<Inbox />} title="Пусто" text="Новые загрузки появятся здесь автоматически." />
              ) : (
                <ul>
                  {d.inbox.preview.map((f) => (
                    <li key={f.id} className="row">
                      <FileIcon kind={f.kind} />
                      <div className="row__main">
                        <div className="row__title">{f.name}</div>
                        <div className="row__sub">{f.classification?.subjectId ? `${names(f.classification.subjectId)} · ${CATEGORY_LABEL[f.classification.category]}` : "Предмет не определён"}</div>
                      </div>
                      {f.classification && <Confidence value={f.classification.confidence} />}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </motion.section>

          <motion.section className="panel" variants={rise}>
            <div className="panel__head">
              <h2 className="panel__title">Неделя</h2>
              {d && (
                <span className="panel__meta num">
                  {d.weekProgress.done}/{d.weekProgress.total}
                </span>
              )}
            </div>
            <div className="panel__body">
              {d && (
                <>
                  <div className="meter" style={{ marginBottom: 12 }}>
                    <motion.span className="meter__fill" initial={{ scaleX: 0 }} animate={{ scaleX: d.weekProgress.done / Math.max(1, d.weekProgress.total) }} transition={{ type: "spring", stiffness: 120, damping: 22, delay: 0.2 }} />
                  </div>
                  <ul className="checklist">
                    {d.weekProgress.items.map((it) => (
                      <li key={it.label} data-done={it.done}>
                        <span className="checklist__box" aria-hidden />
                        {it.label}
                        <span className="sr-only">{it.done ? " — выполнено" : " — не выполнено"}</span>
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </div>
          </motion.section>

          <motion.section className="panel" variants={rise}>
            <div className="panel__head">
              <h2 className="panel__title">Автоматизация</h2>
            </div>
            <div className="panel__body">
              {d && (
                <Link to="/automation" className="automation-status">
                  <span className={`dot ${d.automation.state === "active" ? "dot--ok" : d.automation.state === "paused" ? "dot--warn" : "dot--danger"}`} />
                  <div className="row__main">
                    <div className="row__title">{d.automation.state === "active" ? "Наблюдение включено" : d.automation.state === "paused" ? "Пауза" : "Ошибка"}</div>
                    <div className="row__sub">
                      <FolderSync size={11} style={{ verticalAlign: -1 }} /> <span className="mono">{d.automation.watchedPath}</span> · {formatRelative(d.automation.lastRunAt)}
                    </div>
                  </div>
                  <ArrowRight size={14} className="faint" />
                </Link>
              )}
            </div>
          </motion.section>

          <motion.section className="panel" variants={rise}>
            <div className="panel__head">
              <h2 className="panel__title">Нейросети</h2>
              <Link to="/ai" className="panel__meta">
                Workspace <ArrowRight size={13} />
              </Link>
            </div>
            <div className="panel__body" style={{ display: "grid", gap: 6 }}>
              {d?.providers.map((p) => <ProviderCard key={p.provider} status={p} compact />)}
            </div>
          </motion.section>
        </div>
      </div>
    </motion.div>
  );
}
