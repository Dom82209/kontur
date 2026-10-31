import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

describe("правила автоматизации (этап 6, mock повторяет контракт backend)", () => {
  it("в контракте нет неисполняемых полей", async () => {
    const rules = await new MockDesktopBridge({ latency: 0 }).getAutomationRules();
    for (const retired of ["semesterStart", "weekDetection", "backupBeforeChange", "autoRouteHighConfidence", "versionPattern", "downloadsWatchPath", "obsidianVaultPath"]) {
      expect(rules, retired).not.toHaveProperty(retired);
    }
    expect(rules.confirm).toEqual({ lowConfidenceRoute: true, markFinal: true, markSubmitted: true, allowOverwrite: false });
  });

  it("сохранённое значение возвращается при следующем чтении и не отменяется журналом", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const rules = await b.getAutomationRules();
    const saved = await b.updateAutomationRules({ ...rules, lowConfidenceThreshold: 0.85, finalNamePattern: "ДЗ {Неделя}" });
    expect(saved.operation.undoable).toBe(false);
    const read = await b.getAutomationRules();
    expect(read.lowConfidenceThreshold).toBe(0.85);
    expect(read.finalNamePattern).toBe("ДЗ {Неделя}");
  });

  it("отклоняет нарушение инвариантов и смену папки «Входящих» без изменений", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const rules = await b.getAutomationRules();
    const broken = [
      { ...rules, inboxPath: "Другое" },
      { ...rules, finalNamePattern: "{Фамилия}" },
      { ...rules, confirm: { ...rules.confirm, lowConfidenceRoute: false } },
      { ...rules, confirm: { ...rules.confirm, markFinal: false } },
    ];
    for (const next of broken) await expect(b.updateAutomationRules(next as typeof rules)).rejects.toBeTruthy();
    expect(await b.getAutomationRules()).toEqual(rules);
  });
});
