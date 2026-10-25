import { afterEach, describe, expect, it, vi } from "vitest";

import type { AiJob } from "../types";
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

function client(overrides: Partial<TauriIpcClient>): TauriIpcClient {
  const fail = (name: string) => () => Promise.reject(new Error(`unexpected IPC call: ${name}`));
  return new Proxy(overrides as TauriIpcClient, {
    get: (target, key: string) => (key in target ? target[key as keyof TauriIpcClient] : fail(key)),
  });
}

const job = { id: "job-1", runId: "run-1", homeworkId: "hw", task: "material_analysis", status: "prepared" } as AiJob;

describe("Tauri IPC провайдеров и заданий (этап 9)", () => {
  it("каждая функция вызывает точную команду с точными camelCase-аргументами", async () => {
    const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
    ipc.__setInvokeForTests(async (command, args) => {
      calls.push({ command, args });
      return null;
    });
    await ipc.getProviderStatuses();
    await ipc.checkProvider("codex");
    await ipc.connectProvider("claude");
    await ipc.disconnectProvider("claude");
    await ipc.openProviderApp("claude");
    await ipc.listAiJobs("hw");
    await ipc.getAiJob("job-1");
    await ipc.prepareAiJob("run-1", "solution_review", "auto");
    await ipc.sendAiJob("job-1");
    await ipc.cancelAiJob("job-1");
    await ipc.submitAiResponse("job-1", "ответ");
    await ipc.importAiResponseFile("job-1");
    await ipc.discardAiJob("job-1");
    await ipc.confirmAiResult("op-1", "job-1", null, { kind: "analysis", value: { summary: "s", promptText: "p" } }, 4);
    await ipc.confirmAiResult(
      "op-2",
      "job-2",
      "job-3",
      { kind: "review", value: { verdict: "pass", summary: "ok", correctness: "pass", taskFit: "pass", courseGrounding: "pass", teacher: "na", styleScore: null, critical: [{ criterion: "correctness", text: "x", location: null }] } },
      null,
    );
    expect(calls).toEqual([
      { command: "get_provider_statuses", args: undefined },
      { command: "check_provider", args: { provider: "codex" } },
      { command: "connect_provider", args: { provider: "claude" } },
      { command: "disconnect_provider", args: { provider: "claude" } },
      { command: "open_provider_app", args: { provider: "claude" } },
      { command: "list_ai_jobs", args: { homeworkId: "hw" } },
      { command: "get_ai_job", args: { jobId: "job-1" } },
      { command: "prepare_ai_job", args: { runId: "run-1", task: "solution_review", transport: "auto" } },
      { command: "send_ai_job", args: { jobId: "job-1" } },
      { command: "cancel_ai_job", args: { jobId: "job-1" } },
      { command: "submit_ai_response", args: { jobId: "job-1", text: "ответ" } },
      { command: "import_ai_response_file", args: { jobId: "job-1" } },
      { command: "discard_ai_job", args: { jobId: "job-1" } },
      {
        command: "confirm_ai_result",
        args: { operationId: "op-1", jobId: "job-1", supportingJobId: null, input: { kind: "analysis", value: { summary: "s", promptText: "p" } }, expectedRevision: 4 },
      },
      {
        command: "confirm_ai_result",
        args: {
          operationId: "op-2",
          jobId: "job-2",
          supportingJobId: "job-3",
          input: {
            kind: "review",
            value: {
              verdict: "pass",
              summary: "ok",
              correctness: "pass",
              taskFit: "pass",
              courseGrounding: "pass",
              teacher: "na",
              styleScore: null,
              styleReasons: [],
              critical: [{ criterion: "correctness", text: "x", location: null, evidence: [] }],
              remarks: [],
              unsupportedClaims: [],
              missing: [],
            },
          },
          expectedRevision: null,
        },
      },
    ]);
    // Никаких паролей, токенов и cookies в аргументах.
    expect(JSON.stringify(calls)).not.toMatch(/password|token|cookie|apiKey/i);
  });
});

describe("TauriDesktopBridge провайдеров и заданий", () => {
  it("делегирует методы напрямую, генерирует operationId и передаёт ревизию", async () => {
    const prepareAiJob = vi.fn(async () => job);
    const confirmAiResult = vi.fn(async () => ({ operation: {} as never, value: {} as never }));
    const bridge = new TauriDesktopBridge(client({ prepareAiJob, confirmAiResult }));
    await bridge.prepareAiJob("run-1", "material_analysis");
    expect(prepareAiJob).toHaveBeenCalledWith("run-1", "material_analysis", "auto");
    await bridge.confirmAiResult("job-1", { kind: "prompt", value: { text: "Исправь" } }, { expectedRevision: 7 });
    const [operationId, jobId, supporting, input, revision] = confirmAiResult.mock.calls[0] as unknown as unknown[];
    expect(operationId).toMatch(UUID_V4);
    expect([jobId, supporting, input, revision]).toEqual(["job-1", null, { kind: "prompt", value: { text: "Исправь" } }, 7]);
    await bridge.confirmAiResult("job-1", { kind: "prompt", value: { text: "x" } }, { operationId: "fixed", supportingJobId: "job-2" });
    expect((confirmAiResult.mock.calls[1] as unknown as unknown[]).slice(0, 3)).toEqual(["fixed", "job-1", "job-2"]);
  });

  it("ошибки backend не подменяются: нет ложного успеха и mock fallback", async () => {
    const reauth = new BridgeError("reauth_required", "Войдите в официальном CLI");
    const blocked = new BridgeError("blocked", "Это задание ручной передачи");
    const bridge = new TauriDesktopBridge(client({ sendAiJob: async () => Promise.reject(blocked), checkProvider: async () => Promise.reject(reauth), importAiResponseFile: async () => null }));
    await expect(bridge.sendAiJob("job-1")).rejects.toBe(blocked);
    await expect(bridge.checkProvider("codex")).rejects.toBe(reauth);
    await expect(bridge.importAiResponseFile("job-1")).resolves.toBeNull();
    await expect(bridge.cancelAiJob("job-1")).rejects.toThrow("unexpected IPC call: cancelAiJob");
  });

  it("события ai-job доставляются только до отписки", async () => {
    __setTauriRuntimeForTests(true);
    const handlers = new Map<string, (message: { payload: unknown }) => void>();
    __setListenForTests(async (event, handler) => {
      handlers.set(event, handler);
      return () => handlers.delete(event);
    });
    const progress = vi.fn();
    const off = listenToBridgeEvent("ai-job:progress", progress);
    await vi.waitFor(() => expect(handlers.has("ai-job:progress")).toBe(true));
    const emit = handlers.get("ai-job:progress")!;
    emit({ payload: { jobId: "job-1", runId: "run-1", homeworkId: "hw", phase: "running", elapsedMs: 1000, outputBytes: 64 } });
    off();
    emit({ payload: { jobId: "job-1", runId: "run-1", homeworkId: "hw", phase: "running", elapsedMs: 2000, outputBytes: 64 } });
    expect(progress).toHaveBeenCalledTimes(1);
    expect(progress).toHaveBeenCalledWith(expect.objectContaining({ phase: "running", elapsedMs: 1000 }));
  });
});
