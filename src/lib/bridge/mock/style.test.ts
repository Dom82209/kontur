import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

/** Этап 9: mock повторяет правила профиля личного стиля. */
describe("профиль личного стиля (этап 9, mock)", () => {
  it("источники — только явный выбор; ревизии неизменяемы и устаревают при изменении набора", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const empty = await b.getStyleProfileState();
    expect(empty.sources).toEqual([]);
    await expect(b.addStyleSource("/etc/passwd", "previous_homework", "ru")).rejects.toMatchObject({ code: "conflict" });
    await expect(b.addStyleSource("Стиль/ДЗ.pdf", "style_markdown", "ru")).rejects.toMatchObject({ code: "conflict" });
    await b.addStyleSource("Стиль/ДЗ1.md", "previous_homework", "ru");
    const one = await b.rebuildStyleProfile("ru");
    expect(one.current).toMatchObject({ revision: 1, status: "insufficient_sources", content: null });
    await b.addStyleSource("Стиль/ДЗ2.md", "confirmed_sample", "ru");
    expect((await b.getStyleProfileState()).stale).toBe(true);
    const two = await b.rebuildStyleProfile("ru");
    expect(two.current).toMatchObject({ revision: 2, status: "ready" });
    expect(two.history.map((h) => h.revision)).toEqual([2, 1]);
    expect((await b.rebuildStyleProfile("ru")).current?.revision).toBe(2);
    const removed = await b.removeStyleSource(two.sources[0]!.id);
    expect(removed.stale).toBe(true);
  });

  it("без профиля — insufficient_sources и ручное подтверждение; изменение профиля делает проверку устаревшей", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const check = await b.runStyleCheck("hw-stat-7");
    expect(check).toMatchObject({ status: "insufficient_sources", score: null, passes: false });
    const confirmed = await b.confirmStyleCheck(check.id, "сверила вручную");
    expect(confirmed.passes).toBe(true);
    await b.addStyleSource("Стиль/ДЗ1.md", "previous_homework", "ru");
    await b.addStyleSource("Стиль/ДЗ2.md", "previous_homework", "ru");
    await b.rebuildStyleProfile("ru");
    const wf = (await b.getWorkflow("hw-stat-7"))!;
    expect(wf.styleCheck?.status).toBe("stale");
    expect(wf.readiness.checks.find((c) => c.id === "style")?.ok).toBe(false);
    const fresh = await b.runStyleCheck("hw-stat-7");
    expect(fresh.status).toBe("checked");
    expect(fresh.score).not.toBeNull();
    expect(fresh.body.explanation).toContain("не доказывает");
  });
});
