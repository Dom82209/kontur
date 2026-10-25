import { useCallback, useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import {
  AI_TASK_LABEL,
  bridge,
  BridgeError,
  type AiConfirmInput,
  type AiTask,
  type TransportPreference,
  type HomeworkWorkflowStage,
  type ManualAnalysisInput,
  type ManualReviewInput,
  type ReviewRun,
  type RunKind,
  type RunMode,
  type SourceBundle,
  type TeacherRequirementInput,
  type WorkflowCommandOptions,
} from "../bridge";
import { providerName, RUN_KIND_LABEL, WORKFLOW_STAGE_LABEL } from "../format";
import { isStaleConflict } from "../workflow-ui";
import { useBridgeEvent } from "./useBridgeEvent";

interface WorkflowContext {
  /** Ревизия агрегата ДЗ, которую видит пользователь: устаревшая → conflict. */
  revision?: number;
  /** Перечитать агрегат (после conflict и других ошибок состояния). */
  reload?: () => Promise<void> | void;
}

/**
 * Действия рабочего цикла, общие для карточки ДЗ и AI Workspace.
 * Провайдер не выбирается в компонентах — его определяет роль в настройках (backend).
 * Каждое действие передаёт expectedRevision; при conflict агрегат перечитывается,
 * и пользователю предлагается повторить действие.
 */
export function useWorkflowActions(homeworkId: string, context: WorkflowContext = {}) {
  const toast = useToast();
  const { reportError } = useOps();
  const [busy, setBusy] = useState<string | null>(null);
  const { revision, reload } = context;

  useBridgeEvent("review:completed", ({ run }) => {
    if (run.homeworkId !== homeworkId) return;
    const who = run.provenance === "manual" ? "Вручную" : providerName(run.provider);
    const saved = run.provenance === "provider" ? "подтверждено вами" : "сохранено";
    toast.push({ tone: run.result?.verdict === "needs_fixes" ? "warn" : "ok", title: `${who}: ${RUN_KIND_LABEL[run.kind].toLowerCase()} — ${saved}`, text: run.result?.summary });
  });
  useBridgeEvent("review:failed", ({ run, error }) => {
    if (run.homeworkId !== homeworkId || error.code === "cancelled") return;
    toast.push({ tone: "danger", title: "Запуск не завершён", text: error.message });
  });

  const options = useCallback((): WorkflowCommandOptions => (revision === undefined ? {} : { expectedRevision: revision }), [revision]);

  const guard = useCallback(
    async <T,>(key: string, title: string, fn: () => Promise<T>): Promise<T | null> => {
      setBusy(key);
      try {
        return await fn();
      } catch (e) {
        if (isStaleConflict(e)) {
          await reload?.();
          toast.push({ tone: "warn", title: "Данные ДЗ изменились", text: "Состояние перечитано — проверьте его и повторите действие." });
        } else {
          reportError(e, title);
          if (e instanceof BridgeError && (e.code === "conflict" || e.code === "immutable")) await reload?.();
        }
        return null;
      } finally {
        setBusy(null);
      }
    },
    [reportError, reload, toast],
  );

  const startRun = useCallback(
    (kind: RunKind, versionId: string | null, bundle?: SourceBundle | null, mode: RunMode = "provider") =>
      guard<ReviewRun>("run", "Запуск невозможен", async () => {
        let opts = { ...options(), mode };
        let b = bundle ?? null;
        if (!b) {
          b = await bridge.prepareSourceBundle(homeworkId, kind, versionId, options());
          // Набор источников — своя операция: следующая команда ожидает уже новую ревизию.
          if (opts.expectedRevision !== undefined) opts = { ...opts, expectedRevision: opts.expectedRevision + 1 };
        }
        if (kind === "analyze_materials") return bridge.runMaterialAnalysis(homeworkId, b.id, opts);
        if (kind === "review_teacher") return bridge.runTeacherReview(homeworkId, versionId!, b.id, opts);
        if (kind === "revision_prompt") return bridge.runRevisionPrompt(homeworkId, versionId!, b.id, opts);
        return bridge.runVersionReview(homeworkId, versionId!, b.id, opts);
      }),
    [guard, homeworkId, options],
  );

  const prepareBundle = useCallback(
    (kind: RunKind, versionId: string | null) =>
      guard("bundle", "Набор источников не создан", async () => {
        const bundle = await bridge.prepareSourceBundle(homeworkId, kind, versionId, options());
        toast.push({ tone: bundle.complete ? "ok" : "warn", title: "Снимок источников создан", text: bundle.complete ? `${bundle.items.length} материалов из подборки` : "Не хватает обязательных материалов" });
        return bundle;
      }),
    [guard, homeworkId, options, toast],
  );

  const saveAnalysis = useCallback(
    (runId: string, input: ManualAnalysisInput) =>
      guard("manual", "Результат не сохранён", async () => {
        const r = await bridge.saveManualAnalysis(runId, input, options());
        toast.push({ tone: "ok", title: r.operation.title, text: "Черновик промта сохранён — проверьте и зафиксируйте его" });
        return r.value;
      }),
    [guard, options, toast],
  );

  const saveReview = useCallback(
    (runId: string, input: ManualReviewInput) =>
      guard("manual", "Результат не сохранён", async () => {
        const r = await bridge.saveManualReviewResult(runId, input, options());
        return r.value;
      }),
    [guard, options],
  );

  const cancelRun = useCallback(
    (runId: string) =>
      guard("cancel", "Отмена не выполнена", async () => {
        const run = await bridge.cancelRun(runId, options());
        toast.push({ tone: "info", title: "Запуск отменён" });
        return run;
      }),
    [guard, options, toast],
  );

  const createPrompt = useCallback(
    (runId: string, text?: string) =>
      guard("prompt", "Промт не создан", async () => {
        const r = await bridge.createRevisionPrompt(runId, text, options());
        toast.push({ tone: "ok", title: r.operation.title, text: r.value.status === "draft" ? "Черновик сохранён — проверьте и зафиксируйте его" : "Промт сохранён в истории итераций" });
        return r.value;
      }),
    [guard, options, toast],
  );

  const updatePrompt = useCallback(
    (promptId: string, text: string) => guard("prompt-edit", "Черновик не сохранён", () => bridge.updatePromptText(promptId, text, options())),
    [guard, options],
  );

  const commitPrompt = useCallback(
    (promptId: string) =>
      guard("commit", "Промт не зафиксирован", async () => {
        const r = await bridge.commitPrompt(promptId, options());
        toast.push({ tone: "ok", title: r.operation.title, text: "Теперь промт неизменяем — его можно передать автору" });
        return r.value;
      }),
    [guard, options, toast],
  );

  const sendPrompt = useCallback(
    (promptId: string, manual = false) =>
      guard("send", "Промт не передан", async () => {
        const r = await bridge.sendPromptToAuthor(promptId, options());
        toast.push(
          manual
            ? { tone: "ok", title: "Передача отмечена", text: "Когда автор пришлёт решение, добавьте его как новую версию" }
            : { tone: "ok", title: "Новая версия создана", text: "Версия появилась в рабочем цикле и ждёт проверки" },
        );
        return r.value;
      }),
    [guard, options, toast],
  );

  const saveIteration = useCallback(
    (runId: string) =>
      guard("save", "Не сохранено", async () => {
        const r = await bridge.saveRunAsIteration(runId, options());
        toast.push({ tone: "ok", title: "Результат сохранён как итерация" });
        return r;
      }),
    [guard, options, toast],
  );

  const confirmStyle = useCallback(
    (runId: string) =>
      guard("style", "Подтверждение не сохранено", async () => {
        const r = await bridge.confirmStyleDivergence(runId, options());
        toast.push({ tone: "ok", title: r.operation.title, text: `Расхождение ${r.value.score}/10 при пороге ${r.value.threshold}` });
        return r.value;
      }),
    [guard, options, toast],
  );

  const transition = useCallback(
    (to: HomeworkWorkflowStage, reason?: string) =>
      guard("transition", "Этап не изменён", async () => {
        const r = await bridge.transitionWorkflow(homeworkId, to, { ...options(), reason });
        toast.push({ tone: "ok", title: `Этап: ${WORKFLOW_STAGE_LABEL[to]}` });
        return r.value;
      }),
    [guard, homeworkId, options, toast],
  );

  const addRequirement = useCallback(
    (input: TeacherRequirementInput) => guard("requirement", "Требование не сохранено", async () => (await bridge.addTeacherRequirement(homeworkId, input, options())).value),
    [guard, homeworkId, options],
  );

  const resolveRequirement = useCallback(
    (requirementId: string, status: "done" | "not_done" | "na") =>
      guard("requirement", "Требование не обновлено", () => bridge.resolveRequirement(requirementId, status, "Проверено вручную", options())),
    [guard, options],
  );

  const openApp = useCallback(
    (provider: Parameters<typeof bridge.openProviderApp>[0]) =>
      guard("open", "Не удалось открыть", async () => {
        await bridge.openProviderApp(provider);
        toast.push({ tone: "info", title: `Открываем ${providerName(provider)}`, text: "Backend откроет официальное приложение — вход выполняется там" });
      }),
    [guard, toast],
  );

  // ── Этап 9: задания передачи провайдерам ──

  const prepareJob = useCallback(
    (runId: string, task: AiTask, transport: TransportPreference = "auto") => guard("ai-prepare", "Передача не подготовлена", () => bridge.prepareAiJob(runId, task, transport)),
    [guard],
  );

  const sendJob = useCallback(
    (jobId: string) =>
      guard("ai-send", "Отправка не выполнена", async () => {
        const job = await bridge.sendAiJob(jobId);
        toast.push({ tone: "info", title: `Отправлено: ${job.adapterLabel ?? providerName(job.provider)}`, text: "Процесс официального CLI запущен. Ответ придёт на проверку — сам он ничего не изменит." });
        return job;
      }),
    [guard, toast],
  );

  const cancelJob = useCallback((jobId: string) => guard("ai-cancel", "Остановка не выполнена", () => bridge.cancelAiJob(jobId)), [guard]);

  const submitResponse = useCallback((jobId: string, text: string) => guard("ai-response", "Ответ не сохранён", () => bridge.submitAiResponse(jobId, text)), [guard]);

  const importResponse = useCallback((jobId: string) => guard("ai-response", "Файл ответа не импортирован", () => bridge.importAiResponseFile(jobId)), [guard]);

  const discardJob = useCallback((jobId: string) => guard("ai-discard", "Ответ не отклонён", () => bridge.discardAiJob(jobId)), [guard]);

  const confirmAi = useCallback(
    (jobId: string, task: AiTask, input: AiConfirmInput, supportingJobId?: string | null) =>
      guard("ai-confirm", "Результат не подтверждён", async () => {
        const r = await bridge.confirmAiResult(jobId, input, { ...options(), supportingJobId: supportingJobId ?? null });
        // Анализ и проверки сообщает review:completed; промт исправлений — здесь.
        if (task === "revision_prompt")
          toast.push({
            tone: "ok",
            title: `${AI_TASK_LABEL[task]}: черновик сохранён`,
            text: r.value.job.edited ? "С вашими правками — проверьте и зафиксируйте его" : "Проверьте и зафиксируйте его",
          });
        return r.value;
      }),
    [guard, options, toast],
  );

  return {
    busy,
    options,
    prepareJob,
    sendJob,
    cancelJob,
    submitResponse,
    importResponse,
    discardJob,
    confirmAi,
    startRun,
    prepareBundle,
    saveAnalysis,
    saveReview,
    cancelRun,
    createPrompt,
    updatePrompt,
    commitPrompt,
    sendPrompt,
    saveIteration,
    confirmStyle,
    transition,
    addRequirement,
    resolveRequirement,
    openApp,
  };
}
