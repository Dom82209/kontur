import { afterEach, describe, expect, it, vi } from "vitest";

import type { DownloadsWatcher, ExportImportRequest, InboxImportReport, Operation } from "../types";
import { BridgeError } from "../types";
import { __setListenForTests, listenToBridgeEvent } from "./events";
import {
  __setInvokeForTests,
  dismissDownloadCandidate,
  getDownloadsWatcher,
  importCuExport,
  openAssignment,
  openCourse,
  pickCuExport,
  previewCuExport,
  setDownloadsWatcher,
  setHomeworkAssignmentUrl,
  setSubjectCourseUrl,
} from "./ipc";
import { __setTauriRuntimeForTests } from "./runtime";
import { TauriDesktopBridge, type TauriIpcClient } from "./TauriDesktopBridge";

afterEach(() => {
  __setInvokeForTests(undefined);
  __setListenForTests(undefined);
  __setTauriRuntimeForTests(undefined);
});

const watcher: DownloadsWatcher = { enabled: false, state: "disabled", message: null, lastScanAt: null, candidates: [] };

const request: ExportImportRequest = {
  candidateId: "cand-1",
  archiveSha256: "a".repeat(64),
  entries: [{ entryId: "2", subjectId: "oba", weekNumber: 3, category: "lecture", edited: false }],
};

describe("этап 6: IPC-команды экспорта ЦУ и watcher", () => {
  it("вызывает точные имена команд с camelCase-аргументами", async () => {
    const invoke = vi.fn(async () => null);
    __setInvokeForTests(invoke);

    await getDownloadsWatcher();
    await setDownloadsWatcher(true);
    await dismissDownloadCandidate("cand-1");
    await pickCuExport();
    await previewCuExport("cand-1");
    await importCuExport("op-1", request);
    await setSubjectCourseUrl("oba", "https://my.centraluniversity.ru/learn/courses/1");
    await setHomeworkAssignmentUrl("hw-1", null);
    await openCourse({ subjectId: "oba" });
    await openCourse({ fileId: "inbox-1" });
    await openAssignment({ homeworkId: "hw-1" });
    await openAssignment({ fileId: "inbox-1" });

    expect(invoke.mock.calls).toEqual([
      ["get_downloads_watcher", undefined],
      ["set_downloads_watcher", { enabled: true }],
      ["dismiss_download_candidate", { candidateId: "cand-1" }],
      ["pick_cu_export", undefined],
      ["preview_cu_export", { candidateId: "cand-1" }],
      ["import_cu_export", { operationId: "op-1", request: { ...request, courseUrl: null, assignmentUrl: null } }],
      ["set_subject_course_url", { subjectId: "oba", url: "https://my.centraluniversity.ru/learn/courses/1" }],
      ["set_homework_assignment_url", { homeworkId: "hw-1", url: null }],
      ["open_course", { subjectId: "oba", fileId: null }],
      ["open_course", { subjectId: null, fileId: "inbox-1" }],
      ["open_assignment", { homeworkId: "hw-1", fileId: null }],
      ["open_assignment", { homeworkId: null, fileId: "inbox-1" }],
    ]);
  });

  it("ошибка backend (например, allowlist) приходит как BridgeError без fallback", async () => {
    __setInvokeForTests(async () => {
      throw { code: "blocked", message: "Разрешены только ссылки на my.centraluniversity.ru" };
    });
    await expect(openCourse({ subjectId: "oba" })).rejects.toMatchObject({ code: "blocked" });
    await expect(previewCuExport("cand-1")).rejects.toBeInstanceOf(BridgeError);
  });

  it("событие downloads:watcher-updated доставляется типизированно", async () => {
    __setTauriRuntimeForTests(true);
    let handler: ((message: { payload: unknown }) => void) | undefined;
    __setListenForTests(async (event, h) => {
      expect(event).toBe("downloads:watcher-updated");
      handler = h;
      return () => undefined;
    });
    const received: DownloadsWatcher[] = [];
    const off = listenToBridgeEvent("downloads:watcher-updated", ({ watcher: w }) => received.push(w));
    await vi.waitFor(() => expect(handler).toBeDefined());
    handler!({ payload: { watcher: { ...watcher, enabled: true, state: "watching" } } });
    expect(received[0]?.state).toBe("watching");
    off();
  });
});

describe("TauriDesktopBridge: этап 6 делегирует IPC и генерирует operationId", () => {
  const operation: Operation = { id: "op", kind: "import", title: "Импорт экспорта ЦУ", detail: null, status: "completed", startedAt: "2026-10-21T10:00:00Z", finishedAt: "2026-10-21T10:00:01Z", undoable: false, paths: [], backupPath: null, homeworkId: null };
  const report: InboxImportReport = { operationId: "op", items: [], imported: 0, duplicates: 0, failed: 0, skipped: 2, replayed: false };

  function client(overrides: Partial<TauriIpcClient>): TauriIpcClient {
    return new Proxy(overrides as TauriIpcClient, {
      get: (target, key: string) => (key in target ? target[key as keyof TauriIpcClient] : () => Promise.reject(new Error(`unexpected IPC call: ${key}`))),
    });
  }

  it("importCuExport без operationId создаёт UUID v4 и передаёт запрос без изменений", async () => {
    const importCuExportMock = vi.fn(async () => ({ operation, value: report }));
    const adapter = new TauriDesktopBridge(client({ importCuExport: importCuExportMock }));
    const result = await adapter.importCuExport(request);
    expect(result.value.skipped).toBe(2);
    const [operationId, sent] = importCuExportMock.mock.calls[0] as unknown as [string, ExportImportRequest];
    expect(operationId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(sent).toEqual(request);
  });

  it("отмена системного выбора экспорта — null, без предпросмотра", async () => {
    const previewMock = vi.fn();
    const adapter = new TauriDesktopBridge(client({ pickCuExport: async () => null, previewCuExport: previewMock }));
    await expect(adapter.pickCuExport()).resolves.toBeNull();
    expect(previewMock).not.toHaveBeenCalled();
  });

  it("watcher-команды и ссылки идут в настоящий backend", async () => {
    const setWatcher = vi.fn(async () => ({ ...watcher, enabled: true, state: "watching" as const }));
    const open = vi.fn(async () => undefined);
    const adapter = new TauriDesktopBridge(client({ setDownloadsWatcher: setWatcher, openAssignment: open }));
    await expect(adapter.setDownloadsWatcher(true)).resolves.toMatchObject({ state: "watching" });
    await adapter.openAssignment({ fileId: "inbox-1" });
    expect(setWatcher).toHaveBeenCalledWith(true);
    expect(open).toHaveBeenCalledWith({ fileId: "inbox-1" });
  });
});
