import { motion } from "motion/react";
import { ArrowLeft, ArrowRight, BrainCircuit, ExternalLink, FilePlus, Flag, GitCompareArrows, Layers, Lock, NotebookPen, Pin, RefreshCw, ScanSearch, Send, ShieldCheck, Undo2, Wand2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { weekPath } from "../app/nav";
import { useOps } from "../app/operations";
import { useToast } from "../app/toasts";
import { IterationChain } from "../components/domain/IterationChain";
import { AiStepActions } from "../components/domain/AiStepActions";
import { PromptBox } from "../components/domain/PromptBox";
import { ReviewResultView } from "../components/domain/ReviewResultView";
import { isOpenRun, RunMonitor } from "../components/domain/RunMonitor";
import { SourceList } from "../components/domain/SourceList";
import { SubmissionPanel } from "../components/domain/SubmissionPanel";
import { TeacherCommentsPanel } from "../components/domain/TeacherCommentsPanel";
import { VersionComparePanel } from "../components/domain/VersionComparePanel";
import { WorkflowRail } from "../components/domain/WorkflowRail";
import { Dialog } from "../components/ui/Dialog";
import { FileIcon } from "../components/ui/FileIcon";
import { EmptyState, ErrorState, SkeletonRows } from "../components/ui/States";
import { Tabs } from "../components/ui/Tabs";
import { bridge, type FileRef, type HomeworkIteration, type HomeworkWorkflowStage, type ReviewRun, type RunKind, WORKFLOW_STAGE_ORDER } from "../lib/bridge";
import { formatBytes, formatDateTime, formatDeadline, formatRelative, pad2, providerName, WORKFLOW_STAGE_LABEL } from "../lib/format";
import { useBridgeEvent } from "../lib/hooks/useBridgeEvent";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
import { useHomeworkWorkflow } from "../lib/hooks/useHomeworkWorkflow";
import { useSubjectNames } from "../lib/hooks/useSubjectNames";
import { useWorkflowActions } from "../lib/hooks/useWorkflowActions";
import { applyNamePattern } from "../lib/naming";
import { getWorkflowPresentation, type HomeworkTab } from "../lib/workflow-ui";

type Tab = HomeworkTab;

const DEFAULT_REASON: Record<HomeworkWorkflowStage, string> = {
  materials_collected: "Проверьте материалы и запустите анализ — получится первичный промт.",
  codex_analysis: "Идёт анализ материалов.",
  prompt_ready: "Промт готов — передайте его автору или скопируйте.",
  claude_solving: "Промт передан автору. Получите решение в официальном приложении и добавьте его как новую версию.",
  codex_review: "Новая версия ждёт проверки.",
  needs_fixes: "Проверка нашла проблемы — нужен промт исправлений.",
  teacher_review: "Проверка пройдена. Сверьте работу с комментариями преподавателя.",
  ready_to_submit: "Все проверки пройдены. Соберите комплект и отправьте работу сами.",
  submitted: "Работа отправлена. Снимок в «04 Отправлено» неизменяем.",
};

export function HomeworkDetailPage() {
  const { homeworkId = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const names = useSubjectNames();
  const toast = useToast();
  const { reportError, undo } = useOps();
  const wf = useHomeworkWorkflow(homeworkId);
  const actions = useWorkflowActions(homeworkId, { revision: wf.revision, reload: wf.reloadAll });
  const rules = useBridgeQuery(() => bridge.getAutomationRules(), [], ["operation:completed"]);
  const providers = useBridgeQuery(() => bridge.getProviderStatuses(), [], ["provider:status-changed", "operation:completed"]);
  const h = wf.homework.data;

  const [tab, setTabState] = useState<Tab | null>((params.get("tab") as Tab | null) ?? null);
  const [reason, setReason] = useState<string | null>(null);
  const [promptEdit, setPromptEdit] = useState<string | null>(null);
  const [compare, setCompare] = useState<string[]>([]);
  const [reviewVersion, setReviewVersion] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [candidates, setCandidates] = useState<FileRef[] | null>(null);
  const [finalTarget, setFinalTarget] = useState<HomeworkIteration | null>(null);
  const [busy, setBusy] = useState(false);

  useBridgeEvent("workflow:stage-changed", (e) => {
    if (e.homeworkId === homeworkId) setReason(e.reason);
  });

  // Снимки источников живут в агрегате. Только демонстрационный mock собирает
  // набор при первом открытии; в desktop-режиме снимок создаётся явным действием.
  const demo = bridge.kind === "mock";
  const bundles = wf.workflow.data?.bundles ?? [];
  const bundle = [...bundles].reverse().find((b) => b.purpose === "analyze_materials") ?? bundles[bundles.length - 1] ?? null;
  const prepared = useRef<string | null>(null);
  useEffect(() => {
    if (!wf.workflow.data || !demo || bundles.length > 0 || prepared.current === homeworkId) return;
    prepared.current = homeworkId;
    void bridge.prepareSourceBundle(homeworkId, "analyze_materials").catch(() => undefined);
  }, [homeworkId, wf.workflow.data, demo, bundles.length]);

  const runs = wf.runs.data ?? [];
  const activeRun = runs.find(isOpenRun) ?? null;
  const jobs = wf.aiJobs.data ?? [];
  const activeJob = activeRun ? [...jobs].reverse().find((j) => j.runId === activeRun.id) ?? null : null;
  const latest = h?.versions[h.versions.length - 1] ?? null;
  const current = h?.versions.find((v) => v.id === h.finalVersionId) ?? latest;
  const selectedVersionId = reviewVersion ?? current?.id ?? null;
  const reviewRun = useMemo<ReviewRun | null>(() => runs.find((r) => r.versionId === selectedVersionId && (r.kind === "review_version" || r.kind === "revision_prompt") && r.status === "completed") ?? null, [runs, selectedVersionId]);
  const teacherRun = runs.find((r) => r.kind === "review_teacher" && r.status === "completed" && r.versionId === current?.id) ?? null;
  const prompts = wf.prompts.data ?? [];
  const pendingPrompt = [...prompts].reverse().find((p) => !p.sentAt) ?? null;
  const promptForRun = (runId: string) => prompts.find((p) => p.reviewRunId === runId) ?? null;
  const confirmations = wf.workflow.data?.styleConfirmations ?? [];
  useEffect(() => setPromptEdit(null), [pendingPrompt?.id]);

  if (wf.homework.error && !h) return <ErrorState message={wf.homework.error} onRetry={() => void wf.reloadAll()} />;
  if (wf.homework.loading && !h) return <SkeletonRows count={6} />;
  if (!h) return <EmptyState icon={<Flag />} title="ДЗ не найдено" action={<Link className="btn" to="/homework">К списку</Link>} />;

  const tabNow: Tab = tab ?? getWorkflowPresentation(h.workflowStage, wf.readiness.data?.ready).tab;
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
  const reviewer = rules.data ? providerName(rules.data.workflow.tasks.solution_review) : "проверяющий";
  const analyst = rules.data ? providerName(rules.data.workflow.tasks.material_analysis) : "аналитик";
  /**
   * Этап 9: шаг всегда начинается одинаково — запуск фиксирует снимок источников и
   * ждёт; передача провайдеру (CLI или вручную) или ручной ввод выбираются в мониторе.
   */
  const run = (kind: RunKind, versionId: string | null, b = kind === "analyze_materials" ? bundle : null) =>
    actions.startRun(kind, versionId, b && b.complete && b.purpose === kind && (kind === "analyze_materials" || b.versionId === versionId) ? b : null, "provider");
  const afterSaved = (r: ReviewRun) => setTab(r.kind === "analyze_materials" ? "cycle" : r.kind === "review_teacher" ? "teacher" : "review");
  const stepActions = (r: ReviewRun) => (
    <AiStepActions run={r} jobs={jobs} rules={rules.data} providers={providers.data} actions={actions} onSaved={afterSaved} />
  );
  const transitions = (wf.workflow.data?.transitions ?? []).filter((t) => t.allowed && t.to !== "ready_to_submit" && t.to !== "submitted");
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
      const res = await bridge.createHomeworkVersion(h.id, fileId, { author: "user", ...actions.options() });
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
      const res = await bridge.markVersionFinal(finalTarget.versionId, actions.options());
      setFinalTarget(null);
      toast.push({
        tone: "ok",
        title: res.operation.title,
        text: res.value.finalFileName ? `03 На сдачу/${res.value.finalFileName}` : `${finalTarget.label} — текущая проверяемая версия`,
        action: res.operation.undoable ? { label: "Отменить", run: () => void undo(res.operation.id) } : undefined,
      });
    } catch (e) {
      reportError(e, "Не удалось назначить итоговую");
    } finally {
      setBusy(false);
    }
  };

  const handoff = activeRun && activeJob?.status !== "running" && (activeRun.status === "waiting_for_provider" || activeRun.status === "manual_handoff") ? stepActions(activeRun) : undefined;

  const promptActions = (p: NonNullable<typeof pendingPrompt>) =>
    p.status === "draft" ? (
      <>
        {promptEdit !== null && promptEdit !== p.text && (
          <button className="btn btn--sm" disabled={!!actions.busy} onClick={() => void actions.updatePrompt(p.id, promptEdit)}>
            Сохранить черновик
          </button>
        )}
        <button className="btn btn--primary btn--sm" disabled={!!actions.busy || (promptEdit !== null && promptEdit !== p.text)} onClick={() => void actions.commitPrompt(p.id)}>
          <Lock /> Зафиксировать промт
        </button>
      </>
    ) : (
      <>
        <button className="btn btn--primary btn--sm" disabled={!!actions.busy || !!activeRun} onClick={() => void actions.sendPrompt(p.id, true)}>
          <Send /> Отметить: передан автору
        </button>
        {rules.data && (
          <button className="btn btn--sm" onClick={() => void actions.openApp(rules.data!.workflow.roles.author)}>
            <ExternalLink /> Открыть {author}
          </button>
        )}
      </>
    );

  /** Кандидат для промта исправлений: последняя проверка текущей версии с проблемами и без промта. */
  const fixSource = [teacherRun, reviewRun].find(
    (r) => r && r.versionId === current?.id && (r.result?.verdict === "needs_fixes" || r.result?.verdict === "blocked" || !!r.result?.prompt) && !promptForRun(r.id),
  );

  /** Следующий шаг — одно главное действие для текущего этапа. Доступность решает backend. */
  const nextStep = (() => {
    if (activeRun || actions.busy === "send") return null;
    switch (h.workflowStage) {
      case "materials_collected":
        return (
          <button className="btn btn--primary" disabled={!!actions.busy} onClick={() => void run("analyze_materials", null)}>
            <BrainCircuit /> Анализ · {analyst}
          </button>
        );
      case "codex_analysis":
      case "prompt_ready":
      case "needs_fixes":
        if (pendingPrompt) return pendingPrompt.status === "draft" ? <button className="btn btn--primary" onClick={() => setTab("cycle")}>Проверить и зафиксировать промт <ArrowRight /></button> : promptActions(pendingPrompt);
        if (h.workflowStage === "needs_fixes" && fixSource)
          return (
            <>
              <AiStepActions
                run={fixSource}
                task="revision_prompt"
                handoffOnly
                label={`Промт исправлений · ${rules.data ? providerName(rules.data.workflow.tasks.revision_prompt) : "провайдер"}`}
                jobs={jobs}
                rules={rules.data}
                providers={providers.data}
                actions={actions}
                onSaved={() => setTab("cycle")}
              />
              <button className="btn btn--sm" disabled={!!actions.busy} onClick={() => void actions.createPrompt(fixSource.id)}>
                <Wand2 /> Собрать из замечаний
              </button>
            </>
          );
        if (h.workflowStage === "codex_analysis")
          return (
            <button className="btn btn--primary" disabled={!!actions.busy} onClick={() => void run("analyze_materials", null)}>
              <BrainCircuit /> Повторить анализ
            </button>
          );
        return null;
      case "claude_solving":
        return (
          <button className="btn btn--primary" disabled={locked} onClick={() => void openAdd()}>
            <FilePlus /> Добавить версию решения
          </button>
        );
      case "codex_review":
        return current ? (
          <button className="btn btn--primary" disabled={!!actions.busy} onClick={() => void run("review_version", current.id).then(() => setTab("review"))}>
            <ScanSearch /> Проверить {current.label} · {reviewer}
          </button>
        ) : null;
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
            {h.deadline ? `Дедлайн ${formatDateTime(h.deadline)} · ${deadline.text}` : `Дедлайн: ${deadline.text}`}
          </p>
        </div>
        <div className="pagehead__actions">
          <button className="btn" onClick={() => void bridge.openObsidianNote(`${h.folderPath}/Карточка ДЗ.md`).catch((error: unknown) => reportError(error, "Заметка Obsidian не открыта"))}>
            <NotebookPen /> Obsidian
          </button>
          <Link to={`/ai?homework=${h.id}`} className="btn">
            <BrainCircuit /> AI Workspace
          </Link>
        </div>
      </header>

      {wf.workflow.error && (
        <div className="notice notice--warn wf__stale" role="status">
          <RefreshCw />
          <div>
            <strong>Показано последнее загруженное состояние — оно могло устареть.</strong> {wf.workflow.error}
          </div>
          <button className="btn btn--sm notice__action" onClick={() => void wf.reloadAll()}>
            <RefreshCw /> Перечитать
          </button>
        </div>
      )}

      <section className="panel wf">
        <WorkflowRail homeworkId={h.id} stage={h.workflowStage} cycle={cycle} blockedStages={blockedStages} />
        <motion.div className="wf__next" key={h.workflowStage} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}>
          <div className="wf__now">
            <span className="eyebrow">Сейчас · {WORKFLOW_STAGE_LABEL[h.workflowStage]}</span>
            <p>{reason ?? DEFAULT_REASON[h.workflowStage]}</p>
          </div>
          <div className="wf__actions">{nextStep}</div>
        </motion.div>
        {transitions.length > 0 && !locked && (
          <div className="wf__transitions" aria-label="Другие допустимые переходы">
            {transitions.map((t) => (
              <button key={t.to} className="btn btn--sm btn--ghost" title={t.reason} disabled={!!actions.busy} onClick={() => void actions.transition(t.to)}>
                {WORKFLOW_STAGE_ORDER.indexOf(t.to) < WORKFLOW_STAGE_ORDER.indexOf(h.workflowStage) ? (
                  <>
                    <Undo2 /> Вернуться: {WORKFLOW_STAGE_LABEL[t.to].toLowerCase()}
                  </>
                ) : (
                  <>
                    <ArrowRight /> Перейти: {WORKFLOW_STAGE_LABEL[t.to].toLowerCase()}
                  </>
                )}
              </button>
            ))}
          </div>
        )}
        <RunMonitor run={activeRun} job={activeJob} handoff={handoff} onCancel={(id) => void actions.cancelRun(id)} onCancelJob={(id) => void actions.cancelJob(id)} />
        {h.workflowStage === "claude_solving" && !activeRun && (
          <div className="faint wf__solving">
            Решение создаёт автор ({author}) в официальном приложении — «Контур» не управляет им. Получите файл и добавьте его как новую версию.
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
          { value: "cycle", label: "Рабочий цикл", badge: <span className="tabs__n num" aria-label={`${h.versions.length} версий`}>{h.versions.length}</span> },
          { value: "review", label: "Результат проверки" },
          { value: "teacher", label: "Комментарии преподавателя", badge: <span className="tabs__n num" aria-label={`${comments.length} комментариев`}>{comments.length}</span> },
          { value: "package", label: "Комплект на сдачу" },
        ]}
      />

      <motion.div key={tabNow} id={`homework-sections-panel-${tabNow}`} aria-labelledby={`homework-sections-tab-${tabNow}`} role="tabpanel" tabIndex={0} className="hw__tab" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.2 }}>
        {tabNow === "materials" && (
          <section className="panel">
            <div className="panel__head">
              <h2 className="panel__title">Исходные материалы</h2>
              <div className="panel__meta">
                <span className="faint">{!demo ? "Снимок подборки материалов; изменить состав — в подборке на экране «Материалы»" : "Снимите отметку, чтобы не передавать файл"}</span>
                {!locked && (
                  <button className="btn btn--sm" disabled={!!actions.busy} onClick={() => void actions.prepareBundle("analyze_materials", null)}>
                    <Layers /> Новый снимок из подборки
                  </button>
                )}
              </div>
            </div>
            <div className="panel__body">
              {wf.workflow.loading && !bundle ? (
                <SkeletonRows count={5} />
              ) : !bundle ? (
                <EmptyState
                  icon={<Layers />}
                  title="Снимка источников ещё нет"
                  text="Выберите материалы ДЗ в подборке (экран «Материалы»), затем создайте снимок — он зафиксирует файлы, SHA-256 и извлечённый текст для запуска."
                  action={<Link className="btn btn--sm" to="/materials">К подборке материалов</Link>}
                />
              ) : (
                <SourceList
                  bundle={bundle}
                  busy={locked}
                  onToggle={locked || !demo ? undefined : (id, inc) => void bridge.setSourceIncluded(bundle.id, id, inc).then(() => wf.reloadAll(), (e: unknown) => reportError(e))}
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
                    <div className="eyebrow">{pendingPrompt.status === "draft" ? "Черновик промта — проверьте и зафиксируйте" : "Ожидает передачи автору"}</div>
                    <PromptBox prompt={pendingPrompt} editable={pendingPrompt.status === "draft"} onChange={setPromptEdit} actions={promptActions(pendingPrompt)} compact />
                    {pendingPrompt.status === "committed" && <p className="faint">Скопируйте промт и передайте автору сами. Когда решение будет готово, добавьте его как новую версию.</p>}
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
                  <button
                    className="btn btn--sm btn--primary"
                    title={selectedVersionId !== current?.id ? "Проверяется только текущая выбранная версия" : undefined}
                    disabled={!!activeRun || !!actions.busy || locked || h.workflowStage !== "codex_review" || selectedVersionId !== current?.id}
                    onClick={() => void run("review_version", selectedVersionId)}
                  >
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
                      {reviewRun.provenance === "manual" ? "Вручную" : providerName(reviewRun.provider)} · {formatDateTime(reviewRun.startedAt)} · порог стиля {reviewRun.styleThreshold}
                    </span>
                    {reviewRun.assessment?.styleScore != null && reviewRun.assessment.styleScore > reviewRun.styleThreshold && reviewRun.versionId === current?.id && (
                      confirmations.some((c) => c.reviewRunId === reviewRun.id) ? (
                        <span className="badge badge--warn">
                          <ShieldCheck /> расхождение подтверждено вами
                        </span>
                      ) : (
                        !locked && (
                          <button className="btn btn--sm" disabled={!!actions.busy} onClick={() => void actions.confirmStyle(reviewRun.id)} title="Расхождение с личным стилем выше порога. Подтвердите, что текст ваш и вы его проверили.">
                            <ShieldCheck /> Подтвердить расхождение со стилем
                          </button>
                        )
                      )
                    )}
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
                      !locked && (reviewRun.result?.prompt || ((reviewRun.result?.verdict === "needs_fixes" || reviewRun.result?.verdict === "blocked") && h.workflowStage === "needs_fixes")) ? (
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
              canReview={!!current && h.workflowStage === "teacher_review"}
              onReview={() => current && void run("review_teacher", current.id)}
              onChanged={() => void wf.reloadAll()}
              commandOptions={actions.options}
              onAddRequirement={(input) => actions.addRequirement(input).then((r) => r !== null)}
              onResolve={(id, status) => void actions.resolveRequirement(id, status)}
            />
          </>
        )}

        {tabNow === "package" && (
          <SubmissionPanel
            homework={h}
            readiness={wf.readiness.data}
            pkg={wf.pkg.data}
            receipt={wf.receipt.data}
            commentsCount={comments.length}
            onChanged={() => void wf.reloadAll()}
            readyAllowed={wf.workflow.data?.transitions.find((t) => t.to === "ready_to_submit")?.allowed ?? false}
            commandOptions={actions.options}
          />
        )}
      </motion.div>

      <Dialog open={addOpen} onClose={() => setAddOpen(false)} title="Добавить версию вручную" description="Файл из «02 Работа» станет новой версией (автор — вы). Существующие версии не изменяются.">
        {!candidates ? (
          <SkeletonRows count={2} />
        ) : candidates.length === 0 ? (
          <EmptyState icon={<FilePlus />} title="Нет новых файлов решения" text="Положите файл решения в «02 Работа» этого ДЗ — он появится здесь." />
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
        <p className="muted">{!demo ? "Версия станет текущей проверяемой. Копия в «03 На сдачу» появится при сборке комплекта под именем:" : "Копия ляжет в «03 На сдачу» под именем:"}</p>
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
