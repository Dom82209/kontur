import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

describe("Mock: семестр и сроки (этап 5)", () => {
  it("сохранение семестра пересчитывает даты недель и требует ревизию", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const current = await bridge.getSemester();
    expect(current).not.toBeNull();
    const result = await bridge.setSemester({ startDate: "2026-09-01", utcOffsetMinutes: 180, weekCount: 18 }, current!.revision);
    expect(result.semester.weeks[0]).toEqual({ number: 1, startsAt: "2026-08-31T21:00:00Z", endsAt: "2026-09-07T20:59:59Z" });
    expect(result.semester.weeks[17]!.endsAt).toBe("2027-01-04T20:59:59Z");
    expect(result.semester.warnings[0]).toContain("не с понедельника");
    expect(result.updatedWeeks).toBeGreaterThan(0);
    const week = await bridge.getWeek("stat", 2);
    expect(week).toMatchObject({ startsAt: "2026-09-07T21:00:00Z", datesSource: "semester" });
    await expect(bridge.setSemester({ startDate: "2026-09-01", utcOffsetMinutes: 180, weekCount: 18 }, current!.revision)).rejects.toMatchObject({ code: "conflict" });
    await expect(bridge.setSemester({ startDate: "01.09.2026", utcOffsetMinutes: 180, weekCount: 18 }, result.semester.revision)).rejects.toMatchObject({ code: "blocked" });
  });

  it("срок ДЗ задаётся точно или становится неизвестным; сданное ДЗ неизменяемо", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const list = await bridge.getHomeworkList();
    const open = list.find((h) => h.stage !== "submitted")!;
    const set = await bridge.setHomeworkDeadline(open.id, open.revision, "2026-11-01T23:59:00+03:00");
    expect(set).toMatchObject({ deadline: "2026-11-01T20:59:00Z", deadlineSource: "manual" });
    await expect(bridge.setHomeworkDeadline(open.id, open.revision, null)).rejects.toMatchObject({ code: "conflict" });
    const cleared = await bridge.setHomeworkDeadline(open.id, set.revision, null);
    expect(cleared).toMatchObject({ deadline: null, deadlineSource: null });
    const submitted = list.find((h) => h.stage === "submitted");
    if (submitted) await expect(bridge.setHomeworkDeadline(submitted.id, submitted.revision, null)).rejects.toMatchObject({ code: "blocked" });
  });
});
