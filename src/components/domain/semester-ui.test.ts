import { describe, expect, it } from "vitest";
import { formatUtcOffset, UTC_OFFSETS } from "./SemesterPanel";
import { fromLocalInput, toLocalInput } from "./DeadlineEditor";

describe("UI семестра и сроков", () => {
  it("форматирует смещения часового пояса", () => {
    expect(formatUtcOffset(180)).toBe("UTC+03:00");
    expect(formatUtcOffset(-300)).toBe("UTC−05:00");
    expect(formatUtcOffset(345)).toBe("UTC+05:45");
    expect(UTC_OFFSETS[0]).toBe(-720);
    expect(UTC_OFFSETS[UTC_OFFSETS.length - 1]).toBe(840);
  });

  it("datetime-local и ISO взаимно обратимы, пустое значение — срок неизвестен", () => {
    expect(fromLocalInput("")).toBeNull();
    const iso = fromLocalInput("2026-12-31T23:59")!;
    expect(toLocalInput(iso)).toBe("2026-12-31T23:59");
    expect(toLocalInput(null)).toBe("");
  });
});
