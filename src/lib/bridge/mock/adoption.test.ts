import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

describe("Mock: подключение существующего vault (этап 2)", () => {
  it("preview показывает план и пропуски, dry-run ничего не применяет", async () => {
    const bridge = new MockDesktopBridge();
    const preview = await bridge.previewVaultAdoption();
    expect(preview.summary.filesNew).toBe(3);
    expect(preview.summary.needsReview).toBe(1);
    expect(preview.plan.skipped.map((item) => item.reason)).toEqual(["subject_level", "obsidian_note"]);

    const dry = await bridge.applyVaultAdoption(preview.id, [], true);
    expect(dry).toMatchObject({ dryRun: true, status: "previewed", created: { files: 3 } });
    expect((await bridge.previewVaultAdoption()).summary.filesNew).toBe(3);
  });

  it("apply одноразовый и идемпотентен при повторном preview", async () => {
    const bridge = new MockDesktopBridge();
    const preview = await bridge.previewVaultAdoption();
    const report = await bridge.applyVaultAdoption(preview.id, [], false);
    expect(report).toMatchObject({ status: "applied", created: { subjects: 1, weeks: 1, homeworks: 1, files: 3 } });
    await expect(bridge.applyVaultAdoption(preview.id, [], false)).rejects.toMatchObject({ code: "conflict" });

    const again = await bridge.previewVaultAdoption();
    expect(again.summary).toMatchObject({ filesNew: 0, filesExisting: 3, subjectsExisting: 1 });
    expect((await bridge.applyVaultAdoption(again.id, [], false)).created.files).toBe(0);
  });

  it("ручная категория ДЗ недоступна файлу вне «03 ДЗ», план остаётся доступным", async () => {
    const bridge = new MockDesktopBridge();
    const preview = await bridge.previewVaultAdoption();
    const lecture = preview.plan.files.find((file) => file.category === "lecture")!;
    await expect(
      bridge.applyVaultAdoption(preview.id, [{ relativePath: lecture.relativePath, category: "homework_task" }], false),
    ).rejects.toMatchObject({ code: "blocked" });
    const draft = preview.plan.files.find((file) => file.needsReview)!;
    const report = await bridge.applyVaultAdoption(preview.id, [{ relativePath: draft.relativePath, category: "homework_data" }], false);
    expect(report.overridesApplied).toBe(1);
  });

  it("отменённый план нельзя применить", async () => {
    const bridge = new MockDesktopBridge();
    const preview = await bridge.previewVaultAdoption();
    await bridge.cancelVaultAdoption(preview.id);
    await expect(bridge.applyVaultAdoption(preview.id, [], false)).rejects.toMatchObject({ code: "conflict" });
    await expect(bridge.cancelVaultAdoption(preview.id)).rejects.toMatchObject({ code: "conflict" });
  });
});
