import { afterEach, describe, expect, it, vi } from "vitest";

import { BridgeError } from "../types";
import {
  __setInvokeForTests,
  backendHealth,
  getActivityLog,
  getAutomationRules,
  getHomework,
  getHomeworkList,
  getInboxFiles,
  getSubjects,
  getWeek,
  getWeeks,
} from "./ipc";

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
    ]);
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
