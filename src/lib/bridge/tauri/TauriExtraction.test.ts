import { afterEach, describe, expect, it, vi } from "vitest";

import type { ExtractionBatchReport, MaterialRef, Operation } from "../types";
import { BridgeError } from "../types";
import { __setListenForTests, listenToBridgeEvent } from "./events";
import {
  __setInvokeForTests,
  analyzeMaterial,
  analyzeMaterials,
  cancelExtraction,
  clearExtractionCache,
  getCorpusSelection,
  getExtractionSettings,
  getMaterialExtraction,
  listMaterialExtractions,
  previewCorpus,
  setCorpusSelection,
  setExtractionSettings,
} from "./ipc";
import { __setTauriRuntimeForTests } from "./runtime";
import { TauriDesktopBridge, type TauriIpcClient } from "./TauriDesktopBridge";

afterEach(() => {
  __setInvokeForTests(undefined);
  __setListenForTests(undefined);
  __setTauriRuntimeForTests(undefined);
});

const material: MaterialRef = { kind: "inbox", id: "inbox-1" };
const weekFile: MaterialRef = { kind: "file", id: "file-7" };

describe("этап 7: IPC-команды извлечения текста", () => {
  it("вызывает точные имена команд с camelCase-аргументами", async () => {
    const invoke = vi.fn(async () => null);
    __setInvokeForTests(invoke);

    await listMaterialExtractions();
    await getMaterialExtraction(material);
    await analyzeMaterial(material, true);
    await analyzeMaterials("op-1", [material, weekFile], false);
    await cancelExtraction("op-1");
    await clearExtractionCache();
    await getExtractionSettings();
    await setExtractionSettings(true);
    await getCorpusSelection(null);
    await getCorpusSelection("hw-1");
    await setCorpusSelection("hw-1", [weekFile]);
    await previewCorpus(null);

    expect(invoke.mock.calls).toEqual([
      ["list_material_extractions", undefined],
      ["get_material_extraction", { material }],
      ["analyze_material", { material, force: true }],
      ["analyze_materials", { operationId: "op-1", materials: [material, weekFile], force: false }],
      ["cancel_extraction", { operationId: "op-1" }],
      ["clear_extraction_cache", undefined],
      ["get_extraction_settings", undefined],
      ["set_extraction_settings", { ocrEnabled: true }],
      ["get_corpus_selection", { homeworkId: null }],
      ["get_corpus_selection", { homeworkId: "hw-1" }],
      ["set_corpus_selection", { homeworkId: "hw-1", materials: [weekFile] }],
      ["preview_corpus", { homeworkId: null }],
    ]);
  });

  it("лишние поля ссылки на материал не уходят в backend", async () => {
    const invoke = vi.fn(async () => null);
    __setInvokeForTests(invoke);
    await analyzeMaterial({ ...material, name: "secret.pdf" } as MaterialRef, false);
    expect(invoke.mock.calls[0]).toEqual(["analyze_material", { material, force: false }]);
  });

  it("ошибка backend приходит как BridgeError без fallback", async () => {
    __setInvokeForTests(async () => {
      throw { code: "conflict", message: "Файл изменился вне «Контура»" };
    });
    await expect(analyzeMaterial(material, false)).rejects.toMatchObject({ code: "conflict" });
    await expect(previewCorpus(null)).rejects.toBeInstanceOf(BridgeError);
  });

  it("события extraction:progress и extraction:updated доставляются типизированно", async () => {
    __setTauriRuntimeForTests(true);
    const handlers = new Map<string, (message: { payload: unknown }) => void>();
    __setListenForTests(async (event, h) => {
      handlers.set(event, h);
      return () => undefined;
    });
    const progress: number[] = [];
    const updated: string[] = [];
    const offProgress = listenToBridgeEvent("extraction:progress", (p) => progress.push(p.completed));
    const offUpdated = listenToBridgeEvent("extraction:updated", (p) => updated.push(p.reason));
    await vi.waitFor(() => expect(handlers.size).toBe(2));
    handlers.get("extraction:progress")!({ payload: { operationId: "op", completed: 1, total: 2, material, name: "a.pdf", outcome: "extracted" } });
    handlers.get("extraction:updated")!({ payload: { reason: "cleared", materials: [] } });
    expect(progress).toEqual([1]);
    expect(updated).toEqual(["cleared"]);
    offProgress();
    offUpdated();
  });
});

describe("TauriDesktopBridge: этап 7 делегирует IPC без mock fallback", () => {
  const operation: Operation = { id: "op", kind: "extract", title: "Извлечение текста", detail: null, status: "completed", startedAt: "2026-10-22T10:00:00Z", finishedAt: "2026-10-22T10:00:01Z", undoable: false, paths: [], backupPath: null, homeworkId: null };
  const report: ExtractionBatchReport = { operationId: "op", items: [], extracted: 1, cached: 0, failed: 0, cancelled: 0, stoppedByUser: false, replayed: false };

  function client(overrides: Partial<TauriIpcClient>): TauriIpcClient {
    return new Proxy(overrides as TauriIpcClient, {
      get: (target, key: string) => (key in target ? target[key as keyof TauriIpcClient] : () => Promise.reject(new Error(`unexpected IPC call: ${key}`))),
    });
  }

  it("analyzeMaterials без operationId создаёт UUID v4, force по умолчанию false", async () => {
    const batch = vi.fn(async () => ({ operation, value: report }));
    const adapter = new TauriDesktopBridge(client({ analyzeMaterials: batch }));
    await adapter.analyzeMaterials([material]);
    const [operationId, sent, force] = batch.mock.calls[0] as unknown as [string, MaterialRef[], boolean];
    expect(operationId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(sent).toEqual([material]);
    expect(force).toBe(false);
    await adapter.analyzeMaterials([material], { operationId: "fixed", force: true });
    expect(batch.mock.calls[1]).toEqual(["fixed", [material], true]);
  });

  it("getMaterialExtraction: not_found → null, остальные ошибки пробрасываются", async () => {
    const notFound = new TauriDesktopBridge(client({ getMaterialExtraction: async () => Promise.reject(new BridgeError("not_found", "нет")) }));
    await expect(notFound.getMaterialExtraction(material)).resolves.toBeNull();
    const broken = new TauriDesktopBridge(client({ getMaterialExtraction: async () => Promise.reject(new BridgeError("database", "сломано")) }));
    await expect(broken.getMaterialExtraction(material)).rejects.toMatchObject({ code: "database" });
  });

  it("настройки, отмена и подборка идут в настоящий backend", async () => {
    const setSettings = vi.fn(async () => ({ ocrEnabled: true }) as never);
    const cancel = vi.fn(async () => true);
    const analyze = vi.fn(async () => ({}) as never);
    const select = vi.fn(async () => ({}) as never);
    const adapter = new TauriDesktopBridge(client({ setExtractionSettings: setSettings, cancelExtraction: cancel, analyzeMaterial: analyze, setCorpusSelection: select }));
    await adapter.setExtractionSettings({ ocrEnabled: true });
    await expect(adapter.cancelExtraction("op")).resolves.toBe(true);
    await adapter.analyzeMaterial(material);
    await adapter.setCorpusSelection(null, [material]);
    expect(setSettings).toHaveBeenCalledWith(true);
    expect(cancel).toHaveBeenCalledWith("op");
    expect(analyze).toHaveBeenCalledWith(material, false);
    expect(select).toHaveBeenCalledWith(null, [material]);
  });
});
