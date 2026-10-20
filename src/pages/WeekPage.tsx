import { motion } from "motion/react";
import { ArrowRight, BookOpen, ChevronLeft, ChevronRight, FilePlus, FolderTree, History, Plus } from "lucide-react";
import { Navigate, Link, useNavigate, useParams } from "react-router-dom";
import { weekPath } from "../app/nav";
import { useOps } from "../app/operations";
import { useToast } from "../app/toasts";
import { WorkflowRail } from "../components/domain/WorkflowRail";
import { FileIcon } from "../components/ui/FileIcon";
import { EmptyState, ErrorState, SkeletonRows } from "../components/ui/States";
import { StageBadge } from "../components/ui/StatusBadges";
import { bridge, type FileRef } from "../lib/bridge";
import { formatBytes, formatDate, formatDateTime, formatDeadline, formatRelative, pad2, WORKFLOW_STAGE_LABEL } from "../lib/format";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";

export function WeekIndexRedirect() {
  const subjects = useBridgeQuery(() => bridge.getSubjects(), []);
  if (!subjects.data) return <SkeletonRows count={3} />;
  const s = subjects.data.find((x) => x.id === "oba") ?? subjects.data[0];
  return s ? <Navigate to={weekPath(s.id, s.currentWeek)} replace /> : null;
}

function FileList({ files, empty }: { files: FileRef[]; empty: string }) {
  if (files.length === 0) return <p className="faint wcol__empty">{empty}</p>;
  return (
    <ul>
      {files.map((f) => (
        <motion.li key={f.id} className="row" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}>
          <FileIcon kind={f.kind} />
          <div className="row__main">
            <div className="row__title" title={f.name}>
              {f.name}
            </div>
            <div className="row__sub">
              {formatBytes(f.sizeBytes)} · {formatRelative(f.modifiedAt)}
            </div>
          </div>
        </motion.li>
      ))}
    </ul>
  );
}

export function WeekPage() {
  const { subjectId = "", n = "1" } = useParams();
  const weekNumber = Number(n);
  const navigate = useNavigate();
  const toast = useToast();
  const { reportError, undo } = useOps();
  const subjects = useBridgeQuery(() => bridge.getSubjects(), []);
  const weeks = useBridgeQuery(() => bridge.getWeeks(subjectId), [subjectId], ["operation:completed"]);
  const week = weeks.data?.find((w) => w.number === weekNumber) ?? null;
  const hw = useBridgeQuery(() => (week?.homeworkId ? bridge.getHomework(week.homeworkId) : Promise.resolve(null)), [week?.homeworkId], ["operation:completed"]);
  const log = useBridgeQuery(() => bridge.getActivityLog(), [], ["operation:completed"]);
  const readiness = useBridgeQuery(() => (week?.homeworkId ? bridge.getReadiness(week.homeworkId) : Promise.resolve(null)), [week?.homeworkId], ["operation:completed", "workflow:stage-changed"]);
  const style = useBridgeQuery(() => bridge.getStyleProfile(), []);
  const subject = subjects.data?.find((s) => s.id === subjectId);
  const maxWeek = Math.max(0, ...(weeks.data?.map((w) => w.number) ?? [0]));

  const create = async (num: number) => {
    try {
      const res = await bridge.createWeek(subjectId, num);
      toast.push({ tone: "ok", title: res.operation.title, text: "Созданы папки 01 Лекции · 02 Семинары · 03 ДЗ · 04 Материалы", action: { label: "Отменить", run: () => void undo(res.operation.id) } });
      navigate(weekPath(subjectId, num));
    } catch (e) {
      reportError(e, "Неделя не создана");
    }
  };

  if (weeks.error) return <ErrorState message={weeks.error} onRetry={() => void weeks.reload()} />;

  const history = (log.data ?? []).filter((o) => subject && (o.detail ?? "").includes(subject.name)).slice(0, 6);
  const homework = hw.data;

  return (
    <div className="week">
      <header className="pagehead pagehead--split">
        <div>
          <label className="week__subject">
            <span className="sr-only">Предмет</span>
            <select className="select week__subjectselect" value={subjectId} onChange={(e) => {
              const s = subjects.data?.find((x) => x.id === e.target.value);
              if (s) navigate(weekPath(s.id, s.currentWeek));
            }}>
              {subjects.data?.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
          <h1 className="pagehead__title">
            <span className="num">Неделя {pad2(weekNumber)}</span>
            {week?.topic && <span className="faint"> · {week.topic}</span>}
          </h1>
          {week && (
            <p className="muted">
              {formatDate(week.startsAt)} — {formatDate(week.endsAt)} · <span className="mono">{subject?.name}/Неделя {pad2(weekNumber)}</span>
            </p>
          )}
        </div>
        <div className="weektabs" role="tablist" aria-label="Недели">
          <button className="btn btn--sm btn--ghost btn--icon" aria-label="Предыдущая неделя" disabled={weekNumber <= 1} onClick={() => navigate(weekPath(subjectId, weekNumber - 1))}>
            <ChevronLeft />
          </button>
          {Array.from({ length: maxWeek }, (_, i) => i + 1).map((num) => (
            <button key={num} role="tab" aria-selected={num === weekNumber} className="weektab num" onClick={() => navigate(weekPath(subjectId, num))}>
              {num === weekNumber && <motion.span layoutId="weektab" className="weektab__bg" />}
              <span>{pad2(num)}</span>
            </button>
          ))}
          <button className="btn btn--sm btn--ghost btn--icon" aria-label="Создать следующую неделю" title={`Создать неделю ${maxWeek + 1}`} onClick={() => void create(maxWeek + 1)}>
            <Plus />
          </button>
          <button className="btn btn--sm btn--ghost btn--icon" aria-label="Следующая неделя" disabled={weekNumber >= maxWeek} onClick={() => navigate(weekPath(subjectId, weekNumber + 1))}>
            <ChevronRight />
          </button>
        </div>
      </header>

      {weeks.loading && !weeks.data ? (
        <SkeletonRows count={6} />
      ) : !week ? (
        <EmptyState
          icon={<FilePlus />}
          title={`Недели ${weekNumber} ещё нет`}
          text="Папка недели создаётся автоматически при первой раскладке файла, или её можно создать вручную."
          action={
            <button className="btn btn--primary" onClick={() => void create(weekNumber)}>
              <Plus /> Создать неделю {pad2(weekNumber)}
            </button>
          }
        />
      ) : (
        <motion.div key={week.id} className="week__grid" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: 0.2 }}>
          <section className="panel week__hw">
            <div className="panel__head">
              <h2 className="panel__title">Домашняя работа</h2>
              {homework && <span className="badge badge--accent">{WORKFLOW_STAGE_LABEL[homework.workflowStage]}</span>}
              {homework && <StageBadge stage={homework.stage} />}
            </div>
            <div className="panel__body">
              {homework ? (
                <div className="week__hwbody">
                  <div>
                    <div className="week__hwtitle">{homework.title}</div>
                    <div className={`deadline-inline`} data-tone={formatDeadline(homework.deadline).tone}>
                      Дедлайн: {formatDateTime(homework.deadline)} · {formatDeadline(homework.deadline).text}
                    </div>
                  </div>
                  <WorkflowRail homeworkId={homework.id} stage={homework.workflowStage} cycle={Math.max(1, homework.versions.length)} />
                  {readiness.data && (
                    <div className="week__checks">
                      {readiness.data.checks
                        .filter((c) => c.id === "materials" || c.id === "teacher")
                        .map((c) => (
                          <span key={c.id} className={`week__check${c.ok ? "" : " week__check--bad"}`}>
                            <span className={`dot ${c.ok ? "dot--ok" : "dot--warn"}`} /> {c.label}: <span className="faint">{c.detail}</span>
                          </span>
                        ))}
                      <span className="week__check">
                        <span className={`dot ${style.data ? "dot--ok" : "dot--danger"}`} /> Файл «Мой стиль»: <span className="faint mono">{style.data?.fileName ?? "не найден"}</span>
                      </span>
                    </div>
                  )}
                  <div className="week__hwfoot">
                    <span className="faint">
                      Версий: {homework.versions.length}
                      {homework.finalFileName && <> · итог: <span className="mono">{homework.finalFileName}</span></>}
                    </span>
                    <Link to={`/homework/${homework.id}`} className="btn btn--sm">
                      Открыть ДЗ <ArrowRight />
                    </Link>
                  </div>
                </div>
              ) : (
                <EmptyState icon={<BookOpen />} title="ДЗ на эту неделю нет" text="Когда условие попадёт во «Входящие» с типом «Условие ДЗ», здесь появится работа." />
              )}
            </div>
          </section>

          <section className="panel wcol">
            <div className="panel__head">
              <h2 className="panel__title">Лекции</h2>
              <span className="panel__meta num">{week.lectures.length}</span>
            </div>
            <div className="panel__body">
              <FileList files={week.lectures} empty="Лекций пока нет" />
            </div>
          </section>
          <section className="panel wcol">
            <div className="panel__head">
              <h2 className="panel__title">Семинары</h2>
              <span className="panel__meta num">{week.seminars.length}</span>
            </div>
            <div className="panel__body">
              <FileList files={week.seminars} empty="Семинаров пока нет" />
            </div>
          </section>
          <section className="panel wcol">
            <div className="panel__head">
              <h2 className="panel__title">Материалы</h2>
              <span className="panel__meta num">{week.materials.length}</span>
            </div>
            <div className="panel__body">
              <FileList files={week.materials} empty="Дополнительных материалов нет" />
            </div>
          </section>

          <section className="panel week__tree">
            <div className="panel__head">
              <FolderTree size={14} className="faint" />
              <h2 className="panel__title">Связанные файлы</h2>
            </div>
            <div className="panel__body">
              <ul className="tree mono">
                <li data-depth="0">Неделя {pad2(week.number)}</li>
                <li data-depth="1">
                  01 Лекции <span className="tree__n">{week.lectures.length}</span>
                </li>
                <li data-depth="1">
                  02 Семинары <span className="tree__n">{week.seminars.length}</span>
                </li>
                <li data-depth="1">03 ДЗ</li>
                <li data-depth="2">
                  01 Условие и данные <span className="tree__n">{homework?.taskFiles.length ?? 0}</span>
                </li>
                <li data-depth="2">
                  02 Работа <span className="tree__n">{homework?.versions.length ?? 0}</span>
                </li>
                <li data-depth="2">
                  03 На сдачу <span className="tree__n">{homework?.finalFileName ? 1 : 0}</span>
                </li>
                <li data-depth="2" data-last>
                  04 Отправлено <span className="tree__n">{homework?.stage === "submitted" ? 1 : 0}</span>
                </li>
                <li data-depth="1" data-last>
                  04 Материалы <span className="tree__n">{week.materials.length}</span>
                </li>
              </ul>
            </div>
          </section>

          <section className="panel week__log">
            <div className="panel__head">
              <History size={14} className="faint" />
              <h2 className="panel__title">История действий</h2>
            </div>
            <div className="panel__body">
              {history.length === 0 ? (
                <p className="faint">Операций по предмету пока не было.</p>
              ) : (
                <ul className="loglist">
                  {history.map((o) => (
                    <li key={o.id} data-status={o.status}>
                      <span className="loglist__time num">{formatRelative(o.startedAt)}</span>
                      <span className="loglist__title">{o.title}</span>
                      {o.status === "undone" && <span className="badge">отменено</span>}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>
        </motion.div>
      )}
    </div>
  );
}
