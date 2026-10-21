import { describe, expect, it, vi } from "vitest";

import { bridge } from "../index";
import type { AutomationRules, Homework, Operation, Subject, Week } from "../types";
import { BridgeError } from "../types";
import { TauriDesktopBridge, type TauriIpcClient } from "./TauriDesktopBridge";
import { createTauriBridge } from "./index";

const subject: Subject = {
  id: "subject-1",
  name: "Основы бизнес-аналитики",
  shortName: "ОБА",
  currentWeek: 7,
  weekCount: 1,
};

const week: Week = {
  id: "week-7",
  subjectId: subject.id,
  number: 7,
  startsAt: "2026-10-12T00:00:00Z",
  endsAt: "2026-10-18T23:59:59Z",
  topic: "BPMN",
  lectures: [],
  seminars: [],
  materials: [],
  homeworkId: "homework-7",
};

const homework: Homework = {
  id: "homework-7",
  subjectId: subject.id,
  weekNumber: 7,
  title: "Моделирование процесса",
  deadline: "2026-10-20T18:00:00Z",
  stage: "task",
  taskFiles: [],
  versions: [],
  finalVersionId: null,
  finalFileName: null,
  submittedAt: null,
  folderPath: "Предметы/ОБА/Неделя 07/03 ДЗ",
  workflowStage: "materials_collected",
  assignmentUrl: null,
};

const rules: AutomationRules = {
  inboxPath: "Предметы/00 Входящие",
  downloadsWatchPath: "Загрузки",
  obsidianVaultPath: ".",
  studentName: { lastName: "Не задано", firstName: "Не задано" },
  finalNamePattern: "{Фамилия}_{Имя}_Неделя{Неделя} ДЗ",
  versionPattern: "Работа_Неделя{Неделя}_v{Версия}",
  weekDetection: "filename_then_calendar",
  semesterStart: "1970-01-05T00:00:00Z",
  subjectRules: [],
  lowConfidenceThreshold: 0.7,
  backupBeforeChange: true,
  autoRouteHighConfidence: false,
  confirm: {
    lowConfidenceRoute: true,
    markFinal: true,
    markSubmitted: true,
    allowOverwrite: false,
  },
  workflow: {
    roles: { analyst: "codex", author: "claude", reviewer: "codex" },
    styleThreshold: 4,
    styleProfilePath: "ПРОФИЛЬ_СТИЛЯ_ДЗ.md",
    requireTeacherReview: true,
  },
};

const operation: Operation = {
  id: "operation-1",
  kind: "route",
  title: "Раскладка",
  detail: null,
  status: "completed",
  startedAt: "2026-10-21T10:00:00Z",
  finishedAt: "2026-10-21T10:01:00Z",
  undoable: true,
  paths: ["Предметы/ОБА/Неделя 07"],
  backupPath: null,
  homeworkId: homework.id,
};

function readClient(overrides: Partial<TauriIpcClient> = {}): TauriIpcClient {
  return {
    getSubjects: async () => [subject],
    getWeeks: async () => [week],
    getWeek: async () => week,
    getInboxFiles: async () => [],
    getHomeworkList: async () => [homework],
    getHomework: async () => homework,
    getAutomationRules: async () => rules,
    getActivityLog: async () => [operation],
    updateAutomationRules: async (updatedRules) => ({ operation, value: updatedRules }),
    updateClassification: async () => ({
      id: "inbox-1",
      name: "file.pdf",
      kind: "pdf",
      sizeBytes: 1,
      addedAt: "2026-10-21T10:00:00Z",
      origin: "drop",
      status: "ambiguous",
      classification: null,
      duplicate: null,
    }),
    checkManagedRoots: unexpected("checkManagedRoots"),
    chooseManagedRoot: unexpected("chooseManagedRoot"),
    prepareInboxRoot: unexpected("prepareInboxRoot"),
    pickImportFiles: unexpected("pickImportFiles"),
    importInboxFiles: unexpected("importInboxFiles"),
    getOperationResult: unexpected("getOperationResult"),
    openInboxFile: unexpected("openInboxFile"),
    revealInboxFile: unexpected("revealInboxFile"),
    revealInFinder: unexpected("revealInFinder"),
    ...overrides,
  };
}

function unexpected(name: string): () => Promise<never> {
  return () => Promise.reject(new Error(`unexpected IPC call: ${name}`));
}

describe("TauriDesktopBridge", () => {
  it("имеет kind=tauri, а фабрика создаёт настоящий Tauri-адаптер", () => {
    expect(new TauriDesktopBridge(readClient()).kind).toBe("tauri");
    expect(createTauriBridge()).toBeInstanceOf(TauriDesktopBridge);
  });

  it("все реализованные read-методы делегируют точному IPC-клиенту", async () => {
    const client = readClient({
      getSubjects: vi.fn(async () => [subject]),
      getWeeks: vi.fn(async () => [week]),
      getWeek: vi.fn(async () => week),
      getInboxFiles: vi.fn(async () => []),
      getHomeworkList: vi.fn(async () => [homework]),
      getHomework: vi.fn(async () => homework),
      getAutomationRules: vi.fn(async () => rules),
      getActivityLog: vi.fn(async () => [operation]),
      updateAutomationRules: vi.fn(async (updatedRules) => ({ operation, value: updatedRules })),
      updateClassification: vi.fn(async () => ({
        id: "inbox-1",
        name: "file.pdf",
        kind: "pdf" as const,
        sizeBytes: 1,
        addedAt: "2026-10-21T10:00:00Z",
        origin: "drop" as const,
        status: "ambiguous" as const,
        classification: null,
        duplicate: null,
      })),
    });
    const adapter = new TauriDesktopBridge(client);

    await adapter.getSubjects();
    await adapter.getWeeks("subject-1");
    await adapter.getWeek("subject-1", 7);
    await adapter.getInboxFiles();
    await adapter.getHomeworkList();
    await adapter.getHomework("homework-7");
    await adapter.getAutomationRules();
    await adapter.getActivityLog();
    await adapter.updateAutomationRules(rules);
    await adapter.updateClassification("inbox-1", { subjectId: null });

    expect(client.getSubjects).toHaveBeenCalledOnce();
    expect(client.getWeeks).toHaveBeenCalledWith("subject-1");
    expect(client.getWeek).toHaveBeenCalledWith("subject-1", 7);
    expect(client.getInboxFiles).toHaveBeenCalledOnce();
    expect(client.getHomeworkList).toHaveBeenCalledOnce();
    expect(client.getHomework).toHaveBeenCalledWith("homework-7");
    expect(client.getAutomationRules).toHaveBeenCalledOnce();
    expect(client.getActivityLog).toHaveBeenCalledOnce();
    expect(client.updateAutomationRules).toHaveBeenCalledWith(rules);
    expect(client.updateClassification).toHaveBeenCalledWith("inbox-1", { subjectId: null });
  });

  it("преобразует not_found в null только для Week и Homework", async () => {
    const missing = new BridgeError("not_found", "missing");
    const adapter = new TauriDesktopBridge(readClient({
      getWeek: async () => Promise.reject(missing),
      getHomework: async () => Promise.reject(missing),
    }));

    await expect(adapter.getWeek("subject-1", 99)).resolves.toBeNull();
    await expect(adapter.getHomework("missing")).resolves.toBeNull();
  });

  it.each(["database", "backend_unavailable"] as const)(
    "не преобразует ошибку %s в null",
    async (code) => {
      const failure = new BridgeError(code, "failure");
      const adapter = new TauriDesktopBridge(readClient({
        getWeek: async () => Promise.reject(failure),
        getHomework: async () => Promise.reject(failure),
      }));

      await expect(adapter.getWeek("subject-1", 7)).rejects.toBe(failure);
      await expect(adapter.getHomework("homework-7")).rejects.toBe(failure);
    },
  );

  it("unsupported-метод отклоняется с backend_unavailable без mock fallback", async () => {
    const getSubjects = vi.fn(async () => Promise.reject(new BridgeError("database", "SQLite unavailable")));
    const adapter = new TauriDesktopBridge(readClient({ getSubjects }));

    await expect(adapter.getDashboard()).rejects.toMatchObject({ code: "backend_unavailable" });
    await expect(adapter.getSubjects()).rejects.toMatchObject({ code: "database" });
    expect(getSubjects).toHaveBeenCalledOnce();
  });

  it("основная factory-точка UI всё ещё создаёт mock", () => {
    expect(bridge.kind).toBe("mock");
  });
});
