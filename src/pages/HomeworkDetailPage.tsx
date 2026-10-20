import { motion } from "motion/react";
import { ArrowLeft, ArrowRight, BrainCircuit, ExternalLink, FilePlus, Flag, GitCompareArrows, Pin, ScanSearch, Send, Wand2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { weekPath } from "../app/nav";
import { useOps } from "../app/operations";
import { useToast } from "../app/toasts";
import { IterationChain } from "../components/domain/IterationChain";
import { ReviewResultView } from "../components/domain/ReviewResultView";
import { RunMonitor } from "../components/domain/RunMonitor";
import { SourceList } from "../components/domain/SourceList";
import { SubmissionPanel } from "../components/domain/SubmissionPanel";
import { TeacherCommentsPanel } from "../components/domain/TeacherCommentsPanel";
import { VersionComparePanel } from "../components/domain/VersionComparePanel";
import { WorkflowRail } from "../components/domain/WorkflowRail";
import { Dialog } from "../components/ui/Dialog";
import { FileIcon } from "../components/ui/FileIcon";
import { EmptyState, ErrorState, SkeletonRows } from "../components/ui/States";
import { Tabs } from "../components/ui/Tabs";
import { bridge, type FileRef, type HomeworkIteration, type HomeworkWorkflowStage, type ReviewRun, type SourceBundle } from "../lib/bridge";
import { formatBytes, formatDateTime, formatDeadline, formatRelative, pad2, providerName, WORKFLOW_STAGE_LABEL } from "../lib/format";
import { useBridgeEvent } from "../lib/hooks/useBridgeEvent";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
import { useHomeworkWorkflow } from "../lib/hooks/useHomeworkWorkflow";
import { useSubjectNames } from "../lib/hooks/useSubjectNames";
import { useWorkflowActions } from "../lib/hooks/useWorkflowActions";
import { applyNamePattern } from "../lib/naming";

type Tab = "materials" | "cycle" | "review" | "teacher" | "package";

const DEFAULT_REASON: Record<HomeworkWorkflowStage, string> = {
  materials_collected: "Проверьте материалы и запустите анализ — получится первичный промт.",
  codex_analysis: "Идёт анализ материалов.",
  prompt_ready: "Промт готов — передайте его автору или скопируйте.",
  claude_solving: "Автор создаёт решение. Версия появится в «02 Работа».",
  codex_review: "Новая версия ждёт проверки.",
  needs_fixes: "Проверка нашла проблемы — нужен промт исправлений.",
  teacher_review: "Проверка пройдена. Сверьте работу с комментариями преподавателя.",
  ready_to_submit: "Все проверки пройдены. Соберите комплект и отправьте работу сами.",
  submitted: "Работа отправлена. Снимок в «04 Отправлено» неизменяем.",
};

const TAB_FOR: Record<HomeworkWorkflowStage, Tab> = {
  materials_collected: "materials",
  codex_analysis: "materials",
  prompt_ready: "cycle",
  claude_solving: "cycle",
  codex_review: "review",
  needs_fixes: "review",
  teacher_review: "teacher",
  ready_to_submit: "package",
  submitted: "package",
};

export function HomeworkDetailPage() {
  const { homeworkId = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const names = useSubjectNames();
  const toast = useToast();
  const { reportError, undo } = useOps();
  const wf = useHomeworkWorkflow(homeworkId);
  const actions = useWorkflowActions(homeworkId);
  const rules = useBridgeQuery(() => bridge.getAutomationRules(), [], ["operation:completed"]);
  const h = wf.homework.data;

  const [tab, setTabState] = useState<Tab | null>((params.get("tab") as Tab | null) ?? null);
  const [reason, setReason] = useState<string | null>(null);
  const [bundle, setBundle] = useState<SourceBundle | null>(null);
  const [compare, setCompare] = useState<string[]>([]);
  const [reviewVersion, setReviewVersion] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [candidates, setCandidates] = useState<FileRef[] | null>(null);
  const [finalTarget, setFinalTarget] = useState<HomeworkIteration | null>(null);
  const [busy, setBusy] = useState(false);

  useBridgeEvent("workflow:stage-changed", (e) => {
    if (e.homeworkId === homeworkId) setReason(e.reason);
  });

  // Набор источников для анализа; пересобирается при смене ДЗ.
  useEffect(() => {
    let alive = true;
    void bridge.prepareSourceBundle(homeworkId, "analyze_materials").then((b) => alive && setBundle(b));
    return () => {
      alive = false;
    };
  }, [homeworkId]);

  const runs = wf.runs.data ?? [];
  const activeRun = runs.find((r) => r.status === "running" || r.status === "queued") ?? null;
  const latest = h?.versions[h.versions.length - 1] ?? null;
  const selectedVersionId = reviewVersion ?? latest?.id ?? null;
  const reviewRun = useMemo<ReviewRun | null>(() => runs.find((r) => r.versionId === selectedVersionId && (r.kind === "review_version" || r.kind === "revision_prompt") && r.status === "completed") ?? null, [runs, selectedVersionId]);
  const teacherRun = runs.find((r) => r.kind === "review_teacher" && r.status === "completed" && r.versionId === latest?.id) ?? null;
  const prompts = wf.prompts.data ?? [];
  const pendingPrompt = [...prompts].reverse().find((p) => !p.sentAt) ?? null;
  const promptForRun = (runId: string) => prompts.find((p) => p.reviewRunId === runId) ?? null;

  if (wf.homework.error && !h) return <ErrorState message={wf.homework.error} onRetry={() => void wf.reloadAll()} />;
  if (wf.homework.loading && !h) return <SkeletonRows count={6} />;
  if (!h) return <EmptyState icon={<Flag />} title="ДЗ не найдено" action={<Link className="btn" to="/homework">К списку</Link>} />;

  const tabNow: Tab = tab ?? TAB_FOR[h.workflowStage];
  const setTab = (t: Tab) => {
    setTabState(t);
    setParams((p) => {
      p.set("tab", t);
      return p;
    }, { replace: true });
  };
  const locked = h.workflowStage === "submitted";
  const deadline = formatDeadline(h.deadline);
  const author = rules.data ? providerName(rules.data.workflow.roles.author) : "автор";
  const reviewer = rules.data ? providerName(rules.data.workflow.roles.reviewer) : "проверяющий";
  const finalExt = (finalTarget?.files[0]?.name ?? latest?.fileName ?? ".pdf").split(".").pop();
  const cycle = Math.max(1, h.versions.length);
  const blockedStages: HomeworkWorkflowStage[] = locked || h.workflowStage === "ready_to_submit" || wf.readiness.data?.ready ? [] : ["ready_to_submit", "submitted"];

  const openAdd = async () => {
    setAddOpen(true);
    setCandidates(null);
    try {
      setCandidates(await bridge.getHomeworkCandidates(h.id));
    } catch (e) {
      reportError(e);
      setAddOpen(false);
    }
  };

  const addVersion = async (fileId: string) => {
    setBusy(true);
    try {
      const res = await bridge.createHomeworkVersion(h.id, fileId, { author: "user" });
      setAddOpen(false);
      toast.push({ tone: "ok", title: res.operation.title, text: `${res.value.fileName} — версия неизменяема` });
    } catch (e) {
      reportError(e, "Версия не создана");
    } finally {
      setBusy(false);
    }
  };

  const markFinal = async () => {
    if (!finalTarget) return;
    setBusy(true);
    try {
      const res = await bridge.markVersionFinal(finalTarget.versionId);
      setFinalTarget(null);
      toast.push({ tone: "ok", title: res.operation.title, text: `03 На сдачу/${res.value.finalFileName}`, action: { label: "Отменить", run: () => void undo(res.operation.id) } });
    } catch (e) {
      reportError(e, "Не удалось назначить итоговую");
    } finally {
      setBusy(false);
    }
  };

  /** Следующий шаг — одно главное действие для текущего этапа. */
  const nextStep = (() => {
    if (activeRun || h.workflowStage === "claude_solving" || actions.busy === "send") return null;
    switch (h.workflowStage) {
      case "materials_collected":
        return (
          <button className="btn btn--primary" disabled={!bundle?.complete || !!actions.busy} onClick={() => void actions.startRun("analyze_materials", null, bundle)}>
            <BrainCircuit /> Проанализировать материалы
          </button>
        );
      case "prompt_ready":
        return pendingPrompt ? (
          <>
            <button className="btn btn--primary" disabled={!!actions.busy} onClick={() => void actions.sendPrompt(pendingPrompt.id)}>
              <Send /> Передать промт {author}
            </button>
            <button className="btn" onClick={() => setTab("cycle")}>
              Посмотреть промт
            </button>
          </>
        ) : null;
      case "codex_review":
        return latest ? (
          <button className="btn btn--primary" disabled={!!actions.busy} onClick={() => void actions.startRun("review_version", latest.id).then(() => setTab("review"))}>
            <ScanSearch /> Проверить {latest.label}
          </button>
        ) : null;
      case "needs_fixes": {
        const src = [teacherRun, reviewRun].find((r) => r?.result?.prompt && r.versionId === latest?.id && !promptForRun(r.id));
        return src ? (
          <button className="btn btn--primary" disabled={!!actions.busy} onClick={() => void actions.createPrompt(src.id)}>
            <Wand2 /> Создать промт исправлений
          </button>
        ) : null;
      }
      case "teacher_review":
        return (
          <button className="btn btn--primary" onClick={() => setTab(wf.readiness.data?.ready ? "package" : "teacher")}>
            {wf.readiness.data?.ready ? "К комплекту на сдачу" : "Комментарии преподавателя"} <ArrowRight />
          </button>
        );
      case "ready_to_submit":
        return (
          <button className="btn btn--primary" onClick={() => setTab("package")}>
            К комплекту на сдачу <ArrowRight />
          </button>
        );
      default:
        return null;
    }
  })();

  const comments = wf.comments.data ?? [];
  const reqs = wf.requirements.data ?? [];

  return (
    <div className="hw">
      <Link to="/homework" className="backlink">
        <ArrowLeft size={14} /> Все ДЗ
      </Link>
      <header className="pagehead pagehead--split">
        <div>
          <span className="eyebrow">
            <Link to={weekPath(h.subjectId, h.weekNumber)}>
              {names(h.subjectId)} · Неделя {pad2(h.weekNumber)}
            </Link>
          </span>
          <h1 className="pagehead__title">{h.title}</h1>
          <p className="deadline-inline" data-tone={locked ? "past" : deadline.tone}>
            Дедлайн {formatDateTime(h.deadline)} · {deadline.text}
          </p>
        </div>
        <div className="pagehead__actions">
          <Link to={`/ai?homework=${h.id}`} className="btn">
            <BrainCircuit /> AI Workspace
          </Link>
        </div>
      </header>

      <section className="panel wf">
        <WorkflowRail homeworkId={h.id} stage={h.workflowStage} cycle={cycle} blockedStages={blockedStages} />
        <motion.div className="wf__next" key={h.workflowStage} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}>
          <div className="wf__now">
            <span className="eyebrow">Сейчас · {WORKFLOW_STAGE_LABEL[h.workflowStage]}</span>
            <p>{reason ?? DEFAULT_REASON[h.workflowStage]}</p>
          </div>
          <div className="wf__actions">{nextStep}</div>
        </motion.div>
        <RunMonitor run={activeRun} />
        {h.workflowStage === "claude_solving" && !activeRun && (
          <div className="faint wf__solving">
            <span className="scanline" aria-hidden /> {author} создаёт решение — прогресс в заголовке окна
          </div>
        )}
      </section>

      <Tabs<Tab>
        idBase="homework-sections"
        label="Разделы ДЗ"
        value={tabNow}
        onChange={setTab}
        items={[
          { value: "materials", label: "Исходные материалы", badge: bundle && !bundle.complete ? <span className="badge badge--danger">!</span> : undefined },
          { value: "cycle", label: "Рабочий цикл", badge: <span className="tabs__n num">{h.versions.length}</span> },
          { value: "review", label: "Результат проверки" },
          { value: "teacher", label: "Комментарии преподавателя", badge: <span className="tabs__n num">{comments.length}</span> },
          { value: "package", label: "Комплект на сдачу" },
        ]}
      />

      <motion.div key={tabNow} id={`homework-sections-panel-${tabNow}`} aria-labelledby={`homework-sections-tab-${tabNow}`} role="tabpanel" tabIndex={0} className="hw__tab" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.2 }}>
        {tabNow === "materials" && (
          <section className="panel">
            <div className="panel__head">
              <h2 className="panel__title">Исходные материалы</h2>
              <span className="panel__meta faint">Снимите отметку, чтобы не передавать файл</span>
            </div>
            <div className="panel__body">
              {!bundle ? (
                <SkeletonRows count={5} />
              ) : (
                <SourceList
                  bundle={bundle}
                  busy={locked}
                  onToggle={locked ? undefined : (id, inc) => void bridge.setSourceIncluded(bundle.id, id, inc).then(setBundle, (e: unknown) => reportError(e))}
                />
              )}
            </div>
          </section>
        )}

        {tabNow === "cycle" && (
          <div className="hw__cycle">
            <section className="panel">
              <div className="panel__head">
                <h2 className="panel__title">Итерации</h2>
                <div className="panel__meta">
                  {compare.length > 0 && (
                    <button className="btn btn--sm btn--ghost" onClick={() => setCompare([])}>
                      Сбросить сравнение
                    </button>
                  )}
                  <button className="btn btn--sm" onClick={() => void openAdd()} disabled={locked}>
                    <FilePlus /> Добавить версию вручную
                  </button>
                </div>
              </div>
              <div className="panel__body">
                {pendingPrompt && !locked && (
                  <div className="hw__pending">
                    <div className="eyebrow">Ожидает передачи автору</div>
                    <p className="faint">{pendingPrompt.kind === "initial" ? "Первичный промт" : pendingPrompt.kind === "teacher" ? "Промт по комментариям преподавателя" : "Промт исправлений"} от {formatRelative(pendingPrompt.createdAt)}</p>
                    <pre className="promptbox__text promptbox__text--ro selectable">{pendingPrompt.text}</pre>
                    <div className="promptbox__actions">
                      <button className="btn btn--primary btn--sm" disabled={!!actions.busy || !!activeRun} onClick={() => void actions.sendPrompt(pendingPrompt.id)}>
                        <Send /> Передать {author}
                      </button>
                      <button className="btn btn--sm" onClick={() => rules.data && void actions.openApp(rules.data.workflow.roles.author)}>
                        <ExternalLink /> Открыть {author}
                      </button>
                    </div>
                  </div>
                )}
                {!wf.iterations.data ? (
                  <SkeletonRows count={4} />
                ) : wf.iterations.data.length === 0 ? (
                  <EmptyState icon={<FilePlus />} title="Версий пока нет" text={`Запустите анализ материалов — промт будет передан ${author}, и первая версия появится здесь.`} />
                ) : (
                  <IterationChain
                    iterations={wf.iterations.data}
                    finalVersionId={h.finalVersionId}
                    compare={compare}
                    locked={locked}
                    onToggleCompare={(id) => setCompare((c) => (c.includes(id) ? c.filter((x) => x !== id) : [...c, id].slice(-2)))}
                    onMarkFinal={setFinalTarget}
                  />
                )}
                {compare.length === 1 && (
                  <p className="faint hw__cmphint">
                    <GitCompareArrows size={13} /> Отметьте ещё одну версию для сравнения
                  </p>
                )}
              </div>
            </section>
            {compare.length === 2 && (
              <section className="panel">
                <div className="panel__head">
                  <h2 className="panel__title">Сравнение версий</h2>
                </div>
                <div className="panel__body">
                  <VersionComparePanel fromId={compare[0]!} toId={compare[1]!} />
                </div>
              </section>
            )}
          </div>
        )}

        {tabNow === "review" && (
          <section className="panel">
            <div className="panel__head">
              <h2 className="panel__title">Результат проверки</h2>
              <div className="panel__meta">
                <select className="select select--sm review__version" value={selectedVersionId ?? ""} onChange={(e) => setReviewVersion(e.target.value)} aria-label="Версия">
                  {h.versions.map((v) => (
                    <option key={v.id} value={v.id}>
                      {v.label}
                    </option>
                  ))}
                </select>
                {selectedVersionId && (
                  <button className="btn btn--sm btn--primary" disabled={!!activeRun || !!actions.busy || locked} onClick={() => void actions.startRun("review_version", selectedVersionId)}>
                    <ScanSearch /> {reviewRun ? "Проверить снова" : "Проверить"} ({reviewer})
                  </button>
                )}
              </div>
            </div>
            <div className="panel__body">
              {!reviewRun ? (
                <EmptyState icon={<ScanSearch />} title="Проверки этой версии ещё не было" text="Проверка оценит корректность, соответствие заданию, опору на материалы, стиль, требования преподавателя и комплектность файлов." />
              ) : (
                <>
                  <div className="review__toolbar">
                    <span className="faint">
                      {providerName(reviewRun.provider)} · {formatDateTime(reviewRun.startedAt)}
                    </span>
                    {reviewRun.savedAsIteration ? (
                      <span className="badge badge--ok">
                        <Pin /> сохранено в истории
                      </span>
                    ) : (
                      <button className="btn btn--sm" disabled={locked || !!actions.busy} onClick={() => void actions.saveIteration(reviewRun.id)}>
                        <Pin /> Сохранить как итерацию
                      </button>
                    )}
                    {h.versions.findIndex((v) => v.id === selectedVersionId) > 0 && (
                      <button
                        className="btn btn--sm btn--ghost"
                        onClick={() => {
                          const i = h.versions.findIndex((v) => v.id === selectedVersionId);
                          setCompare([h.versions[i - 1]!.id, selectedVersionId!]);
                          setTab("cycle");
                        }}
                      >
                        <GitCompareArrows /> Сравнить с предыдущей
                      </button>
                    )}
                  </div>
                  <ReviewResultView
                    run={reviewRun}
                    comments={comments}
                    promptActions={
                      !locked && reviewRun.result?.prompt ? (
                        promptForRun(reviewRun.id) ? (
                          <span className="badge badge--ok">промт сохранён</span>
                        ) : (
                          <button className="btn btn--sm btn--primary" disabled={!!actions.busy} onClick={() => void actions.createPrompt(reviewRun.id)}>
                            <Wand2 /> Сохранить промт исправлений
                          </button>
                        )
                      ) : undefined
                    }
                  />
                </>
              )}
            </div>
          </section>
        )}

        {tabNow === "teacher" && (
          <>
            {teacherRun?.result && (
              <div className={`notice teacher__review-notice ${teacherRun.result.verdict === "needs_fixes" ? "notice--danger" : "notice--ok"}`}>
                <ScanSearch />
                <div>
                  Сверка {h.versions.find((v) => v.id === teacherRun.versionId)?.label} с комментариями: {teacherRun.result.summary}
                </div>
                {teacherRun.result.prompt && !promptForRun(teacherRun.id) && !locked && (
                  <button className="btn btn--sm notice__action" onClick={() => void actions.createPrompt(teacherRun.id)}>
                    <Wand2 /> Создать промт исправлений
                  </button>
                )}
              </div>
            )}
            <TeacherCommentsPanel
              homeworkId={h.id}
              comments={comments}
              requirements={reqs}
              locked={locked}
              running={!!activeRun}
              canReview={!!latest}
              onReview={() => latest && void actions.startRun("review_teacher", latest.id)}
              onChanged={() => void wf.reloadAll()}
            />
          </>
        )}

        {tabNow === "package" && <SubmissionPanel homework={h} readiness={wf.readiness.data} pkg={wf.pkg.data} receipt={wf.receipt.data} commentsCount={comments.length} onChanged={() => void wf.reloadAll()} />}
      </motion.div>

      <Dialog open={addOpen} onClose={() => setAddOpen(false)} title="Добавить версию вручную" description="Файл из «02 Работа» станет новой версией (автор — вы). Существующие версии не изменяются.">
        {!candidates ? (
          <SkeletonRows count={2} />
        ) : (
          <ul className="pick">
            {candidates.map((f) => (
              <li key={f.id}>
                <button className="row pick__row" onClick={() => void addVersion(f.id)} disabled={busy}>
                  <FileIcon kind={f.kind} />
                  <div className="row__main">
                    <div className="row__title">{f.name}</div>
                    <div className="row__sub">
                      {formatBytes(f.sizeBytes)} · изменён {formatRelative(f.modifiedAt)}
                    </div>
                  </div>
                  <span className="badge badge--accent mono">→ v{pad2(h.versions.length + 1)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Dialog>

      <Dialog
        open={!!finalTarget}
        onClose={() => setFinalTarget(null)}
        title={`Сделать ${finalTarget?.label ?? ""} итоговой?`}
        footer={
          <>
            <button className="btn btn--ghost" onClick={() => setFinalTarget(null)}>
              Отмена
            </button>
            <button className="btn btn--primary" onClick={() => void markFinal()} disabled={busy} data-autofocus>
              <Flag /> Назначить итоговой
            </button>
          </>
        }
      >
        <p className="muted">Копия ляжет в «03 На сдачу» под именем:</p>
        <div className="path mono">
          03 На сдачу/
          {rules.data && finalTarget ? applyNamePattern(rules.data.finalNamePattern, { ...rules.data.studentName, week: h.weekNumber, subject: names(h.subjectId), version: finalTarget.label }, finalExt) : "…"}
        </div>
        {finalTarget && finalTarget.status !== "passed" && finalTarget.status !== "final" && (
          <div className="notice notice--warn">
            <Flag />
            <div>У этой версии нет пройденной проверки — «Готово к сдаче» будет недоступно, пока проверка не пройдена.</div>
          </div>
        )}
      </Dialog>
    </div>
  );
}
