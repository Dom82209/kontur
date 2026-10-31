import { afterEach, describe, expect, it, vi } from "vitest";

import { BridgeError, type AutomationRules } from "../types";
import {
  __setInvokeForTests,
  backendHealth,
  buildSubmissionPackage,
  classifyFiles,
  confirmSubmission,
  createWeek,
  getActivityLog,
  getAutomationRules,
  getHomework,
  getHomeworkList,
  getInboxFiles,
  getSubjects,
  getStyleProfile,
  getWeek,
  getWeeks,
  importTeacherTranscript,
  previewRoute,
  previewTeacherTranscript,
  removeTeacherComment,
  routeFiles,
  setPackageCheck,
  undoOperation,
  updateAutomationRules,
  updateClassification,
} from "./ipc";

const rules: AutomationRules = {
  inboxPath: "Предметы/00 Входящие",
  studentName: { lastName: "Не задано", firstName: "Не задано" },
  finalNamePattern: "{Фамилия}_{Имя}_Неделя{Неделя} ДЗ",
  subjectRules: [],
  lowConfidenceThreshold: 0.7,
  confirm: { lowConfidenceRoute: true, markFinal: true, markSubmitted: true, allowOverwrite: false },
  workflow: {
    roles: { analyst: "codex", author: "claude", reviewer: "codex" },
    tasks: { material_analysis: "codex", initial_prompt: "codex", solution_review: "codex", revision_prompt: "codex", teacher_review: "codex" },
    styleThreshold: 4,
    styleProfilePath: "ПРОФИЛЬ_СТИЛЯ_ДЗ.md",
    requireTeacherReview: true,
  },
};

afterEach(() => {
  __setInvokeForTests(undefined);
});

describe("низкоуровневый Tauri IPC-клиент", () => {
  it("вызывает точные имена команд и передаёт camelCase-аргументы", async () => {
    const invoke = vi.fn(async () => []);
    __setInvokeForTests(invoke);

    await backendHealth();
    await getSubjects();
    await getWeeks("subject-1");
    await getWeek("subject-1", 7);
    await getHomeworkList();
    await getHomework("homework-1");
    await getInboxFiles();
    await getAutomationRules();
    await getActivityLog();
    await updateAutomationRules(rules);
    await updateClassification("inbox-1", {});

    expect(invoke.mock.calls).toEqual([
      ["backend_health", undefined],
      ["get_subjects", undefined],
      ["get_weeks", { subjectId: "subject-1" }],
      ["get_week", { subjectId: "subject-1", weekNumber: 7 }],
      ["get_homework_list", undefined],
      ["get_homework", { homeworkId: "homework-1" }],
      ["get_inbox_files", undefined],
      ["get_automation_rules", undefined],
      ["get_activity_log", undefined],
      ["update_automation_rules", { rules }],
      ["update_classification", {
        fileId: "inbox-1",
        patch: {
          subjectId: { state: "absent" },
          weekNumber: { state: "absent" },
          category: { state: "absent" },
          homeworkId: { state: "absent" },
        },
      }],
    ]);
  });

  it("передаёт tri-state classification patch без потери absent/null/value", async () => {
    const invoke = vi.fn(async (_command: string, _args?: Record<string, unknown>) => ({}));
    __setInvokeForTests(invoke);

    await updateClassification("inbox-1", {});
    await updateClassification("inbox-1", { subjectId: null, weekNumber: null, category: null });
    await updateClassification("inbox-1", { subjectId: "subject-1", weekNumber: 7, category: "lecture" });

    expect(invoke.mock.calls.map((call) => call[1])).toEqual([
      {
        fileId: "inbox-1",
        patch: {
          subjectId: { state: "absent" },
          weekNumber: { state: "absent" },
          category: { state: "absent" },
          homeworkId: { state: "absent" },
        },
      },
      {
        fileId: "inbox-1",
        patch: {
          subjectId: { state: "null" },
          weekNumber: { state: "null" },
          category: { state: "null" },
          homeworkId: { state: "absent" },
        },
      },
      {
        fileId: "inbox-1",
        patch: {
          subjectId: { state: "value", value: "subject-1" },
          weekNumber: { state: "value", value: 7 },
          category: { state: "value", value: "lecture" },
          homeworkId: { state: "absent" },
        },
      },
    ]);
  });

  it("передаёт AutomationRules без изменения", async () => {
    const invoke = vi.fn(async () => ({ operation: {}, value: rules }));
    __setInvokeForTests(invoke);

    await updateAutomationRules(rules);
    expect(invoke).toHaveBeenCalledWith("update_automation_rules", { rules });
  });

  it("этап 10 передаёт точные IPC-команды и DTO без snake_case", async () => {
    const invoke = vi.fn(async () => ({}));
    __setInvokeForTests(invoke);
    const destination = { subjectId: "subject-1", weekNumber: 7, category: "homework_task" as const };
    const transcript = {
      text: "В: Что исправить?\nО: Добавить вывод.",
      source: "chat" as const,
      date: "2026-10-26T10:00:00Z",
      confirmed: true,
    };
    const confirmation = {
      submittedAt: "2026-10-26T11:00:00Z",
      assignmentUrl: "https://lms.example.edu/task/1",
      proof: null,
      userConfirmed: true as const,
    };

    await createWeek("subject-1", 7);
    await classifyFiles(["file-1", "file-2"]);
    await previewRoute("file-1", destination);
    await routeFiles(["file-1"], destination);
    await routeFiles(["file-2"], null);
    await undoOperation("operation-1");
    await getStyleProfile();
    await previewTeacherTranscript(transcript);
    await importTeacherTranscript("homework-1", transcript);
    await removeTeacherComment("comment-1");
    await buildSubmissionPackage("homework-1");
    await setPackageCheck("package-1", "manual_review", true);
    await confirmSubmission("homework-1", confirmation);

    expect(invoke.mock.calls).toEqual([
      ["create_week", { subjectId: "subject-1", weekNumber: 7 }],
      ["classify_files", { fileIds: ["file-1", "file-2"] }],
      ["preview_route", { fileId: "file-1", destination }],
      ["route_files", { fileIds: ["file-1"], destination }],
      ["route_files", { fileIds: ["file-2"], destination: null }],
      ["undo_operation", { operationId: "operation-1" }],
      ["get_style_profile", undefined],
      ["preview_teacher_transcript", { input: transcript }],
      ["import_teacher_transcript", { homeworkId: "homework-1", input: transcript }],
      ["remove_teacher_comment", { commentId: "comment-1" }],
      ["build_submission_package", { homeworkId: "homework-1" }],
      ["set_package_check", { packageId: "package-1", checkId: "manual_review", done: true }],
      ["confirm_submission", { homeworkId: "homework-1", confirmation }],
    ]);
  });

  it("преобразует ошибку write-команды в BridgeError", async () => {
    __setInvokeForTests(vi.fn(async () => Promise.reject({ code: "conflict", message: "invalid rules" })));

    await expect(updateAutomationRules(rules)).rejects.toMatchObject({
      code: "conflict",
      message: "invalid rules",
    });
  });

  it("возвращает типизированные данные без преобразования DTO", async () => {
    const subjects = [{ id: "s1", name: "ОБА", shortName: "ОБА", currentWeek: 7, weekCount: 2 }];
    __setInvokeForTests(vi.fn(async () => subjects));

    await expect(getSubjects()).resolves.toEqual(subjects);
  });

  it("читает health DTO", async () => {
    const health = {
      ready: true,
      appVersion: "0.1.0",
      schemaVersion: 2,
      platform: "macos-aarch64",
      databaseExists: true,
    };
    __setInvokeForTests(vi.fn(async () => health));

    await expect(backendHealth()).resolves.toEqual(health);
  });

  it("преобразует структурированную Rust-ошибку в BridgeError", async () => {
    __setInvokeForTests(vi.fn(async () => Promise.reject({ code: "not_found", message: "week not found" })));

    const error = await getWeek("s1", 99).catch((value: unknown) => value);
    expect(error).toBeInstanceOf(BridgeError);
    expect(error).toMatchObject({ code: "not_found", message: "week not found" });
  });

  it("преобразует неизвестный Rust-код в unknown, сохраняя сообщение", async () => {
    __setInvokeForTests(vi.fn(async () => Promise.reject({ code: "future_code", message: "future failure" })));

    await expect(getSubjects()).rejects.toMatchObject({ code: "unknown", message: "future failure" });
  });

  it("вне Tauri возвращает backend_unavailable", async () => {
    __setInvokeForTests(null);

    await expect(getSubjects()).rejects.toMatchObject({ code: "backend_unavailable" });
  });

  it("не делает неявный fallback на mock", async () => {
    const invoke = vi.fn(async () => Promise.reject({ code: "database", message: "SQLite unavailable" }));
    __setInvokeForTests(invoke);

    await expect(getSubjects()).rejects.toMatchObject({ code: "database" });
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
