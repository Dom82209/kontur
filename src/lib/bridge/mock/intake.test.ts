import { describe, expect, it, vi } from "vitest";
import { createBridge } from "../index";
import { filesFromImportReport, newOperationId } from "../intake";
import { MockDesktopBridge } from "./MockDesktopBridge";

describe("mock: импорт во «Входящие» этапа 5", () => {
  it("эмитит прогресс с operationId, затем file-added, inbox:updated и completed", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const operationId = newOperationId();
    const events: string[] = [];
    const progress: number[] = [];
    bridge.on("operation:progress", (p) => {
      if (p.operationId === operationId) progress.push(p.progress);
    });
    bridge.on("inbox:file-added", () => events.push("file-added"));
    bridge.on("inbox:updated", (p) => events.push(`updated:${p.fileIds.length}`));
    bridge.on("operation:completed", ({ operation }) => {
      if (operation.id === operationId) events.push("completed");
    });

    const result = await bridge.importFiles(
      [
        { name: "Лекция 9.pdf", sizeBytes: 100 },
        { name: "Семинар 9.docx", sizeBytes: 50 },
      ],
      operationId,
    );

    expect(result.operation).toMatchObject({ id: operationId, kind: "import", status: "completed", undoable: false });
    expect(result.value.imported).toBe(2);
    expect(progress).toEqual([0, 0.5, 1]);
    expect(events).toEqual(["file-added", "updated:2", "completed"]);
  });

  it("тот же файл повторно — дубль, то же имя с другим размером — « (2)»", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    await bridge.importFiles([{ name: "Задание.pdf", sizeBytes: 10 }]);
    const again = await bridge.importFiles([
      { name: "Задание.pdf", sizeBytes: 10 },
      { name: "Задание.pdf", sizeBytes: 11 },
    ]);
    const [duplicate, renamed] = again.value.items;
    expect(duplicate).toMatchObject({ status: "duplicate", restored: false });
    expect(renamed).toMatchObject({ status: "imported", renamedFrom: "Задание.pdf" });
    expect(renamed?.status === "imported" && renamed.file.name).toBe("Задание (2).pdf");
    expect(filesFromImportReport(again.value)).toHaveLength(2);
  });

  it("повтор команды с тем же operationId возвращает сохранённый итог без новых файлов", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const operationId = newOperationId();
    const first = await bridge.importFiles([{ name: "Отчёт.xlsx", sizeBytes: 5 }], operationId);
    const added = vi.fn();
    bridge.on("inbox:file-added", added);
    const second = await bridge.importFiles([{ name: "Отчёт.xlsx", sizeBytes: 5 }], operationId);
    expect(second.value.replayed).toBe(true);
    expect(second.value.items).toEqual(first.value.items);
    expect(added).not.toHaveBeenCalled();
    await expect(bridge.getOperationResult(operationId)).resolves.toMatchObject({ importReport: { imported: 1 } });
  });

  it("пустой набор отклоняется structured-ошибкой", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    await expect(bridge.importFiles([])).rejects.toMatchObject({ code: "blocked" });
  });

  it("управляемые каталоги: выбор и подготовка «Входящих»", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const before = await bridge.getManagedRoots();
    expect(before.backup.state).toBe("not_configured");
    const selection = await bridge.chooseManagedRoot("backup");
    expect(selection.cancelled).toBe(false);
    expect(selection.roots.backup.state).toBe("ok");
    expect((await bridge.prepareInboxRoot()).inbox.state).toBe("ok");
  });

  it("native-события в mock — безопасные пустые подписки", () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    expect(() => bridge.onNativeFileDrop(() => undefined)()).not.toThrow();
    expect(() => bridge.onNativeDragState(() => undefined)()).not.toThrow();
  });
});

describe("factory", () => {
  it("по умолчанию и вне Tauri runtime создаёт mock, даже с флагом", () => {
    expect(createBridge(undefined).kind).toBe("mock");
    expect(createBridge("tauri").kind).toBe("mock");
  });

  it("newOperationId выдаёт UUID v4", () => {
    expect(newOperationId()).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });
});
