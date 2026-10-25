import { ClipboardCopy, ClipboardPaste, ExternalLink, FileUp, ListChecks, Send, Square, Trash2, TriangleAlert } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useToast } from "../../app/toasts";
import {
  AI_TASK_LABEL,
  bridge,
  isOpenJob,
  type AiJob,
  type AiTask,
  type AutomationRules,
  type ProviderStatus,
  type ReviewRun,
} from "../../lib/bridge";
import { copyText } from "../../lib/clipboard";
import { formatElapsed, JOB_STATUS_LABEL, PARSE_STATUS_LABEL, plural, providerName } from "../../lib/format";
import { useBridgeEvent } from "../../lib/hooks/useBridgeEvent";
import type { useWorkflowActions } from "../../lib/hooks/useWorkflowActions";
import { Dialog } from "../ui/Dialog";

type Actions = ReturnType<typeof useWorkflowActions>;

interface Props {
  open: boolean;
  onClose: () => void;
  run: ReviewRun | null;
  task: AiTask;
  onTaskChange?: (task: AiTask) => void;
  /** Задания ДЗ (без текстов) — для поиска уже подготовленной передачи. */
  jobs: AiJob[];
  rules: AutomationRules | null;
  providers: ProviderStatus[] | null;
  actions: Actions;
  /** Открыть форму проверки ответа (анализ/проверка) с черновиком. */
  onReview: (job: AiJob, supporting: AiJob | null) => void;
  /** Промт исправлений подтверждён — закрыть и показать результат. */
  onConfirmed?: () => void;
}

const fmtChars = (n: number) => `${n.toLocaleString("ru-RU")} ${plural(n, "символ", "символа", "символов")}`;

/**
 * Передача шага провайдеру (этап 9). Пользователь видит весь текст и список файлов
 * до отправки. Официальный CLI запускается только кнопкой «Отправить»; при ручной
 * передаче промт копируется только кнопкой, приложение/сайт открывается только кнопкой,
 * ответ вставляется или импортируется из файла. Ответ сам ничего не меняет — его
 * нужно проверить и подтвердить.
 */
export function AiHandoffDialog({ open, onClose, run, task, onTaskChange, jobs, rules, providers, actions, onReview, onConfirmed }: Props) {
  const toast = useToast();
  const [job, setJob] = useState<AiJob | null>(null);
  const [loading, setLoading] = useState(false);
  const [answer, setAnswer] = useState("");
  const [promptEdit, setPromptEdit] = useState<string | null>(null);
  const [live, setLive] = useState<{ elapsedMs: number; outputBytes: number } | null>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);

  const provider = rules?.workflow.tasks[task] ?? null;
  const status = providers?.find((p) => p.provider === provider) ?? null;
  const connected = status?.availability === "connected";
  const summary = run ? [...jobs].reverse().find((j) => j.runId === run.id && j.task === task && isOpenJob(j)) ?? null : null;

  const load = useCallback(
    async (id: string) => {
      try {
        setJob(await bridge.getAiJob(id));
      } catch {
        setJob(null);
      }
    },
    [],
  );

  useEffect(() => {
    if (!open) return;
    setAnswer("");
    setPromptEdit(null);
    setLive(null);
    if (summary) void load(summary.id);
    else setJob(null);
    // Перечитывается при открытии и при смене задачи; дальнейшие изменения — по событиям.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, task, run?.id, summary?.id]);

  useBridgeEvent("ai-job:updated", (e) => {
    if (open && job && e.jobId === job.id) void load(job.id);
  });
  useBridgeEvent("ai-job:progress", (e) => {
    if (open && job && e.jobId === job.id) setLive({ elapsedMs: e.elapsedMs, outputBytes: e.outputBytes });
  });

  if (!run) return null;

  const prepare = async (transport: "auto" | "manual_handoff") => {
    setLoading(true);
    const prepared = await actions.prepareJob(run.id, task, transport);
    setLoading(false);
    if (prepared) setJob(prepared);
  };

  const send = async () => {
    if (!job) return;
    const sent = await actions.sendJob(job.id);
    if (sent) setJob(sent);
  };

  const copy = async () => {
    if (!job?.promptText) return;
    const ok = await copyText(job.promptText, promptRef.current);
    toast.push(ok ? { tone: "ok", title: "Промт скопирован", text: `${fmtChars(job.promptChars)} — вставьте его в ${providerName(job.provider)}` } : { tone: "warn", title: "Не удалось скопировать", text: "Выделите текст в поле и скопируйте вручную (⌘C)" });
  };

  const submit = async () => {
    if (!job || !answer.trim()) return;
    const saved = await actions.submitResponse(job.id, answer);
    if (saved) {
      setJob(saved);
      setAnswer("");
    }
  };

  const importFile = async () => {
    if (!job) return;
    const saved = await actions.importResponse(job.id);
    if (saved) setJob(saved);
  };

  const discard = async () => {
    if (!job) return;
    const closed = await actions.discardJob(job.id);
    if (closed) setJob(null);
  };

  const confirmPrompt = async () => {
    if (!job) return;
    const text = (promptEdit ?? job.draft?.text ?? "").trim();
    if (!text) return;
    const done = await actions.confirmAi(job.id, task, { kind: "prompt", value: { text } });
    if (done) {
      onConfirmed?.();
      onClose();
    }
  };

  const analysisJob = task === "initial_prompt" ? [...jobs].reverse().find((j) => j.runId === run.id && j.task === "material_analysis" && j.status === "response_ready") ?? null : null;
  const promptProvider = rules?.workflow.tasks.initial_prompt;
  const needsSeparatePrompt = task === "material_analysis" && job?.status === "response_ready" && !job.draft?.promptText && promptProvider !== rules?.workflow.tasks.material_analysis;
  const busy = !!actions.busy || loading;
  const included = job?.context.filter((s) => s.charsIncluded > 0).length ?? 0;

  const footer = !job ? (
    <>
      <button className="btn btn--ghost" onClick={onClose}>
        Закрыть
      </button>
      <button className="btn" disabled={busy} onClick={() => void prepare("manual_handoff")}>
        <ClipboardPaste /> Подготовить ручную передачу
      </button>
      {connected && (
        <button className="btn btn--primary" disabled={busy} onClick={() => void prepare("auto")}>
          <ListChecks /> Подготовить для {providerName(provider!)} CLI
        </button>
      )}
    </>
  ) : job.status === "prepared" && job.transport === "cli" ? (
    <>
      <button className="btn btn--ghost" disabled={busy} onClick={() => void discard()}>
        <Trash2 /> Отменить передачу
      </button>
      <button className="btn btn--primary" disabled={busy} onClick={() => void send()}>
        <Send /> Отправить в {providerName(job.provider)} CLI
      </button>
    </>
  ) : job.status === "running" ? (
    <button className="btn" disabled={busy} onClick={() => void actions.cancelJob(job.id)}>
      <Square /> Остановить
    </button>
  ) : job.status === "response_ready" && task !== "revision_prompt" ? (
    <>
      <button className="btn btn--ghost" disabled={busy} onClick={() => void discard()}>
        <Trash2 /> Отклонить ответ
      </button>
      {task === "initial_prompt" ? (
        <button className="btn btn--primary" disabled={busy || !analysisJob} title={analysisJob ? undefined : "Нет полученного итога анализа"} onClick={() => analysisJob && onReview(analysisJob, job)}>
          <ListChecks /> Проверить анализ с этим промтом
        </button>
      ) : (
        <button className="btn btn--primary" disabled={busy} onClick={() => onReview(job, null)}>
          <ListChecks /> Проверить и подтвердить…
        </button>
      )}
    </>
  ) : job.status === "response_ready" ? (
    <>
      <button className="btn btn--ghost" disabled={busy} onClick={() => void discard()}>
        <Trash2 /> Отклонить ответ
      </button>
      <button className="btn btn--primary" disabled={busy || !(promptEdit ?? job.draft?.text ?? "").trim()} onClick={() => void confirmPrompt()}>
        <ListChecks /> Подтвердить как черновик промта
      </button>
    </>
  ) : (
    <button className="btn btn--ghost" onClick={onClose}>
      Закрыть
    </button>
  );

  return (
    <Dialog
      open={open}
      onClose={onClose}
      extraWide
      title={`${AI_TASK_LABEL[task]} · ${provider ? providerName(provider) : "провайдер"}`}
      description={
        connected
          ? "Официальный CLI подключён. Материалы уйдут только после кнопки «Отправить» — ниже весь передаваемый текст."
          : "CLI не подключён — ручная передача: скопируйте промт кнопкой, получите ответ в официальном приложении и вставьте его сюда."
      }
      footer={footer}
    >
      <div className="handoff">
        {onTaskChange && run.kind === "analyze_materials" && (
          <div className="segmented handoff__tasks" role="radiogroup" aria-label="Задача">
            {(["material_analysis", "initial_prompt"] as const).map((t) => (
              <button key={t} role="radio" aria-checked={task === t} className="btn btn--sm" data-active={task === t} onClick={() => onTaskChange(t)}>
                {AI_TASK_LABEL[t]} · {rules ? providerName(rules.workflow.tasks[t]) : "…"}
              </button>
            ))}
          </div>
        )}

        {!job ? (
          <div className="handoff__empty">
            <p>
              Передача ещё не подготовлена. «Контур» соберёт текст из снимка источников этого шага (извлечённый текст материалов{run.versionId ? " и решения" : ""}), ограничит объём и покажет всё целиком. Ничего не отправляется до вашего явного действия.
            </p>
            {status && !connected && status.cli.detected && !status.cli.enabled && (
              <p className="faint">Официальный CLI {providerName(status.provider)} найден, но не подключён — подключить его можно в Настройках.</p>
            )}
            {task === "initial_prompt" && !analysisJob && <div className="notice notice--warn">Сначала получите итог анализа материалов — первичный промт строится по нему.</div>}
          </div>
        ) : (
          <>
            <div className="handoff__meta">
              <span className="badge">{job.transport === "cli" ? `официальный CLI${job.adapterLabel ? ` · ${job.adapterLabel}` : ""}` : "ручная передача"}</span>
              <span className={`badge ${job.status === "response_ready" ? "badge--info" : job.status === "failed" ? "badge--danger" : ""}`}>{JOB_STATUS_LABEL[job.status]}</span>
              <span className="faint">
                {included} {plural(included, "файл", "файла", "файлов")} · {fmtChars(job.promptChars)} · лимит {fmtChars(job.limitChars)}
              </span>
            </div>
            {job.contextTruncated && (
              <div className="notice notice--warn">
                <TriangleAlert /> Материалы больше лимита — часть текста не передаётся (отмечено ниже и в самом тексте).
              </div>
            )}
            <section>
              <h3 className="handoff__h">Что будет передано</h3>
              <ul className="handoff__files">
                {job.context.map((s) => (
                  <li key={s.label}>
                    <span className="mono faint">[{s.label}]</span> <span className="handoff__name">{s.name}</span>
                    <span className="faint num">
                      {s.charsIncluded === s.charsTotal ? fmtChars(s.charsTotal) : `${s.charsIncluded.toLocaleString("ru-RU")} из ${fmtChars(s.charsTotal)}`}
                    </span>
                    {s.truncated && <span className="badge badge--warn">{s.charsIncluded === 0 ? "не вошёл" : "обрезан"}</span>}
                  </li>
                ))}
              </ul>
            </section>
            <section>
              <h3 className="handoff__h">Полный текст передачи</h3>
              <textarea ref={promptRef} className="textarea handoff__prompt mono" readOnly value={job.promptText ?? ""} aria-label="Полный текст передачи провайдеру" />
              {job.transport === "manual_handoff" && (job.status === "prepared" || job.status === "response_ready") && (
                <div className="handoff__row">
                  <button className="btn btn--sm" onClick={() => void copy()}>
                    <ClipboardCopy /> Скопировать промт
                  </button>
                  <button className="btn btn--sm" onClick={() => void actions.openApp(job.provider)}>
                    <ExternalLink /> {status?.handoff.appInstalled ? `Открыть приложение ${providerName(job.provider)}` : `Открыть ${providerName(job.provider)} в браузере`}
                  </button>
                </div>
              )}
              {job.transport === "cli" && job.status === "prepared" && (
                <p className="faint handoff__consent">
                  По кнопке «Отправить» этот текст уйдёт в {status?.accountLabel ?? `${providerName(job.provider)} CLI`} через стандартный ввод процесса. CLI запускается без shell, в пустом рабочем каталоге, без доступа к вашим файлам и со своим собственным входом — «Контур» не передаёт ему пароли и токены.
                </p>
              )}
            </section>

            {job.status === "running" && (
              <div className="notice">
                Процесс CLI выполняется · <span className="num">{formatElapsed(live?.elapsedMs ?? 0)}</span>. Можно закрыть окно — ответ придёт на проверку.
              </div>
            )}

            {(job.status === "prepared" || job.status === "response_ready") && job.transport === "manual_handoff" && (
              <section>
                <h3 className="handoff__h">Ответ провайдера</h3>
                <textarea className="textarea handoff__answer" value={answer} onChange={(e) => setAnswer(e.target.value)} placeholder="Вставьте сюда ответ из официального приложения" aria-label="Ответ провайдера" />
                <div className="handoff__row">
                  <button className="btn btn--sm btn--primary" disabled={busy || !answer.trim()} onClick={() => void submit()}>
                    <ClipboardPaste /> Разобрать ответ
                  </button>
                  {bridge.kind !== "mock" && (
                    <button className="btn btn--sm" disabled={busy} onClick={() => void importFile()}>
                      <FileUp /> Импортировать файл ответа…
                    </button>
                  )}
                </div>
              </section>
            )}

            {(job.status === "response_ready" || job.status === "failed" || job.status === "cancelled") && (
              <section>
                <h3 className="handoff__h">Результат разбора</h3>
                {job.parseStatus && (
                  <div className={`notice ${job.parseStatus === "valid" ? "notice--ok" : job.parseStatus === "partial" ? "notice--warn" : "notice--danger"}`}>
                    {PARSE_STATUS_LABEL[job.parseStatus]}
                    {job.responseSource === "file" && job.responseFileName ? ` · из файла «${job.responseFileName}»` : job.responseSource === "paste" ? " · вставлен вручную" : job.responseSource === "cli" ? " · получен из CLI" : ""}
                  </div>
                )}
                {job.errorMessage && <div className="notice notice--danger">{job.errorMessage}</div>}
                {job.issues.length > 0 && (
                  <ul className="handoff__issues">
                    {job.issues.map((issue, i) => (
                      <li key={`${issue.code}-${i}`}>{issue.message}</li>
                    ))}
                  </ul>
                )}
                {needsSeparatePrompt && onTaskChange && promptProvider && (
                  <div className="notice">
                    Промт для автора готовит {providerName(promptProvider)} (задача «{AI_TASK_LABEL.initial_prompt}»).
                    <button className="btn btn--sm notice__action" onClick={() => onTaskChange("initial_prompt")}>
                      Перейти к промту
                    </button>
                  </div>
                )}
                {task === "revision_prompt" && job.status === "response_ready" && (
                  <label className="field">
                    <span className="field__label">Промт исправлений — проверьте и при необходимости поправьте</span>
                    <textarea className="textarea handoff__answer" value={promptEdit ?? job.draft?.text ?? ""} onChange={(e) => setPromptEdit(e.target.value)} />
                    <span className="field__hint">Сохранится черновиком: его ещё можно поправить и затем зафиксировать.</span>
                  </label>
                )}
                {job.responseText !== null && (
                  <details className="handoff__raw">
                    <summary>Сырой ответ ({fmtChars(job.responseChars)})</summary>
                    <pre className="mono">{job.responseText}</pre>
                  </details>
                )}
              </section>
            )}

            {job.log.length > 0 && (
              <details className="handoff__log">
                <summary>Журнал запуска</summary>
                <ol>
                  {job.log.map((entry, i) => (
                    <li key={i} data-level={entry.level}>
                      <span className="faint num">{new Date(entry.at).toLocaleTimeString("ru-RU")}</span> {entry.message}
                    </li>
                  ))}
                </ol>
              </details>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
}
