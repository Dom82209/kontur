import { describe, expect, it } from "vitest";
import type { DroppedItem } from "../types";
import { MockDesktopBridge } from "./MockDesktopBridge";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

async function imported(bridge: MockDesktopBridge, item: DroppedItem) {
  const [file] = await bridge.importDroppedFiles([item]);
  await settle();
  return file!;
}

describe("Mock: несколько ДЗ в неделе (этап 4)", () => {
  it("три ДЗ одной недели получают порядковые номера и свои папки", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const week = await bridge.getWeek("stat", 8);
    const existing = week!.homeworkIds.length;
    const second = await bridge.createHomework({ subjectId: "stat", weekNumber: 8, title: "Кейс", deadline: null });
    const third = await bridge.createHomework({ subjectId: "stat", weekNumber: 8, title: "Тест", deadline: "2026-11-05T23:59:00+03:00" });
    expect(second.ordinal).toBe(existing + 1);
    expect(third.ordinal).toBe(existing + 2);
    expect(third.folderPath).toBe(`Предметы/Статистика/Неделя 08/03 ДЗ (${existing + 2})`);
    expect(third.deadline).toBe("2026-11-05T20:59:00Z");
    expect(second.deadline).toBeNull();
    expect((await bridge.getWeek("stat", 8))!.homeworkIds).toEqual([...week!.homeworkIds, second.id, third.id]);
    await expect(bridge.createHomework({ subjectId: "stat", weekNumber: 99, title: "x", deadline: null })).rejects.toMatchObject({ code: "not_found" });
  });

  it("файл ДЗ в неделе с несколькими ДЗ раскладывается только после выбора ДЗ", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    await bridge.createHomework({ subjectId: "stat", weekNumber: 8, title: "Тест", deadline: null });
    const extra = await bridge.createHomework({ subjectId: "stat", weekNumber: 8, title: "Кейс", deadline: null });
    expect((await bridge.getWeek("stat", 8))!.homeworkIds.length).toBeGreaterThan(1);
    const file = await imported(bridge, { name: "решение кейса.ipynb", sizeBytes: 900 });
    const destination = { subjectId: "stat", weekNumber: 8, category: "homework_draft" as const };
    await expect(bridge.previewRoute(file.id, destination)).rejects.toMatchObject({ code: "confirmation_required" });
    const preview = await bridge.previewRoute(file.id, { ...destination, homeworkId: extra.id });
    expect(preview.targetPath.startsWith(`${extra.folderPath}/02 Работа/`)).toBe(true);
    await bridge.updateClassification(file.id, { subjectId: "stat", weekNumber: 8, category: "homework_draft", homeworkId: extra.id });
    const result = await bridge.routeFiles([file.id], null);
    expect(result.value[0]!.targetPath.startsWith(extra.folderPath)).toBe(true);
  });
});
