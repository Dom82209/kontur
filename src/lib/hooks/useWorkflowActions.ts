import { useCallback, useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type ReviewRun, type RunKind, type SourceBundle } from "../bridge";
import { providerName, RUN_KIND_LABEL } from "../format";
import { useBridgeEvent } from "./useBridgeEvent";

/**
 * Действия рабочего цикла, общие для карточки ДЗ и AI Workspace.
 * Провайдер не выбирается в компонентах — его определяет роль в настройках (backend).
 */
export function useWorkflowActions(homeworkId: string) {
  const toast = useToast();
  const { reportError } = useOps();
  const [busy, setBusy] = useState<string | null>(null);

  useBridgeEvent("review:completed", ({ run }) => {
    if (run.homeworkId !== homeworkId) return;
    toast.push({ tone: run.result?.verdict === "needs_fixes" ? "warn" : "ok", title: `${providerName(run.provider)}: ${RUN_KIND_LABEL[run.kind].toLowerCase()} — готово`, text: run.result?.summary });
  });
  useBridgeEvent("review:failed", ({ run, error }) => {
    if (run.homeworkId !== homeworkId || error.code === "cancelled") return;
    toast.push({ tone: "danger", title: "Запуск не завершён", text: error.message });
  });

  const guard = useCallback(
    async <T,>(key: string, title: string, fn: () => Promise<T>): Promise<T | null> => {
      setBusy(key);
      try {
        return await fn();
      } catch (e) {
        reportError(e, title);
        return null;
      } finally {
        setBusy(null);
      }
    },
    [reportError],
  );

  const startRun = useCallback(
    (kind: RunKind, versionId: string | null, bundle?: SourceBundle | null) =>
      guard<ReviewRun>("run", "Запуск невозможен", async () => {
        const b = bundle ?? (await bridge.prepareSourceBundle(homeworkId, kind, versionId));
        if (kind === "analyze_materials") return bridge.runMaterialAnalysis(homeworkId, b.id);
        if (kind === "review_teacher") return bridge.runTeacherReview(homeworkId, versionId!, b.id);
        if (kind === "revision_prompt") return bridge.runRevisionPrompt(homeworkId, versionId!, b.id);
        return bridge.runVersionReview(homeworkId, versionId!, b.id);
      }),
    [guard, homeworkId],
  );

  const createPrompt = useCallback(
    (runId: string, text?: string) =>
      guard("prompt", "Промт не создан", async () => {
        const r = await bridge.createRevisionPrompt(runId, text);
        toast.push({ tone: "ok", title: r.operation.title, text: "Промт сохранён в истории итераций" });
        return r.value;
      }),
    [guard, toast],
  );

  const sendPrompt = useCallback(
    (promptId: string) =>
      guard("send", "Промт не передан", async () => {
        const r = await bridge.sendPromptToAuthor(promptId);
        toast.push({ tone: "ok", title: "Новая версия создана", text: "Версия появилась в рабочем цикле и ждёт проверки" });
        return r.value;
      }),
    [guard, toast],
  );

  const saveIteration = useCallback(
    (runId: string) =>
      guard("save", "Не сохранено", async () => {
        const r = await bridge.saveRunAsIteration(runId);
        toast.push({ tone: "ok", title: "Результат сохранён как итерация" });
        return r;
      }),
    [guard, toast],
  );

  const openApp = useCallback(
    (provider: Parameters<typeof bridge.openProviderApp>[0]) =>
      guard("open", "Не удалось открыть", async () => {
        await bridge.openProviderApp(provider);
        toast.push({ tone: "info", title: `Открываем ${providerName(provider)}`, text: "Backend откроет официальное приложение — вход выполняется там" });
      }),
    [guard, toast],
  );

  return { busy, startRun, createPrompt, sendPrompt, saveIteration, openApp };
}
