import { afterEach, describe, expect, it, vi } from "vitest";

import type { InboxFile, InboxImportReport, ManagedRoots, Operation, OperationResult } from "../types";
import { BridgeError } from "../types";
import { __setListenForTests, listenToIntakeEvent } from "./events";
import {
  __setInvokeForTests,
  chooseManagedRoot,
  importInboxFiles,
  pickImportFiles,
  revealInFinder,
} from "./ipc";
import { __setTauriRuntimeForTests } from "./runtime";
import { TauriDesktopBridge, type TauriIpcClient } from "./TauriDesktopBridge";

afterEach(() => {
  __setInvokeForTests(undefined);
  __setListenForTests(undefined);
  __setTauriRuntimeForTests(undefined);
});

const file: InboxFile = {
  id: "inbox-1",
  name: "Лекция.pdf",
  kind: "pdf",
  sizeBytes: 10,
  addedAt: "2026-10-21T10:00:00Z",
  origin: "drop",
  status: "new",
  classification: null,
  duplicate: null,
  provenance: null,
};

const operation: Operation = {
  id: "8c7d6c4e-2c5a-4b8e-9f1a-0a1b2c3d4e5f",
  kind: "import",
  title: "Импорт во «Входящие»",
  detail: "Добавлено: 1 · дублей: 1 · ошибок: 1",
  status: "completed",
  startedAt: "2026-10-21T10:00:00Z",
  finishedAt: "2026-10-21T10:00:01Z",
  undoable: false,
  paths: ["Предметы/00 Входящие/Лекция.pdf"],
  backupPath: null,
  homeworkId: null,
};

const report: InboxImportReport = {
  operationId: operation.id,
  items: [
    { status: "imported", name: "Лекция.pdf", file, renamedFrom: null, recovered: false },
    { status: "duplicate", name: "Копия.pdf", file, restored: false },
    { status: "failed", name: "нет.pdf", error: { code: "not_found", message: "Исходный файл не найден" } },
  ],
  imported: 1,
  duplicates: 1,
  failed: 1,
  skipped: 0,
  replayed: false,
};

const roots: ManagedRoots = {
  vault: { kind: "vault", state: "ok", displayPath: "~/Учёба", message: null },
  inbox: { kind: "inbox", state: "missing", displayPath: "~/Учёба/Предметы/00 Входящие", message: null },
  backup: { kind: "backup", state: "not_configured", displayPath: null, message: "Не выбрана" },
  downloads: { kind: "downloads", state: "not_configured", displayPath: null, message: "Не выбрана" },
  importReady: true,
};

function client(overrides: Partial<TauriIpcClient>): TauriIpcClient {
  const fail = (name: string) => () => Promise.reject(new Error(`unexpected IPC call: ${name}`));
  return {
    getSubjects: fail("getSubjects"),
    getWeeks: fail("getWeeks"),
    getWeek: fail("getWeek"),
    getInboxFiles: fail("getInboxFiles"),
    getHomeworkList: fail("getHomeworkList"),
    getHomework: fail("getHomework"),
    getAutomationRules: fail("getAutomationRules"),
    getActivityLog: fail("getActivityLog"),
    updateAutomationRules: fail("updateAutomationRules"),
    updateClassification: fail("updateClassification"),
    checkManagedRoots: fail("checkManagedRoots"),
    chooseManagedRoot: fail("chooseManagedRoot"),
    prepareInboxRoot: fail("prepareInboxRoot"),
    pickImportFiles: fail("pickImportFiles"),
    importInboxFiles: fail("importInboxFiles"),
    getOperationResult: fail("getOperationResult"),
    openInboxFile: fail("openInboxFile"),
    revealInboxFile: fail("revealInboxFile"),
    revealInFinder: fail("revealInFinder"),
    getDownloadsWatcher: fail("getDownloadsWatcher"),
    setDownloadsWatcher: fail("setDownloadsWatcher"),
    dismissDownloadCandidate: fail("dismissDownloadCandidate"),
    pickCuExport: fail("pickCuExport"),
    previewCuExport: fail("previewCuExport"),
    importCuExport: fail("importCuExport"),
    setSubjectCourseUrl: fail("setSubjectCourseUrl"),
    setHomeworkAssignmentUrl: fail("setHomeworkAssignmentUrl"),
    openCourse: fail("openCourse"),
    openAssignment: fail("openAssignment"),
    listMaterialExtractions: fail("listMaterialExtractions"),
    getMaterialExtraction: fail("getMaterialExtraction"),
    analyzeMaterial: fail("analyzeMaterial"),
    analyzeMaterials: fail("analyzeMaterials"),
    cancelExtraction: fail("cancelExtraction"),
    clearExtractionCache: fail("clearExtractionCache"),
    getExtractionSettings: fail("getExtractionSettings"),
    setExtractionSettings: fail("setExtractionSettings"),
    getCorpusSelection: fail("getCorpusSelection"),
    setCorpusSelection: fail("setCorpusSelection"),
    previewCorpus: fail("previewCorpus"),
    getWorkflow: fail("getWorkflow"),
    getDashboard: fail("getDashboard"),
    getIterations: fail("getIterations"),
    compareVersions: fail("compareVersions"),
    getHomeworkCandidates: fail("getHomeworkCandidates"),
    createSourceBundle: fail("createSourceBundle"),
    startWorkflowRun: fail("startWorkflowRun"),
    cancelWorkflowRun: fail("cancelWorkflowRun"),
    pinWorkflowRun: fail("pinWorkflowRun"),
    saveManualAnalysis: fail("saveManualAnalysis"),
    saveManualReview: fail("saveManualReview"),
    createRevisionPrompt: fail("createRevisionPrompt"),
    updatePromptDraft: fail("updatePromptDraft"),
    commitPrompt: fail("commitPrompt"),
    handOffPrompt: fail("handOffPrompt"),
    createHomeworkVersion: fail("createHomeworkVersion"),
    selectHomeworkVersion: fail("selectHomeworkVersion"),
    addTeacherComments: fail("addTeacherComments"),
    addTeacherRequirement: fail("addTeacherRequirement"),
    resolveTeacherRequirement: fail("resolveTeacherRequirement"),
    confirmStyleDivergence: fail("confirmStyleDivergence"),
    markReadyToSubmit: fail("markReadyToSubmit"),
    transitionWorkflow: fail("transitionWorkflow"),
    ...overrides,
  };
}

describe("Tauri native intake", () => {
  it("отменённый системный picker возвращает пустой список и не запускает импорт", async () => {
    const importInboxFiles = vi.fn();
    const adapter = new TauriDesktopBridge(client({ pickImportFiles: async () => [], importInboxFiles }));
    await expect(adapter.pickFiles()).resolves.toEqual([]);
    expect(importInboxFiles).not.toHaveBeenCalled();
  });

  it("picker помечает элементы origin=picker и сохраняет sourcePath", async () => {
    const adapter = new TauriDesktopBridge(
      client({ pickImportFiles: async () => [{ name: "a.pdf", sizeBytes: 3, sourcePath: "/Users/u/Downloads/a.pdf" }] }),
    );
    await expect(adapter.pickFiles()).resolves.toEqual([
      { name: "a.pdf", sizeBytes: 3, sourcePath: "/Users/u/Downloads/a.pdf", origin: "picker" },
    ]);
  });

  it("importFiles передаёт operationId и только sourcePath/origin", async () => {
    const importInboxFiles = vi.fn(async (): Promise<OperationResult<InboxImportReport>> => ({ operation, value: report }));
    const adapter = new TauriDesktopBridge(client({ importInboxFiles }));
    const result = await adapter.importFiles(
      [
        { name: "a.pdf", sizeBytes: 3, sourcePath: "/x/a.pdf", origin: "picker" },
        { name: "b.pdf", sizeBytes: 4, sourcePath: "/x/b.pdf" },
      ],
      operation.id,
    );
    expect(importInboxFiles).toHaveBeenCalledWith(operation.id, [
      { sourcePath: "/x/a.pdf", origin: "picker" },
      { sourcePath: "/x/b.pdf", origin: "drop" },
    ]);
    expect(result.value.duplicates).toBe(1);
  });

  it("генерирует UUID v4, если operationId не передан", async () => {
    const importInboxFiles = vi.fn(async (_id: string) => ({ operation, value: report }));
    const adapter = new TauriDesktopBridge(client({ importInboxFiles }));
    await adapter.importFiles([{ name: "a.pdf", sizeBytes: 3, sourcePath: "/x/a.pdf" }]);
    expect(importInboxFiles.mock.calls[0]![0]).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });

  it("элемент без sourcePath останавливает импорт до IPC", async () => {
    const importInboxFiles = vi.fn();
    const adapter = new TauriDesktopBridge(client({ importInboxFiles }));
    await expect(adapter.importFiles([{ name: "browser.pdf", sizeBytes: 1 }])).rejects.toMatchObject({ code: "io_error" });
    expect(importInboxFiles).not.toHaveBeenCalled();
  });

  it("importDroppedFiles возвращает импортированные и найденные дубли без ошибок", async () => {
    const adapter = new TauriDesktopBridge(client({ importInboxFiles: async () => ({ operation, value: report }) }));
    const files = await adapter.importDroppedFiles([{ name: "a.pdf", sizeBytes: 3, sourcePath: "/x/a.pdf" }]);
    expect(files).toHaveLength(2);
  });

  it("ошибка backend не подменяется mock-успехом", async () => {
    const failure = new BridgeError("blocked", "Хранилище Obsidian не выбрано");
    const adapter = new TauriDesktopBridge(client({ importInboxFiles: async () => Promise.reject(failure) }));
    await expect(adapter.importFiles([{ name: "a.pdf", sizeBytes: 3, sourcePath: "/x/a.pdf" }])).rejects.toBe(failure);
  });

  it("getOperationResult превращает not_found в null", async () => {
    const adapter = new TauriDesktopBridge(
      client({ getOperationResult: async () => Promise.reject(new BridgeError("not_found", "нет")) }),
    );
    await expect(adapter.getOperationResult("missing")).resolves.toBeNull();
  });

  it("каталоги и Finder делегируются backend-командам", async () => {
    const calls = {
      checkManagedRoots: vi.fn(async () => roots),
      chooseManagedRoot: vi.fn(async () => ({ cancelled: true, roots })),
      prepareInboxRoot: vi.fn(async () => roots),
      openInboxFile: vi.fn(async () => undefined),
      revealInboxFile: vi.fn(async () => undefined),
      revealInFinder: vi.fn(async () => undefined),
    };
    const adapter = new TauriDesktopBridge(client(calls));
    await adapter.getManagedRoots();
    await expect(adapter.chooseManagedRoot("backup")).resolves.toMatchObject({ cancelled: true });
    await adapter.prepareInboxRoot();
    await adapter.openInboxFile("inbox-1");
    await adapter.revealInboxFile("inbox-1");
    await adapter.revealInFinder("Предметы/00 Входящие");
    expect(calls.chooseManagedRoot).toHaveBeenCalledWith("backup");
    expect(calls.openInboxFile).toHaveBeenCalledWith("inbox-1");
    expect(calls.revealInFinder).toHaveBeenCalledWith("Предметы/00 Входящие");
  });

  it("native drop подписывается на intake:native-drop и помечает origin=drop", () => {
    let handler: ((payload: { items: { name: string; sizeBytes: number; sourcePath: string }[] }) => void) | undefined;
    const unsubscribe = vi.fn();
    const subscribeIntake = vi.fn((event: string, listener: (payload: never) => void) => {
      expect(event).toBe("intake:native-drop");
      handler = listener as typeof handler;
      return unsubscribe;
    });
    const adapter = new TauriDesktopBridge(client({}), undefined, subscribeIntake as never);
    const listener = vi.fn();
    const off = adapter.onNativeFileDrop(listener);
    handler?.({ items: [] });
    expect(listener).not.toHaveBeenCalled();
    handler?.({ items: [{ name: "a.pdf", sizeBytes: 1, sourcePath: "/x/a.pdf" }] });
    expect(listener).toHaveBeenCalledWith([{ name: "a.pdf", sizeBytes: 1, sourcePath: "/x/a.pdf", origin: "drop" }]);
    off();
    expect(unsubscribe).toHaveBeenCalledOnce();
  });
});

describe("intake events: ранняя отписка", () => {
  it("отписка до завершения регистрации снимает listener и не доставляет drop", async () => {
    let resolveRegistration: ((unlisten: () => void) => void) | undefined;
    let emit: ((message: { payload: unknown }) => void) | undefined;
    const nativeUnlisten = vi.fn();
    __setListenForTests((_event, handler) => {
      emit = handler;
      return new Promise((resolve) => {
        resolveRegistration = resolve;
      });
    });
    const listener = vi.fn();
    const unsubscribe = listenToIntakeEvent("intake:native-drop", listener);
    await vi.waitFor(() => expect(resolveRegistration).toBeTypeOf("function"));

    unsubscribe();
    unsubscribe();
    resolveRegistration?.(nativeUnlisten);
    await vi.waitFor(() => expect(nativeUnlisten).toHaveBeenCalledOnce());
    emit?.({ payload: { items: [{ name: "a.pdf", sizeBytes: 1, sourcePath: "/x/a.pdf" }] } });
    expect(listener).not.toHaveBeenCalled();
  });

  it("ранняя отписка адаптера от прогресса импорта не теряет настоящий unlisten", async () => {
    let resolveRegistration: ((unlisten: () => void) => void) | undefined;
    const nativeUnlisten = vi.fn();
    __setListenForTests(
      () =>
        new Promise((resolve) => {
          resolveRegistration = resolve;
        }),
    );
    const adapter = new TauriDesktopBridge(client({}));
    const off = adapter.on("operation:progress", vi.fn());
    off();
    await vi.waitFor(() => expect(resolveRegistration).toBeTypeOf("function"));
    resolveRegistration?.(nativeUnlisten);
    await vi.waitFor(() => expect(nativeUnlisten).toHaveBeenCalledOnce());
  });
});

describe("IPC команды этапа 5", () => {
  it("отправляют точные имена команд и camelCase-аргументы", async () => {
    const invoke = vi.fn(async (command: string) => (command === "pick_import_files" ? [] : { ok: true }));
    __setInvokeForTests(invoke);
    await pickImportFiles();
    await importInboxFiles(operation.id, [{ sourcePath: "/x/a.pdf", origin: "drop" }]);
    await chooseManagedRoot("vault");
    await revealInFinder("Предметы/00 Входящие/a.pdf");
    expect(invoke.mock.calls).toEqual([
      ["pick_import_files", undefined],
      ["import_inbox_files", { operationId: operation.id, items: [{ sourcePath: "/x/a.pdf", origin: "drop" }] }],
      ["choose_managed_root", { kind: "vault" }],
      ["reveal_in_finder", { relativePath: "Предметы/00 Входящие/a.pdf" }],
    ]);
  });

  it("вне Tauri runtime импорт отклоняется backend_unavailable без fallback", async () => {
    __setTauriRuntimeForTests(false);
    await expect(importInboxFiles(operation.id, [])).rejects.toMatchObject({ code: "backend_unavailable" });
  });
});

describe("createBridge: явный флаг разработчика", () => {
  it("создаёт TauriDesktopBridge только при флаге и наличии Tauri runtime", async () => {
    const { createBridge } = await import("../index");
    __setTauriRuntimeForTests(true);
    expect(createBridge("tauri").kind).toBe("tauri");
    expect(createBridge(undefined).kind).toBe("mock");
  });
});
