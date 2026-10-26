import { afterEach, describe, expect, it, vi } from "vitest";

import type { HomeworkWorkflow, ReviewRun } from "../types";
import { BridgeError } from "../types";
import { __setListenForTests, listenToBridgeEvent } from "./events";
import * as ipc from "./ipc";
import { __setTauriRuntimeForTests } from "./runtime";
import { TauriDesktopBridge, type TauriIpcClient } from "./TauriDesktopBridge";

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

afterEach(() => {
  ipc.__setInvokeForTests(undefined);
  __setListenForTests(undefined);
  __setTauriRuntimeForTests(undefined);
});

function recordInvoke() {
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  ipc.__setInvokeForTests(async (command, args) => {
    calls.push({ command, args });
    return { operation: { id: "op" }, value: { id: "value" } };
  });
  return calls;
}

describe("Tauri IPC рабочего цикла (этап 8)", () => {
  it("каждая функция вызывает точную команду с точными camelCase-аргументами", async () => {
    const calls = recordInvoke();
    await ipc.getWorkflow("hw");
    await ipc.getDashboard();
    await ipc.getIterations("hw");
    await ipc.compareVersions("v1", "v2");
    await ipc.getHomeworkCandidates("hw");
    await ipc.createSourceBundle("op-1", "hw", "analyze_materials", null, 3);
    await ipc.startWorkflowRun("op-2", { homeworkId: "hw", kind: "review_version", versionId: "v1", bundleId: "b1", mode: "manual" }, 4);
    await ipc.cancelWorkflowRun("op-3", "run", null);
    await ipc.pinWorkflowRun("op-4", "run", 5);
    await ipc.saveManualAnalysis("op-5", "run", { summary: "s", promptText: "p" }, 6);
    await ipc.saveManualReview("op-6", "run", { verdict: "pass", summary: "ok", correctness: "pass", taskFit: "pass", courseGrounding: "warn", teacher: "na", styleScore: 3 }, 7);
    await ipc.createRevisionPrompt("op-7", "run", null, 8);
    await ipc.updatePromptDraft("op-8", "prompt", "text", 9);
    await ipc.commitPrompt("op-9", "prompt", 10);
    await ipc.handOffPrompt("op-10", "prompt", 11);
    await ipc.createHomeworkVersion("op-11", "hw", { fileId: "f", attachmentFileIds: ["a"], author: "user", promptId: null, note: null }, 12);
    await ipc.selectHomeworkVersion("op-12", "v1", 13);
    await ipc.addTeacherComments("op-13", "hw", [{ source: "chat", date: "2026-10-21", question: "q", answer: "a" }], 14);
    await ipc.addTeacherRequirement("op-14", "hw", { commentId: "c", text: "t", origin: "o", status: "not_done" }, 15);
    await ipc.resolveTeacherRequirement("op-15", "req", "done", "note", 16);
    await ipc.confirmStyleDivergence("op-16", "run", 17);
    await ipc.markReadyToSubmit("op-17", "hw", true, 18);
    await ipc.transitionWorkflow("op-18", "hw", "needs_fixes", "причина", 19);

    expect(calls.map((call) => call.command)).toEqual([
      "get_workflow",
      "get_dashboard",
      "get_iterations",
      "compare_versions",
      "get_homework_candidates",
      "create_source_bundle",
      "start_workflow_run",
      "cancel_workflow_run",
      "pin_workflow_run",
      "save_manual_analysis",
      "save_manual_review",
      "create_revision_prompt",
      "update_prompt_draft",
      "commit_prompt",
      "hand_off_prompt",
      "create_homework_version",
      "select_homework_version",
      "add_teacher_comments",
      "add_teacher_requirement",
      "resolve_teacher_requirement",
      "confirm_style_divergence",
      "mark_ready_to_submit",
      "transition_workflow",
    ]);
    expect(calls[5]!.args).toEqual({ operationId: "op-1", homeworkId: "hw", purpose: "analyze_materials", versionId: null, expectedRevision: 3 });
    expect(calls[6]!.args).toEqual({ operationId: "op-2", request: { homeworkId: "hw", kind: "review_version", versionId: "v1", bundleId: "b1", mode: "manual" }, expectedRevision: 4 });
    // Необязательные списки ручной проверки уходят явными пустыми массивами (deny_unknown_fields в Rust).
    expect(calls[10]!.args).toEqual({
      operationId: "op-6",
      runId: "run",
      input: {
        verdict: "pass",
        summary: "ok",
        correctness: "pass",
        taskFit: "pass",
        courseGrounding: "warn",
        teacher: "na",
        styleScore: 3,
        styleReasons: [],
        critical: [],
        remarks: [],
        unsupportedClaims: [],
        missing: [],
      },
      expectedRevision: 7,
    });
    expect(calls[18]!.args).toEqual({ operationId: "op-14", homeworkId: "hw", input: { commentId: "c", text: "t", origin: "o", status: "not_done", note: null }, expectedRevision: 15 });
    expect(calls[22]!.args).toEqual({ operationId: "op-18", homeworkId: "hw", to: "needs_fixes", reason: "причина", expectedRevision: 19 });
    const keys = (value: unknown): string[] =>
      value && typeof value === "object" && !Array.isArray(value)
        ? Object.entries(value).flatMap(([key, inner]) => [key, ...keys(inner)])
        : Array.isArray(value)
          ? value.flatMap(keys)
          : [];
    for (const call of calls) {
      for (const key of keys(call.args)) expect(key).not.toMatch(/_/);
    }
  });

  it("ошибка backend приходит структурированной BridgeError, без подмены", async () => {
    ipc.__setInvokeForTests(async () => Promise.reject({ code: "conflict", message: "Данные ДЗ изменились — перечитайте состояние" }));
    await expect(ipc.commitPrompt("op", "prompt", 1)).rejects.toMatchObject({ code: "conflict" });
    ipc.__setInvokeForTests(null);
    await expect(ipc.getWorkflow("hw")).rejects.toMatchObject({ code: "backend_unavailable" });
  });
});

const run: ReviewRun = {
  id: "run-1",
  homeworkId: "hw",
  kind: "review_version",
  role: "reviewer",
  provider: "codex",
  versionId: "v1",
  sourceBundleId: "b1",
  sourcesUsed: [],
  status: "waiting_for_provider",
  progress: 0,
  step: "Провайдер недоступен: AI-адаптер ещё не подключён",
  startedAt: "2026-10-22T10:00:00Z",
  finishedAt: null,
  savedAsIteration: false,
  result: null,
  errorMessage: null,
  provenance: "manual",
  styleThreshold: 4,
  assessment: null,
};

function client(overrides: Partial<TauriIpcClient>): TauriIpcClient {
  const fail = (name: string) => () => Promise.reject(new Error(`unexpected IPC call: ${name}`));
  return new Proxy(overrides as TauriIpcClient, {
    get: (target, key: string) => (key in target ? target[key as keyof TauriIpcClient] : fail(key)),
  });
}

describe("TauriDesktopBridge рабочего цикла", () => {
  it("передаёт operationId (UUID v4) и expectedRevision в точные IPC-вызовы", async () => {
    const startWorkflowRun = vi.fn(async () => ({ operation: {} as never, value: run }));
    const createSourceBundle = vi.fn(async () => ({ operation: {} as never, value: { id: "b1" } as never }));
    const handOffPrompt = vi.fn(async () => ({ operation: {} as never, value: {} as never }));
    const createHomeworkVersion = vi.fn(async () => ({ operation: {} as never, value: {} as never }));
    const bridge = new TauriDesktopBridge(client({ startWorkflowRun, createSourceBundle, handOffPrompt, createHomeworkVersion }));

    const started = await bridge.runVersionReview("hw", "v1", "b1", { expectedRevision: 7 });
    expect(started.status).toBe("waiting_for_provider");
    expect(started.result).toBeNull();
    const [operationId, request, revision] = startWorkflowRun.mock.calls[0] as unknown as [string, ipc.StartRunRequest, number];
    expect(operationId).toMatch(UUID_V4);
    expect(request).toEqual({ homeworkId: "hw", kind: "review_version", versionId: "v1", bundleId: "b1", mode: "provider" });
    expect(revision).toBe(7);

    await bridge.runMaterialAnalysis("hw", "b1", { operationId: "11111111-1111-4111-8111-111111111111", mode: "manual" });
    expect(startWorkflowRun.mock.calls[1]).toEqual(["11111111-1111-4111-8111-111111111111", { homeworkId: "hw", kind: "analyze_materials", versionId: null, bundleId: "b1", mode: "manual" }, null]);

    await bridge.prepareSourceBundle("hw", "review_version", "v1", { expectedRevision: 2 });
    expect(createSourceBundle.mock.calls[0]).toEqual([expect.stringMatching(UUID_V4), "hw", "review_version", "v1", 2]);
    await bridge.sendPromptToAuthor("prompt", { expectedRevision: 3 });
    expect(handOffPrompt.mock.calls[0]).toEqual([expect.stringMatching(UUID_V4), "prompt", 3]);
    await bridge.createHomeworkVersion("hw", "file", { attachmentFileIds: ["att"], expectedRevision: 4 });
    expect(createHomeworkVersion.mock.calls[0]).toEqual([
      expect.stringMatching(UUID_V4),
      "hw",
      { fileId: "file", attachmentFileIds: ["att"], author: null, promptId: null, note: null },
      4,
    ]);
  });

  it("не подменяет ошибки mock-данными и не делает fallback", async () => {
    const conflict = new BridgeError("conflict", "Данные ДЗ изменились — перечитайте состояние и повторите действие");
    const getWorkflow = vi.fn(async () => Promise.reject(conflict));
    const unavailable = new BridgeError("provider_unavailable", "Codex: официальный CLI не найден");
    const fileFailure = new BridgeError("blocked", "Комплект не готов");
    const bridge = new TauriDesktopBridge(client({
      getWorkflow,
      commitPrompt: async () => Promise.reject(conflict),
      connectProvider: async () => Promise.reject(unavailable),
      buildSubmissionPackage: async () => Promise.reject(fileFailure),
      importTeacherTranscript: async () => Promise.reject(fileFailure),
    }));
    await expect(bridge.commitPrompt("prompt", { expectedRevision: 1 })).rejects.toBe(conflict);
    await expect(bridge.getReviewRuns("hw")).rejects.toBe(conflict);
    await expect(bridge.getWorkflow("hw")).rejects.toBe(conflict);
    await expect(bridge.setSourceIncluded("b", "i", false)).rejects.toMatchObject({ code: "immutable" });
    await expect(bridge.connectProvider("codex")).rejects.toBe(unavailable);
    await expect(bridge.buildSubmissionPackage("hw")).rejects.toBe(fileFailure);
    await expect(bridge.importTeacherTranscript("hw", { text: "x", source: "chat", date: "2026-10-21" })).rejects.toBe(fileFailure);
  });

  it("not_found превращается в null только для getWorkflow, а не для списков агрегата", async () => {
    const missing = new BridgeError("not_found", "ДЗ не найдено");
    const bridge = new TauriDesktopBridge(client({ getWorkflow: async () => Promise.reject(missing) }));
    await expect(bridge.getWorkflow("missing")).resolves.toBeNull();
    await expect(bridge.getPrompts("missing")).rejects.toBe(missing);
    await expect(bridge.getReadiness("missing")).rejects.toBe(missing);
  });

  it("части агрегата читаются из одного get_workflow", async () => {
    const workflow = {
      runs: [run],
      prompts: [],
      teacherComments: [],
      teacherRequirements: [],
      readiness: { homeworkId: "hw", ready: false, checks: [], versionId: null, reviewRunId: null, revision: 2 },
      submissionPackage: null,
      submissionReceipt: null,
    } as unknown as HomeworkWorkflow;
    const getWorkflow = vi.fn(async () => workflow);
    const bridge = new TauriDesktopBridge(client({ getWorkflow }));
    expect(await bridge.getReviewRuns("hw")).toEqual([run]);
    expect((await bridge.getReadiness("hw")).revision).toBe(2);
    await expect(bridge.getSubmissionPackage("hw")).resolves.toBeNull();
    expect(getWorkflow).toHaveBeenCalledTimes(3);
  });

  it("статус провайдера берётся из backend-проекции, без mock", async () => {
    const getProviderStatuses = vi.fn(async () => [{ provider: "codex", state: "disconnected", availability: "manual", adapter: "manual_handoff" }] as never);
    const bridge = new TauriDesktopBridge(client({ getProviderStatuses }));
    const status = await bridge.getProviderStatus("codex");
    expect(status).toMatchObject({ availability: "manual", adapter: "manual_handoff" });
    await expect(bridge.getProviderStatus("claude")).rejects.toMatchObject({ code: "not_found" });
  });
});

describe("События рабочего цикла", () => {
  it("homework:updated доставляется, а после отписки callback не вызывается", async () => {
    __setTauriRuntimeForTests(true);
    let emit: ((message: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    __setListenForTests(async (_event, handler) => {
      emit = handler;
      return unlisten;
    });
    const listener = vi.fn();
    const unsubscribe = listenToBridgeEvent("homework:updated", listener);
    await vi.waitFor(() => expect(emit).toBeTypeOf("function"));
    emit?.({ payload: { homeworkId: "hw", revision: 3, stage: "codex_review", reason: "create_homework_version", operationId: "op" } });
    expect(listener).toHaveBeenCalledWith({ homeworkId: "hw", revision: 3, stage: "codex_review", reason: "create_homework_version", operationId: "op" });
    unsubscribe();
    unsubscribe();
    emit?.({ payload: { homeworkId: "hw", revision: 4, stage: "codex_review", reason: "x", operationId: "op" } });
    expect(listener).toHaveBeenCalledOnce();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("ранняя отписка readiness-updated не оставляет подписку", async () => {
    __setTauriRuntimeForTests(true);
    let resolve: ((unlisten: () => void) => void) | undefined;
    const unlisten = vi.fn();
    __setListenForTests(() => new Promise((r) => (resolve = r)));
    const listener = vi.fn();
    const unsubscribe = listenToBridgeEvent("homework:readiness-updated", listener);
    unsubscribe();
    await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
    resolve?.(unlisten);
    await vi.waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
    expect(listener).not.toHaveBeenCalled();
  });
});
