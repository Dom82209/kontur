import { describe, expect, it } from "vitest";
import { compareDeadlines, formatDeadline, formatWeekRange, UNKNOWN_DEADLINE } from "./format";

describe("неизвестные сроки и даты недели", () => {
  it("не выдумывает срок, если он не задан", () => {
    expect(formatDeadline(null)).toEqual({ text: UNKNOWN_DEADLINE, tone: "unknown" });
  });

  it("ставит ДЗ без срока в конец и сохраняет порядок известных", () => {
    const items = [null, "2026-11-02T20:59:00Z", null, "2026-10-30T20:59:00Z"];
    expect([...items].sort(compareDeadlines)).toEqual(["2026-10-30T20:59:00Z", "2026-11-02T20:59:00Z", null, null]);
  });

  it("показывает честную подпись вместо пустых дат недели", () => {
    expect(formatWeekRange(null, null)).toBe("даты недели не заданы");
    expect(formatWeekRange("2026-10-26T00:00:00Z", null)).toBe("даты недели не заданы");
    expect(formatWeekRange("2026-10-26T00:00:00Z", "2026-11-01T23:59:00Z")).toContain("—");
  });
});
