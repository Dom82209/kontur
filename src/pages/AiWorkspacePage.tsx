import { AnimatePresence, motion } from "motion/react";
import { BrainCircuit, CircleAlert, CircleCheck, ExternalLink, FilePlus2, GitCompareArrows, MessagesSquare, Pin, Play, ScanSearch, Wand2 } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useOps } from "../app/operations";
import { ProviderCard } from "../components/domain/ProviderCard";
import { ReviewResultView } from "../components/domain/ReviewResultView";
import { RunMonitor } from "../components/domain/RunMonitor";
import { SourceList } from "../components/domain/SourceList";
import { VersionComparePanel } from "../components/domain/VersionComparePanel";
import { Dialog } from "../components/ui/Dialog";
import { EmptyState, SkeletonRows } from "../components/ui/States";
import { bridge, type ReviewRun, type RunKind, type SourceBundle, type WorkflowRole } from "../lib/bridge";
import { formatRelative, providerName, ROLE_LABEL, RUN_KIND_LABEL, WORKFLOW_STAGE_LABEL } from "../lib/format";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
import { useHomeworkWorkflow } from "../lib/hooks/useHomeworkWorkflow";
import { useSubjectNames } from "../lib/hooks/useSubjectNames";
import { useWorkflowActions } from "../lib/hooks/useWorkflowActions";

const KINDS: { kind: RunKind; role: WorkflowRole; icon: ReactNode; hint: string }[] = [
  { kind: "analyze_materials", role: "analyst", icon: <BrainCircuit />, hint: "Разбор материалов и первичный промт для автора" },
  { kind: "review_version", role: "reviewer", icon: <ScanSearch />, hint: "Решение, условие, опора на материалы, стиль, файлы" },
  { kind: "revision_prompt", role: "reviewer", icon: <Wand2 />, hint: "Промт по всем открытым замечаниям и требованиям" },
  { kind: "review_teacher", role: "reviewer", icon: <MessagesSquare />, hint: "Сверка версии с каждым ответом преподавателя" },
];

export function AiWorkspacePage() {
  const [params, setParams] = useSearchParams();
  const { reportError } = useOps();
  const names = useSubjectNames();
  const list = useBridgeQuery(() => bridge.getHomeworkList(), [], ["workflow:stage-changed"]);
  const rules = useBridgeQuery(() => bridge.getAutomationRules(), [], ["operation:completed"]);
  const providers = useBridgeQuery(() => Promise.all([bridge.getProviderStatus("codex"), bridge.getProviderStatus("claude")]), [], ["provider:status-changed"]);

  const active = list.data?.filter((h) => h.stage !== "submitted") ?? [];
  const homeworkId = params.get("homework") ?? active[0]?.id ?? "";
  const kind = (params.get("run") as RunKind | null) ?? "review_version";
  const wf = useHomeworkWorkflow(homeworkId);
  const actions = useWorkflowActions(homeworkId);
  const h = wf.homework.data;
  const latest = h?.versions[h.versions.length - 1] ?? null;
  const [versionId, setVersionId] = useState<string | null>(null);
  const vId = kind === "analyze_materials" ? null : (versionId && h?.versions.some((v) => v.id === versionId) ? versionId : (latest?.id ?? null));
  const [bundle, setBundle] = useState<SourceBundle | null>(null);
  const [bundleError, setBundleError] = useState<string | null>(null);
  const [compareOpen, setCompareOpen] = useState(false);
  const [promptDraft, setPromptDraft] = useState<string | null>(null);

  const set = (k: string, v: string) =>
    setParams((p) => {
      p.set(k, v);
      return p;
    });

  // Набор источников зависит от ДЗ, типа запуска и версии.
  useEffect(() => {
    if (!homeworkId) return;
    let alive = true;
    setBundle(null);
    setBundleError(null);
    bridge.prepareSourceBundle(homeworkId, kind, vId).then(
      (b) => alive && setBundle(b),
      (e: unknown) => alive && setBundleError(e instanceof Error ? e.message : "Ошибка"),
    );
    return () => {
      alive = false;
    };
  }, [homeworkId, kind, vId, wf.comments.data?.length]);

  const runs = wf.runs.data ?? [];
  const running = runs.find((r) => r.status === "running" || r.status === "queued") ?? null;
  const result = useMemo<ReviewRun | null>(() => runs.find((r) => r.kind === kind && r.status === "completed" && (kind === "analyze_materials" || r.versionId === vId)) ?? null, [runs, kind, vId]);
  useEffect(() => setPromptDraft(null), [result?.id]);

  const role = KINDS.find((k) => k.kind === kind)!.role;
  const providerId = rules.data?.workflow.roles[role];
  const authorId = rules.data?.workflow.roles.author;
  const status = providers.data?.find((p) => p.provider === providerId);
  const savedPrompt = result ? (wf.prompts.data ?? []).find((p) => p.reviewRunId === result.id) ?? null : null;
  const prevVersion = h && vId ? h.versions[h.versions.findIndex((v) => v.id === vId) - 1] ?? null : null;
  const needsVersion = kind !== "analyze_materials";
  const blockedReason = !h ? "Выберите ДЗ" : needsVersion && !vId ? "У этого ДЗ ещё нет версий" : kind === "review_teacher" && (wf.comments.data?.length ?? 0) === 0 ? "Добавьте комментарии преподавателя в карточке ДЗ" : bundle && !bundle.complete ? "Не хватает обязательных материалов" : status?.state !== "connected" ? `${providerName(providerId ?? "codex")} не подключён` : running ? "Идёт другой запуск" : null;

  const createVersion = async () => {
    if (!result) return;
    const p = savedPrompt ?? (await actions.createPrompt(result.id, promptDraft ?? undefined));
    if (p && !p.sentAt) await actions.sendPrompt(p.id);
  };

  return (
    <div className="ai">
      <header className="pagehead">
        <div>
          <span className="eyebrow">AI Workspace</span>
          <h1 className="pagehead__title">Запуски рабочего цикла</h1>
          <p className="muted">Провайдер определяется ролью в настройках. Адаптеры работают через backend: вход выполняется только в официальных приложениях, пароли и токены здесь не вводятся.</p>
        </div>
      </header>

      <div className="ai2">
        <div className="ai2__side">
          <section className="panel">
            <div className="panel__body ai2__config">
              <label className="field">
                <span className="field__label">Домашнее задание</span>
                <select className="select" value={homeworkId} onChange={(e) => set("homework", e.target.value)}>
                  {active.map((x) => (
                    <option key={x.id} value={x.id}>
                      {names(x.subjectId)} · нед. {x.weekNumber} · {x.title}
                    </option>
                  ))}
                </select>
              </label>
              {h && (
                <div className="ai2__stage faint">
                  Этап: <strong>{WORKFLOW_STAGE_LABEL[h.workflowStage]}</strong> ·{" "}
                  <Link to={`/homework/${h.id}`} className="rule__link">
                    карточка ДЗ
                  </Link>
                </div>
              )}
              <div className="runkinds" role="radiogroup" aria-label="Тип запуска">
                {KINDS.map((k) => (
                  <button key={k.kind} role="radio" aria-checked={kind === k.kind} className="runkind" onClick={() => set("run", k.kind)}>
                    {kind === k.kind && <motion.span layoutId="runkind-sel" className="runkind__sel" />}
                    <span className="runkind__icon">{k.icon}</span>
                    <span className="runkind__text">
                      <span className="runkind__title">{RUN_KIND_LABEL[k.kind]}</span>
                      <span className="runkind__hint">
                        {k.hint} · {ROLE_LABEL[k.role].toLowerCase()}: {rules.data ? providerName(rules.data.workflow.roles[k.role]) : "…"}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
              {needsVersion && h && h.versions.length > 0 && (
                <label className="field">
                  <span className="field__label">Версия</span>
                  <select className="select" value={vId ?? ""} onChange={(e) => setVersionId(e.target.value)}>
                    {h.versions.map((v) => (
                      <option key={v.id} value={v.id}>
                        {v.label} · {v.author === "user" ? "вы" : providerName(v.author)}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {status && <ProviderCard status={status} compact />}
            </div>
          </section>

          <section className="panel">
            <div className="panel__head">
              <h2 className="panel__title">Будут переданы</h2>
              {bundle && <span className="panel__meta num">{bundle.items.filter((i) => i.included).length}/{bundle.items.length}</span>}
            </div>
            <div className="panel__body">
              {bundleError ? (
                <p className="faint">{bundleError}</p>
              ) : !bundle ? (
                <SkeletonRows count={4} />
              ) : (
                <SourceList bundle={bundle} onToggle={(id, inc) => void bridge.setSourceIncluded(bundle.id, id, inc).then(setBundle, (e: unknown) => reportError(e))} />
              )}
            </div>
          </section>
        </div>

        <div className="ai2__main">
          <section className="panel ai2__run">
            <div className="ai2__runbar">
              <div>
                <div className="ai2__runtitle">{RUN_KIND_LABEL[kind]}</div>
                <div className="faint" style={{ fontSize: 12 }}>
                  {blockedReason ?? `${providerName(providerId ?? "codex")} получит ${bundle?.items.filter((i) => i.included).length ?? 0} источников. Результат сохранит список использованных материалов.`}
                </div>
              </div>
              <button className="btn btn--primary" disabled={!!blockedReason || !!actions.busy} onClick={() => void actions.startRun(kind, vId, bundle)}>
                <Play /> Запустить
              </button>
            </div>
            <RunMonitor run={running} />
          </section>

          <section className="panel">
            <div className="panel__head">
              <h2 className="panel__title">Результат</h2>
              {result && <span className="panel__meta faint">{formatRelative(result.finishedAt ?? result.startedAt)}</span>}
            </div>
            <div className="panel__body">
              {!result ? (
                <EmptyState icon={<BrainCircuit />} title="Запусков этого типа ещё не было" text="Выберите источники и нажмите «Запустить». Результат появится здесь вместе с готовым промтом." />
              ) : (
                <>
                  <div className="review__toolbar">
                    {result.savedAsIteration ? (
                      <span className="badge badge--ok">
                        <Pin /> сохранено как итерация
                      </span>
                    ) : (
                      <button className="btn btn--sm" disabled={!!actions.busy} onClick={() => void actions.saveIteration(result.id)}>
                        <Pin /> Сохранить как итерацию
                      </button>
                    )}
                    {prevVersion && kind !== "analyze_materials" && (
                      <button className="btn btn--sm btn--ghost" onClick={() => setCompareOpen(true)}>
                        <GitCompareArrows /> Сравнить с предыдущей
                      </button>
                    )}
                  </div>
                  <ReviewResultView
                    run={result}
                    comments={wf.comments.data ?? []}
                    onPromptChange={savedPrompt || kind === "analyze_materials" ? undefined : setPromptDraft}
                    promptActions={
                      result.result?.prompt && authorId ? (
                        <>
                          <button className="btn btn--sm" onClick={() => void actions.openApp(authorId)}>
                            <ExternalLink /> Открыть {providerName(authorId)}
                          </button>
                          {kind === "analyze_materials" ? (
                            (() => {
                              const p = (wf.prompts.data ?? []).find((x) => x.reviewRunId === result.id);
                              return p && !p.sentAt ? (
                                <button className="btn btn--sm btn--primary" disabled={!!actions.busy || !!running} onClick={() => void actions.sendPrompt(p.id)}>
                                  <FilePlus2 /> Создать новую версию
                                </button>
                              ) : (
                                <span className="badge badge--ok">промт передан</span>
                              );
                            })()
                          ) : savedPrompt?.sentAt ? (
                            <span className="badge badge--ok">по промту создана версия</span>
                          ) : (
                            <button className="btn btn--sm btn--primary" disabled={!!actions.busy || !!running} onClick={() => void createVersion()}>
                              <FilePlus2 /> Создать новую версию
                            </button>
                          )}
                        </>
                      ) : undefined
                    }
                  />
                </>
              )}
            </div>
          </section>

          <section className="panel">
            <div className="panel__head">
              <h2 className="panel__title">История запусков</h2>
            </div>
            <div className="panel__body">
              {runs.length === 0 ? (
                <p className="faint">По этому ДЗ запусков ещё не было.</p>
              ) : (
                <ul className="runs">
                  <AnimatePresence initial={false}>
                    {runs.map((r) => (
                      <motion.li key={r.id} layout className="run" data-status={r.status} initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }}>
                        <div className="run__head">
                          {r.status === "completed" ? <CircleCheck size={14} className="run__ok" /> : r.status === "failed" || r.status === "cancelled" ? <CircleAlert size={14} className="run__fail" /> : <span className="dot dot--accent" />}
                          <span className="run__provider">{providerName(r.provider)}</span>
                          <span>{RUN_KIND_LABEL[r.kind]}</span>
                          {r.versionId && <span className="mono faint">{h?.versions.find((v) => v.id === r.versionId)?.label}</span>}
                          {r.savedAsIteration && <Pin size={12} className="faint" aria-label="Сохранено как итерация" />}
                          <span className="faint num" style={{ marginLeft: "auto" }}>
                            {formatRelative(r.startedAt)}
                          </span>
                        </div>
                        {r.result && <div className="faint">{r.result.summary}</div>}
                        {r.status === "cancelled" && <div className="faint">Отменено</div>}
                        {r.errorMessage && <div className="run__error">{r.errorMessage}</div>}
                        <div className="faint run__time">Источников: {r.sourcesUsed.length}</div>
                      </motion.li>
                    ))}
                  </AnimatePresence>
                </ul>
              )}
            </div>
          </section>
        </div>
      </div>

      <Dialog open={compareOpen} onClose={() => setCompareOpen(false)} title={`Сравнение ${prevVersion?.label ?? ""} → ${h?.versions.find((v) => v.id === vId)?.label ?? ""}`} wide>
        {prevVersion && vId && <VersionComparePanel fromId={prevVersion.id} toId={vId} />}
      </Dialog>
    </div>
  );
}
