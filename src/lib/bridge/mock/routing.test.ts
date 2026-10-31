import { describe, expect, it } from "vitest";
import { assertNativeSourcePaths } from "../intake";
import type { DroppedItem, Homework, RouteDestination } from "../types";
import { MockDesktopBridge } from "./MockDesktopBridge";
import { CATEGORY_FOLDER, weekFolder } from "./data";

const settle = () => new Promise((resolve) => setTimeout(resolve, 10));

async function imported(bridge: MockDesktopBridge, item: DroppedItem) {
  const [file] = await bridge.importDroppedFiles([item]);
  await settle(); // дождаться фоновой mock-классификации
  return file!;
}

async function routeOne(bridge: MockDesktopBridge, item: DroppedItem, destination: RouteDestination) {
  const file = await imported(bridge, item);
  const result = await bridge.routeFiles([file.id], destination);
  return { file, result };
}

describe("раскладка файлов этапа 1", () => {
  it("weekFolder начинается с Предметы/", () => {
    expect(weekFolder("linal", 8)).toBe("Предметы/Линейная алгебра/Неделя 08");
  });

  it("для дополнительных материалов используется 04 Дополнительно", async () => {
    expect(CATEGORY_FOLDER.material).toBe("04 Дополнительно");
    const bridge = new MockDesktopBridge({ latency: 0 });
    const file = await imported(bridge, { name: "Дополнение.pdf", sizeBytes: 123 });
    const preview = await bridge.previewRoute(file.id, { subjectId: "linal", weekNumber: 8, category: "material" });
    expect(preview.targetPath).toContain("/04 Дополнительно/");
  });

  it("homework_task появляется в Homework.taskFiles", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const { file } = await routeOne(bridge, { name: "Новое условие.pdf", sizeBytes: 500 }, { subjectId: "linal", weekNumber: 8, category: "homework_task" });
    const homework = await bridge.getHomework("hw-linal-8");
    expect(homework?.taskFiles.some((item) => item.id === file.id)).toBe(true);
  });

  it("при отсутствии Homework создаёт его и связывает с Week", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    expect(await bridge.getWeek("stress", 8)).toBeNull();
    await routeOne(bridge, { name: "Условие недели 8.pdf", sizeBytes: 700 }, { subjectId: "stress", weekNumber: 8, category: "homework_task" });
    const week = await bridge.getWeek("stress", 8);
    const homework = week?.homeworkIds[0] ? await bridge.getHomework(week.homeworkIds[0]) : null;
    expect(week?.homeworkIds).toEqual(["hw-stress-8"]);
    expect(homework).toMatchObject({ subjectId: "stress", weekNumber: 8, folderPath: "Предметы/Стресс-менеджмент/Неделя 08/03 ДЗ" });
  });

  it("разложенный файл ДЗ доступен в SourceBundle", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const { file } = await routeOne(bridge, { name: "Условие проекта.pdf", sizeBytes: 800 }, { subjectId: "stress", weekNumber: 8, category: "homework_task" });
    const bundle = await bridge.prepareSourceBundle("hw-stress-8", "analyze_materials");
    expect(bundle.items.some((item) => item.file.id === file.id && item.kind === "task")).toBe(true);
  });

  it("operation:completed публикуется после обновления состояния и до resolve", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const file = await imported(bridge, { name: "Условие к событию.pdf", sizeBytes: 900 });
    let resolved = false;
    let beforeResolve = false;
    let resolveSnapshot!: (value: Homework | null) => void;
    const snapshot = new Promise<Homework | null>((resolve) => {
      resolveSnapshot = resolve;
    });
    const off = bridge.on("operation:completed", ({ operation }) => {
      if (operation.kind !== "route") return;
      beforeResolve = !resolved;
      void bridge.getHomework("hw-stress-8").then(resolveSnapshot);
    });
    await bridge.routeFiles([file.id], { subjectId: "stress", weekNumber: 8, category: "homework_task" });
    resolved = true;
    off();
    expect(beforeResolve).toBe(true);
    expect((await snapshot)?.taskFiles.some((item) => item.id === file.id)).toBe(true);
  });

  it("undo полностью восстанавливает Week, Homework и файл во входящих", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const file = await imported(bridge, { name: "Условие для undo.pdf", sizeBytes: 1_000 });
    const homeworkBefore = await bridge.getHomeworkList();
    const routed = await bridge.routeFiles([file.id], { subjectId: "stress", weekNumber: 8, category: "homework_task" });
    expect(await bridge.getWeek("stress", 8)).not.toBeNull();
    await bridge.undoOperation(routed.operation.id);
    expect(await bridge.getWeek("stress", 8)).toBeNull();
    expect(await bridge.getHomeworkList()).toEqual(homeworkBefore);
    expect((await bridge.getInboxFiles()).some((item) => item.id === file.id)).toBe(true);
  });

  it("ищет дубликаты во всех хранилищах недели и не перезаписывает файл", async () => {
    const cases: { subjectId: string; weekNumber: number; category: RouteDestination["category"]; name: string; prepare?: (bridge: MockDesktopBridge) => Promise<string> }[] = [
      { subjectId: "matan", weekNumber: 8, category: "lecture", name: "Лекция 8. Ряды.pdf" },
      { subjectId: "matan", weekNumber: 8, category: "seminar", name: "Семинар 8.pdf" },
      { subjectId: "matan", weekNumber: 6, category: "material", name: "Доп. материалы — Исследование функций.pdf" },
      { subjectId: "linal", weekNumber: 8, category: "homework_task", name: "Условие ДЗ 8.pdf" },
      { subjectId: "oba", weekNumber: 7, category: "homework_draft", name: "Работа_Неделя7_v01.xlsx" },
      {
        subjectId: "linal",
        weekNumber: 8,
        category: "homework_draft",
        name: "",
        prepare: async (bridge) => (await bridge.getHomeworkCandidates("hw-linal-8"))[0]!.name,
      },
    ];
    for (const item of cases) {
      const bridge = new MockDesktopBridge({ latency: 0 });
      const name = item.prepare ? await item.prepare(bridge) : item.name;
      const file = await imported(bridge, { name, sizeBytes: 1_000 });
      const destination = { subjectId: item.subjectId, weekNumber: item.weekNumber, category: item.category };
      const preview = await bridge.previewRoute(file.id, destination);
      expect(preview.duplicate, name).not.toBeNull();
      expect(preview.targetName, name).not.toBe(name);
      await bridge.routeFiles([file.id], destination);
      const second = await bridge.previewRoute((await imported(bridge, { name, sizeBytes: 1_000 })).id, destination);
      expect(second.targetName, name).not.toBe(preview.targetName);
    }
  });

  it("browser/mock импорт без sourcePath продолжает работать, а native guard его отклоняет", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const [file] = await bridge.importDroppedFiles([{ name: "browser-file.pdf", sizeBytes: 42 }]);
    expect(file?.name).toBe("browser-file.pdf");
    expect(() => assertNativeSourcePaths([{ name: "native-without-path.pdf", sizeBytes: 42 }])).toThrow(/исходный путь/);
    expect(() => assertNativeSourcePaths([{ name: "native.pdf", sizeBytes: 42, sourcePath: "/tmp/native.pdf" }])).not.toThrow();
  });
});
