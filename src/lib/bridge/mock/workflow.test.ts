import { beforeEach, describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";
import type { ReviewRun, RunKind } from "../types";

/** Полный mock-процесс рабочего цикла ДЗ без задержек. */

let b: MockDesktopBridge;
const HW = "hw-linal-8";

async function settle(runId: string, homeworkId = HW): Promise<ReviewRun> {
  for (let i = 0; i < 200; i += 1) {
    const run = (await b.getReviewRuns(homeworkId)).find((r) => r.id === runId);
    if (run && run.status !== "queued" && run.status !== "running") return run;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("run did not finish");
}

async function run(kind: RunKind, versionId: string | null, homeworkId = HW): Promise<ReviewRun> {
  const bundle = await b.prepareSourceBundle(homeworkId, kind, versionId);
  const started =
    kind === "analyze_materials"
      ? await b.runMaterialAnalysis(homeworkId, bundle.id)
      : kind === "review_teacher"
        ? await b.runTeacherReview(homeworkId, versionId!, bundle.id)
        : await b.runVersionReview(homeworkId, versionId!, bundle.id);
  const done = await settle(started.id, homeworkId);
  expect(done.status).toBe("completed");
  expect(done.sourcesUsed.length).toBeGreaterThan(0);
  return done;
}

async function latestVersion(homeworkId = HW) {
  const h = (await b.getHomework(homeworkId))!;
  return h.versions[h.versions.length - 1]!;
}

async function stage(homeworkId = HW) {
  return (await b.getHomework(homeworkId))!.workflowStage;
}

beforeEach(async () => {
  b = new MockDesktopBridge({ latency: 0 });
  await b.connectProvider("codex"); // первая попытка в mock падает с ошибкой
  await b.connectProvider("codex");
  await b.connectProvider("claude");
});

describe("рабочий цикл ДЗ (mock)", () => {
  it("сохраняет роли и порог, обновляет ProviderStatus и применяет настройки к новым запускам", async () => {
    const rules = await b.getAutomationRules();
    rules.workflow.roles = { analyst: "claude", author: "codex", reviewer: "claude" };
    rules.workflow.styleThreshold = 7;
    const saved = await b.updateAutomationRules(rules);

    expect((await b.getAutomationRules()).workflow).toMatchObject({
      roles: { analyst: "claude", author: "codex", reviewer: "claude" },
      styleThreshold: 7,
    });
    expect((await b.getProviderStatus("codex")).roles).toEqual(["author"]);
    expect((await b.getProviderStatus("claude")).roles).toEqual(["analyst", "reviewer"]);

    const analysis = await run("analyze_materials", null);
    expect(analysis).toMatchObject({ role: "analyst", provider: "claude" });
    const [prompt] = await b.getPrompts(HW);
    await b.sendPromptToAuthor(prompt!.id);
    const version = await latestVersion();
    expect(version.author).toBe("codex");
    const review = await run("review_version", version.id);
    expect(review).toMatchObject({ role: "reviewer", provider: "claude" });
    expect(review.result?.style?.threshold).toBe(7);

    await b.undoOperation(saved.operation.id);
    expect((await b.getAutomationRules()).workflow.roles).toEqual({ analyst: "codex", author: "claude", reviewer: "codex" });
    expect((await b.getProviderStatus("codex")).roles).toEqual(["analyst", "reviewer"]);
  });

  it("отклоняет недопустимый порог стиля", async () => {
    const rules = await b.getAutomationRules();
    rules.workflow.styleThreshold = 11;
    await expect(b.updateAutomationRules(rules)).rejects.toMatchObject({ code: "conflict" });
  });

  it("проходит весь процесс от анализа материалов до неизменяемого снимка", async () => {
    // 1. Создание первого промта
    const analysis = await run("analyze_materials", null);
    expect(analysis.result?.prompt?.kind).toBe("initial");
    expect(await stage()).toBe("prompt_ready");
    const [initial] = await b.getPrompts(HW);
    expect(initial?.sources.length).toBeGreaterThan(0);

    // 2. Сохранение версии автором
    await b.sendPromptToAuthor(initial!.id);
    const v1 = await latestVersion();
    expect(v1.label).toBe("v01");
    expect(v1.author).toBe("claude");
    expect(v1.promptId).toBe(initial!.id);
    expect(await stage()).toBe("codex_review");
    const v1Snapshot = structuredClone(v1);

    // 3. Провал проверки
    const r1 = await run("review_version", v1.id);
    expect(r1.result?.verdict).toBe("needs_fixes");
    expect(r1.result?.critical).toHaveLength(2);
    expect(r1.result?.style?.score).toBe(6);
    expect(await stage()).toBe("needs_fixes");
    await b.saveRunAsIteration(r1.id);

    // 4. Промт исправлений
    const fix = await b.createRevisionPrompt(r1.id);
    expect(fix.value.kind).toBe("revision");
    expect(fix.value.addresses.length).toBeGreaterThanOrEqual(2);
    expect(fix.value.basedOnVersionId).toBe(v1.id);
    expect(await stage()).toBe("prompt_ready");

    // 5. Следующая версия; старая не изменилась
    await b.sendPromptToAuthor(fix.value.id);
    const v2 = await latestVersion();
    expect(v2.label).toBe("v02");
    expect((await b.getHomework(HW))!.versions[0]).toEqual(v1Snapshot);
    const r2 = await run("review_version", v2.id);
    expect(r2.result?.critical).toHaveLength(0);
    expect(r2.result?.remarks).toHaveLength(1);
    expect(r2.result?.style?.score).toBe(4);
    await b.saveRunAsIteration(r2.id);
    const cmp = await b.compareVersions(v1.id, v2.id);
    expect(cmp.fixed).toHaveLength(2);
    expect(cmp.style).toEqual({ from: 6, to: 4 });

    // 6. Комментарии преподавателя
    await b.saveTeacherComments(HW, [
      { source: "chat", date: new Date().toISOString(), question: "Нужна проверка ответа?", answer: "Да, подстановкой." },
      { source: "chat", date: new Date().toISOString(), question: "Оформлять в LaTeX?", answer: "Да, формулы в LaTeX." },
      { source: "chat", date: new Date().toISOString(), question: "Нужен ли вывод?", answer: "Добавьте вывод в конце каждой задачи." },
    ]);
    const t2 = await run("review_teacher", v2.id);
    const reqs = await b.getTeacherRequirements(HW);
    expect(reqs).toHaveLength(3);
    expect(reqs.filter((r) => r.status === "not_done")).toHaveLength(1);
    expect(reqs.every((r) => r.commentId && r.origin)).toBe(true);
    expect(await stage()).toBe("needs_fixes");

    // 7. Готовность блокируется невыполненным требованием
    await b.markVersionFinal(v2.id);
    const blocked = await b.getReadiness(HW);
    expect(blocked.ready).toBe(false);
    expect(blocked.checks.find((c) => c.id === "teacher")?.ok).toBe(false);
    await expect(b.markReadyToSubmit(HW, { noTeacherComments: false })).rejects.toMatchObject({ code: "blocked" });
    await expect(b.buildSubmissionPackage(HW)).rejects.toMatchObject({ code: "blocked" });

    // 8. Исправление по комментариям → v03 → все проверки
    const teacherFix = await b.createRevisionPrompt(t2.id);
    expect(teacherFix.value.kind).toBe("teacher");
    await b.sendPromptToAuthor(teacherFix.value.id);
    const v3 = await latestVersion();
    expect(v3.label).toBe("v03");
    const r3 = await run("review_version", v3.id);
    await b.saveRunAsIteration(r3.id);
    expect(r3.result?.style?.score).toBe(3);
    await run("review_teacher", v3.id);
    expect((await b.getTeacherRequirements(HW)).every((r) => r.status === "done")).toBe(true);
    await b.markVersionFinal(v3.id);
    const ready = await b.getReadiness(HW);
    expect(ready.ready).toBe(true);
    await b.markReadyToSubmit(HW, { noTeacherComments: false });
    expect(await stage()).toBe("ready_to_submit");

    // 9. Сборка комплекта
    const pkg = (await b.buildSubmissionPackage(HW)).value;
    expect(pkg.items.find((i) => i.role === "main")?.finalName).toMatch(/^Гончарова_Доминика_Неделя8 ДЗ\./);
    expect(pkg.uploadList.length).toBeGreaterThan(0);

    // 10. Ручное подтверждение отправки
    const confirmation = { submittedAt: new Date().toISOString(), assignmentUrl: "https://lms.example.ru/a/8", proof: { name: "screen.png", sizeBytes: 1000 }, userConfirmed: true as const };
    await expect(b.confirmSubmission(HW, confirmation)).rejects.toMatchObject({ code: "blocked" });
    for (const c of pkg.checklist.filter((x) => !x.auto)) await b.setPackageCheck(pkg.id, c.id, true);
    const receipt = (await b.confirmSubmission(HW, confirmation)).value;
    expect(receipt.versionId).toBe(v3.id);
    expect(receipt.proof?.name).toBe("screen.png");
    expect(receipt.snapshotPath).toMatch(/04 Отправлено$/);
    expect(await stage()).toBe("submitted");

    // 11. Отправленный снимок нельзя изменить
    const imm = { code: "immutable" };
    await expect(b.createHomeworkVersion(HW, "x")).rejects.toMatchObject(imm);
    await expect(b.markVersionFinal(v2.id)).rejects.toMatchObject(imm);
    await expect(b.saveTeacherComments(HW, [{ source: "chat", date: "", question: "q", answer: "a" }])).rejects.toMatchObject(imm);
    await expect(b.buildSubmissionPackage(HW)).rejects.toMatchObject(imm);
    await expect(b.setPackageCheck(pkg.id, "manual_review", false)).rejects.toMatchObject(imm);
    await expect(b.confirmSubmission(HW, confirmation)).rejects.toMatchObject(imm);
    const finalOp = (await b.getActivityLog()).find((o) => o.kind === "mark_final" && o.homeworkId === HW && o.undoable);
    await expect(b.undoOperation(finalOp!.id)).rejects.toMatchObject(imm);
    expect((await b.getSubmissionReceipt(HW))?.snapshotHash).toBe(receipt.snapshotHash);
  });

  it("не запускает анализ без обязательных материалов", async () => {
    const bundle = await b.prepareSourceBundle("hw-matanadv-8", "analyze_materials");
    expect(bundle.complete).toBe(false);
    expect(bundle.missing.some((m) => m.kind === "task" && m.required)).toBe(true);
    await expect(b.runMaterialAnalysis("hw-matanadv-8", bundle.id)).rejects.toMatchObject({ code: "blocked" });
  });

  it("исключение источника пересчитывает недостающие материалы", async () => {
    const bundle = await b.prepareSourceBundle(HW, "analyze_materials");
    const style = bundle.items.find((i) => i.kind === "style")!;
    const updated = await b.setSourceIncluded(bundle.id, style.id, false);
    expect(updated.complete).toBe(false);
    expect(updated.missing.some((m) => m.kind === "style")).toBe(true);
  });

  it("запуск можно отменить, этап возвращается", async () => {
    const before = await stage();
    const bundle = await b.prepareSourceBundle(HW, "analyze_materials");
    const started = await b.runMaterialAnalysis(HW, bundle.id);
    const cancelled = await b.cancelRun(started.id);
    expect(cancelled.status).toBe("cancelled");
    expect(await stage()).toBe(before);
  });

  it("без подключённого провайдера запуск отклоняется", async () => {
    await b.disconnectProvider("codex");
    const bundle = await b.prepareSourceBundle(HW, "analyze_materials");
    await expect(b.runMaterialAnalysis(HW, bundle.id)).rejects.toMatchObject({ code: "provider_unavailable" });
  });

  it("сид сценария: v01 → ошибки → v02 → комментарии, одно требование не выполнено", async () => {
    const its = await b.getIterations("hw-oba-7");
    expect(its.map((i) => i.label)).toEqual(["v01", "v02"]);
    expect(its[0]?.status).toBe("failed");
    expect(its[0]?.review?.result?.critical).toHaveLength(2);
    expect(its[0]?.review?.result?.style?.score).toBe(6);
    expect(its[1]?.fixedFindings).toHaveLength(2);
    expect(its[1]?.review?.result?.style?.score).toBe(4);
    expect((await b.getTeacherComments("hw-oba-7"))).toHaveLength(3);
    expect((await b.getTeacherRequirements("hw-oba-7")).filter((r) => r.status === "not_done")).toHaveLength(1);
    expect(await stage("hw-oba-7")).toBe("prompt_ready");
  });
});
