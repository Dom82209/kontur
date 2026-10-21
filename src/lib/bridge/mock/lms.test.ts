import { describe, expect, it } from "vitest";
import { lmsUrlProblem } from "../../lmsLinks";
import { newOperationId } from "../intake";
import { MockDesktopBridge } from "./MockDesktopBridge";

describe("mock: экспорт ЦУ и watcher этапа 6", () => {
  it("watcher выключен по умолчанию и не включается без папки загрузок", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    await expect(bridge.getDownloadsWatcher()).resolves.toMatchObject({ enabled: false, state: "disabled", candidates: [] });
    await expect(bridge.setDownloadsWatcher(true)).rejects.toMatchObject({ code: "blocked" });
    await bridge.chooseManagedRoot("downloads");
    await expect(bridge.setDownloadsWatcher(true)).resolves.toMatchObject({ enabled: true, state: "watching" });
    await expect(bridge.setDownloadsWatcher(false)).resolves.toMatchObject({ enabled: false, state: "disabled" });
  });

  it("предпросмотр → импорт: фазы прогресса, файлы с происхождением, ничего не разложено", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const candidate = (await bridge.pickCuExport())!;
    const preview = await bridge.previewCuExport(candidate.id);
    expect(preview.entries.some((e) => !e.selectable)).toBe(true);
    const operationId = newOperationId();
    const phases: string[] = [];
    bridge.on("cu-import:progress", (p) => p.operationId === operationId && phases.push(p.phase));
    const selected = preview.entries.filter((e) => e.selected);
    const result = await bridge.importCuExport(
      {
        candidateId: candidate.id,
        archiveSha256: preview.archiveSha256,
        entries: selected.map((e) => ({ entryId: e.entryId, subjectId: e.classification.subjectId, weekNumber: e.classification.weekNumber, category: e.classification.category, edited: false })),
        courseUrl: "https://my.centraluniversity.ru/learn/courses/view/actual/oba",
      },
      operationId,
    );
    expect(result.value.imported).toBe(selected.length);
    expect(result.value.skipped).toBe(preview.entries.length - selected.length);
    expect(phases[0]).toBe("inspect");
    expect(phases).toContain("extract");
    expect(phases.at(-1)).toBe("cleanup");
    const inbox = await bridge.getInboxFiles();
    const imported = inbox.filter((f) => f.origin === "lms" && f.provenance?.archiveName === candidate.name);
    expect(imported).toHaveLength(selected.length);
    expect(imported.every((f) => f.status !== "routed" && f.provenance?.source === "cu_export")).toBe(true);
    const fileId = imported[0]!.id;
    await expect(bridge.openCourse({ fileId })).resolves.toBeUndefined();
    await expect(bridge.openAssignment({ fileId })).rejects.toMatchObject({ code: "not_found" });

    const replay = await bridge.importCuExport({ candidateId: candidate.id, archiveSha256: preview.archiveSha256, entries: [] }, operationId);
    expect(replay.value.replayed).toBe(true);
  });

  it("изменённый отпечаток и чужой домен отклоняются", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const candidate = (await bridge.pickCuExport())!;
    const preview = await bridge.previewCuExport(candidate.id);
    const entries = [{ entryId: preview.entries[0]!.entryId, subjectId: null, weekNumber: null, category: "unknown" as const, edited: false }];
    await expect(bridge.importCuExport({ candidateId: candidate.id, archiveSha256: "f".repeat(64), entries })).rejects.toMatchObject({ code: "conflict" });
    await expect(bridge.importCuExport({ candidateId: candidate.id, archiveSha256: preview.archiveSha256, entries, assignmentUrl: "https://evil.example/x" })).rejects.toMatchObject({ code: "blocked" });
    await expect(bridge.setSubjectCourseUrl("oba", "http://my.centraluniversity.ru/x")).rejects.toMatchObject({ code: "blocked" });
  });
});

describe("подсказка allowlist ссылок LMS", () => {
  it("пропускает только https://my.centraluniversity.ru", () => {
    expect(lmsUrlProblem("https://my.centraluniversity.ru/learn/courses/1")).toBeNull();
    expect(lmsUrlProblem("")).toBeNull();
    for (const bad of ["http://my.centraluniversity.ru/x", "https://evil.example", "https://my.centraluniversity.ru.evil.example/", "https://u:p@my.centraluniversity.ru/", "https://my.centraluniversity.ru:8443/", "javascript:alert(1)", "not a url"]) {
      expect(lmsUrlProblem(bad), bad).not.toBeNull();
    }
  });
});
