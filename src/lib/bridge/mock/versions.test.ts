import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

const HW = "hw-oba-7";

/** Этап 10: mock повторяет правила родословной, решений, сравнения и плана исправлений. */
describe("версии: родословная и сравнение (этап 10, mock)", () => {
  it("цепочка родителей, принятие, отклонение с причиной и замена итоговой", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const h = (await b.getHomework(HW))!;
    const [v1, v2] = h.versions;
    expect(v1!.lineage.parentVersionId).toBeNull();
    expect(v2!.lineage).toMatchObject({ parentVersionId: v1!.id, origin: "claude_result", decision: "pending" });
    expect((await b.getVersionLineage(HW, v2!.id)).map((v) => v.id)).toEqual([v1!.id, v2!.id]);
    if (h.finalVersionId !== v2!.id) await b.markVersionFinal(v2!.id);
    const current = v2!.id;
    expect((await b.getHomework(HW))!.versions[1]!.lineage.decision).toBe("final");
    await expect(b.rejectVersion(current, "не подходит")).rejects.toMatchObject({ code: "blocked" });
    const other = current === v1!.id ? v2! : v1!;
    await expect(b.rejectVersion(other.id, "  ")).rejects.toBeTruthy();
    await b.rejectVersion(other.id, "Потеряны выводы");
    let fresh = (await b.getHomework(HW))!;
    expect(fresh.versions.find((v) => v.id === other.id)!.lineage).toMatchObject({ decision: "rejected", decisionReason: "Потеряны выводы" });
    await expect(b.markVersionFinal(other.id)).rejects.toMatchObject({ code: "blocked" });
    await b.acceptVersion(other.id);
    await b.markVersionFinal(other.id);
    await b.markVersionFinal(current);
    fresh = (await b.getHomework(HW))!;
    expect(fresh.versions.find((v) => v.id === current)!.lineage.decision).toBe("final");
    expect(fresh.versions.find((v) => v.id === other.id)!.lineage).toMatchObject({ decision: "superseded", supersededByVersionId: current });
  });

  it("сравнение содержимого честно требует ручной проверки для непрочитанных форматов", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const h = (await b.getHomework(HW))!;
    const [v1, v2] = h.versions;
    await expect(b.compareVersionContents(HW, v1!.id, v1!.id)).rejects.toMatchObject({ code: "conflict" });
    const c = await b.compareVersionContents(HW, v1!.id, v2!.id);
    expect(c.status).toBe("manual_required");
    expect(c.manualChecks.length).toBeGreaterThan(0);
    expect(c.warnings.some((w) => w.includes("Формулы не пересчитываются"))).toBe(true);
    expect(c.changes.some((x) => x.category === "file")).toBe(true);
  });

  it("план исправлений строится из текущей проверки и требований; предложения ждут подтверждения", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const h = (await b.getHomework(HW))!;
    await expect(b.buildCorrectionPlan(HW)).rejects.toMatchObject({ code: "blocked" });
    await b.markVersionFinal(h.versions[0]!.id);
    const plan = await b.buildCorrectionPlan(HW);
    const p = plan;
    expect(p.items.length).toBeGreaterThan(0);
    expect(p.items.every((i) => i.origin === "review" ? i.status === "proposed" : i.status === "open")).toBe(true);
    expect(p.stale).toBe(false);
    const proposed = p.items.find((i) => i.status === "proposed");
    expect(proposed).toBeTruthy();
    if (proposed) {
      await expect(b.setCorrectionItemStatus(proposed.id, "resolved", p.versionId)).rejects.toMatchObject({ code: "blocked" });
      const next = await b.setCorrectionItemStatus(proposed.id, "open");
      expect(next.items.find((i) => i.id === proposed.id)!.status).toBe("open");
      await expect(b.setCorrectionItemStatus(proposed.id, "resolved", p.versionId)).rejects.toMatchObject({ code: "conflict" });
      const resolved = await b.setCorrectionItemStatus(proposed.id, "resolved", h.versions[1]!.id);
      expect(resolved.items.find((i) => i.id === proposed.id)!.resolvedInVersionId).toBe(h.versions[1]!.id);
    }
    expect((await b.getCorrectionPlan(HW))!.id).toBe(p.id);
  });
});
