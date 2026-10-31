import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

describe("Mock: предметы (этап 3)", () => {
  it("создаёт предмет без записи в vault и нормализует ключевые слова", async () => {
    const bridge = new MockDesktopBridge();
    const subject = await bridge.createSubject({ name: "  Теория   вероятностей ", shortName: "ТВ", keywords: ["ТВиМС", "твимс"] });
    expect(subject).toMatchObject({ name: "Теория вероятностей", folderPath: "Предметы/Теория вероятностей", keywords: ["твимс"], archived: false, revision: 0 });
    await expect(bridge.createSubject({ name: "теория вероятностей", shortName: "Т", keywords: [] })).rejects.toMatchObject({ code: "conflict" });
    await expect(bridge.createSubject({ name: "00 Входящие", shortName: "В", keywords: [] })).rejects.toMatchObject({ code: "blocked" });
  });

  it("ключевое слово не может принадлежать двум активным предметам", async () => {
    const bridge = new MockDesktopBridge();
    await expect(bridge.createSubject({ name: "Матстат", shortName: "МС", keywords: ["стат"] })).rejects.toMatchObject({ code: "conflict" });
  });

  it("переименование сохраняет папку, устаревшая ревизия отклоняется", async () => {
    const bridge = new MockDesktopBridge();
    const [meow] = (await bridge.getSubjects()).filter((s) => s.id === "meow");
    const renamed = await bridge.updateSubject("meow", meow!.revision, { name: "Математика и алгебра", shortName: "МиА", keywords: ["миа"] });
    expect(renamed).toMatchObject({ name: "Математика и алгебра", folderPath: "Предметы/Мяу", revision: 1 });
    await expect(bridge.updateSubject("meow", 0, { name: "X", shortName: "X", keywords: [] })).rejects.toMatchObject({ code: "conflict" });
  });

  it("слияние: предпросмотр ничего не меняет, конфликт недель блокирует", async () => {
    const bridge = new MockDesktopBridge();
    const preview = await bridge.previewSubjectMerge("matan-adv", "matan");
    expect(preview.canMerge).toBe(false);
    expect(preview.conflictingWeeks.length).toBeGreaterThan(0);
    await expect(bridge.mergeSubjects("matan-adv", "matan", preview.fingerprint)).rejects.toMatchObject({ code: "blocked" });
    expect((await bridge.getSubjects()).find((s) => s.id === "matan-adv")?.archived).toBe(false);
  });

  it("слияние в новый пустой предмет переносит недели и архивирует источник", async () => {
    const bridge = new MockDesktopBridge();
    const target = await bridge.createSubject({ name: "Стресс и сон", shortName: "СиС", keywords: [] });
    const preview = await bridge.previewSubjectMerge("stress", target.id);
    expect(preview.canMerge).toBe(true);
    const result = await bridge.mergeSubjects("stress", target.id, preview.fingerprint);
    expect(result.source).toMatchObject({ archived: true, mergedIntoId: target.id });
    expect(result.target.keywords).toContain("стресс");
    expect(result.movedWeeks).toBe(preview.movedWeeks.length);
    await expect(bridge.setSubjectArchived("stress", result.source.revision, false)).rejects.toMatchObject({ code: "blocked" });
    await expect(bridge.previewSubjectMerge("stress", target.id)).rejects.toMatchObject({ code: "blocked" });
  });
});
