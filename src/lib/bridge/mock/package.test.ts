import { describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";

/** Этап 8: демо-комплект — неизменяемые ревизии и честные статусы. */
describe("ревизии комплекта (этап 8, mock)", () => {
  it("демо-комплект показывает отсутствующий файл и ручную проверку ссылки", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const pkg = (await b.getSubmissionPackage("hw-stat-7"))!;
    expect(pkg.revision).toBe(1);
    expect(pkg.items.find((i) => i.id === "notes")?.status).toBe("missing");
    expect(pkg.links[0]).toMatchObject({ status: "manual_required", verified: false });
    expect(pkg.checklist.find((c) => c.id === "inputs_fresh")?.status).toBe("verified");
    await expect(b.setPackageCheck(pkg.id, "attachments", true)).rejects.toMatchObject({ code: "blocked" });
    const links = await b.setPackageCheck(pkg.id, "links", true);
    expect(links.checklist.find((c) => c.id === "links")).toMatchObject({ manualConfirmed: true, done: true, status: "manual_required" });
    // Отсутствующий файл блокирует подтверждение даже после всех ручных отметок.
    await b.setPackageCheck(pkg.id, "manual_review", true);
    const confirmation = { submittedAt: new Date().toISOString(), assignmentUrl: "https://lms.example.ru/a/7", proof: null, userConfirmed: true as const };
    await expect(b.confirmSubmission("hw-stat-7", confirmation)).rejects.toMatchObject({ code: "blocked" });
  });

  it("история ревизий есть в агрегате", async () => {
    const b = new MockDesktopBridge({ latency: 0 });
    const wf = (await b.getWorkflow("hw-stat-7"))!;
    expect(wf.packageHistory.map((p) => [p.revision, p.stale])).toEqual([[1, false]]);
  });
});
