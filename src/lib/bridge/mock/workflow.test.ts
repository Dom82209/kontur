import { beforeEach, describe, expect, it } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";
import { TASK_FOR_RUN, type AiConfirmInput, type AiJob, type ManualAnalysisInput, type ManualReviewInput, type ReviewRun, type RunKind } from "../types";

/**
 * Полный mock-процесс рабочего цикла ДЗ без задержек. Этап 9: каждый AI-шаг —
 * задание передачи: подготовка (полный текст) → явная отправка в «CLI» (демо) →
 * ответ → подтверждение пользователем. Без подтверждения ничего не меняется.
 */

let b: MockDesktopBridge;
const HW = "hw-linal-8";

async function settleJob(jobId: string): Promise<AiJob> {
  for (let i = 0; i < 400; i += 1) {
    const job = await b.getAiJob(jobId);
    if (job.status !== "running") return job;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("job did not finish");
}

async function run(kind: RunKind, versionId: string | null, homeworkId = HW): Promise<ReviewRun> {
  const bundle = await b.prepareSourceBundle(homeworkId, kind, versionId);
  const started =
    kind === "analyze_materials"
      ? await b.runMaterialAnalysis(homeworkId, bundle.id)
      : kind === "review_teacher"
        ? await b.runTeacherReview(homeworkId, versionId!, bundle.id)
        : await b.runVersionReview(homeworkId, versionId!, bundle.id);
  expect(started.status).toBe("waiting_for_provider");
  const task = TASK_FOR_RUN[kind];
  const job = await b.prepareAiJob(started.id, task);
  expect(job.transport).toBe("cli");
  expect(job.promptText).toContain("[S1]");
  await b.sendAiJob(job.id);
  const ready = await settleJob(job.id);
  expect(ready.status).toBe("response_ready");
  // Ответ получен, но запуск ещё не завершён — нужно подтверждение.
  expect((await b.getReviewRuns(homeworkId)).find((r) => r.id === started.id)?.status).toBe("waiting_for_provider");
  const input: AiConfirmInput = kind === "analyze_materials" ? { kind: "analysis", value: ready.draft as ManualAnalysisInput } : { kind: "review", value: ready.draft as ManualReviewInput };
  const done = (await b.confirmAiResult(job.id, input)).value.run;
  expect(done.status).toBe("completed");
  expect(done.provenance).toBe("provider");
  expect(done.sourcesUsed.length).toBeGreaterThan(0);
  return done;
}

/** Решение автор создаёт сам; пользователь добавляет его как новую версию. */
async function addVersion(promptId: string, homeworkId = HW) {
  await b.sendPromptToAuthor(promptId);
  const author = (await b.getAutomationRules()).workflow.roles.author;
  const [file] = await b.getHomeworkCandidates(homeworkId);
  return (await b.createHomeworkVersion(homeworkId, file!.id, { author, promptId })).value;
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
    const original = await b.getAutomationRules();
    const rules = structuredClone(original);
    rules.workflow.roles = { analyst: "claude", author: "codex", reviewer: "claude" };
    rules.workflow.styleThreshold = 7;
    const saved = await b.updateAutomationRules(rules);
    // Как в backend: сохранение правил не отменяется через журнал.
    expect(saved.operation.undoable).toBe(false);

    expect((await b.getAutomationRules()).workflow).toMatchObject({
      roles: { analyst: "claude", author: "codex", reviewer: "claude" },
      styleThreshold: 7,
    });
    expect((await b.getProviderStatus("codex")).roles).toEqual(["author"]);
    expect((await b.getProviderStatus("claude")).roles).toEqual(["analyst", "reviewer"]);

    rules.workflow.tasks = { material_analysis: "claude", initial_prompt: "claude", solution_review: "claude", revision_prompt: "claude", teacher_review: "claude" };
    await b.updateAutomationRules(rules);
    expect((await b.getProviderStatus("claude")).tasks).toHaveLength(5);
    const analysis = await run("analyze_materials", null);
    expect(analysis).toMatchObject({ role: "analyst", provider: "claude" });
    const [prompt] = await b.getPrompts(HW);
    expect(prompt).toMatchObject({ provenance: "provider", createdBy: "claude", status: "draft" });
    await b.commitPrompt(prompt!.id);
    const version = await addVersion(prompt!.id);
    expect(version.author).toBe("codex");
    const review = await run("review_version", version.id);
    expect(review).toMatchObject({ role: "reviewer", provider: "claude" });
    expect(review.result?.style?.threshold).toBe(7);

    await b.updateAutomationRules(original);
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
    expect(await stage()).toBe("codex_analysis");
    const [initial] = await b.getPrompts(HW);
    expect(initial?.sources.length).toBeGreaterThan(0);
    await b.commitPrompt(initial!.id);
    expect(await stage()).toBe("prompt_ready");

    // 2. Решение автора — новая версия (автор работает в своём приложении)
    const v1 = await addVersion(initial!.id);
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
    expect(fix.value.provenance).toBe("manual");
    expect(await stage()).toBe("needs_fixes");
    await b.commitPrompt(fix.value.id);

    // 5. Следующая версия; старая не изменилась
    const v2 = await addVersion(fix.value.id);
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
      { source: "chat", date: new Date().toISOString(), question: "Нужна проверка ответа?", answer: "Да, подстановкой.", authority: "teacher_official", author: "Преподаватель", confirmedOfficial: true },
      { source: "chat", date: new Date().toISOString(), question: "Оформлять в LaTeX?", answer: "Да, формулы в LaTeX.", authority: "teacher_official", author: "Преподаватель", confirmedOfficial: true },
      { source: "chat", date: new Date().toISOString(), question: "Нужен ли вывод?", answer: "Добавьте вывод в конце каждой задачи.", authority: "teacher_official", author: "Преподаватель", confirmedOfficial: true },
    ]);
    // Требования из ответов преподавателя пользователь фиксирует сам.
    const comments = await b.getTeacherComments(HW);
    for (const [i, c] of comments.entries()) {
      await b.addTeacherRequirement(HW, { commentId: c.id, text: c.answer, origin: c.answer, status: i === 2 ? "not_done" : "done" });
    }
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
    await b.commitPrompt(teacherFix.value.id);
    const v3 = await addVersion(teacherFix.value.id);
    expect(v3.label).toBe("v03");
    const r3 = await run("review_version", v3.id);
    await b.saveRunAsIteration(r3.id);
    expect(r3.result?.style?.score).toBe(3);
    for (const r of (await b.getTeacherRequirements(HW)).filter((x) => x.status === "not_done")) await b.resolveRequirement(r.id, "done", "Исправлено в v03");
    await run("review_teacher", v3.id);
    expect((await b.getTeacherRequirements(HW)).every((r) => r.status === "done")).toBe(true);
    await b.markVersionFinal(v3.id);
    // Этап 9: стиль — локальной проверкой; без профиля — ручное подтверждение.
    const styleCheck = await b.runStyleCheck(HW);
    if (!styleCheck.passes) await b.confirmStyleCheck(styleCheck.id, "проверено вручную");
    const ready = await b.getReadiness(HW);
    expect(ready.checks.filter((c) => !c.ok)).toEqual([]);
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
    for (const c of pkg.checklist.filter((x) => !x.auto && x.status === "manual_required")) await b.setPackageCheck(pkg.id, c.id, true);
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

  it("отмена запуска останавливает выполняющееся задание без результата", async () => {
    const bundle = await b.prepareSourceBundle(HW, "analyze_materials");
    const started = await b.runMaterialAnalysis(HW, bundle.id);
    const job = await b.prepareAiJob(started.id, "material_analysis");
    await b.sendAiJob(job.id);
    const cancelled = await b.cancelRun(started.id);
    expect(cancelled.status).toBe("cancelled");
    expect(cancelled.result).toBeNull();
    const stopped = await settleJob(job.id);
    expect(stopped.status).toBe("cancelled");
    expect(stopped.responseText).toBeNull();
  });

  it("без подключённого CLI — ручная передача, CLI отклоняется честно", async () => {
    await b.disconnectProvider("codex");
    const bundle = await b.prepareSourceBundle(HW, "analyze_materials");
    const started = await b.runMaterialAnalysis(HW, bundle.id);
    await expect(b.prepareAiJob(started.id, "material_analysis", "cli")).rejects.toMatchObject({ code: "provider_unavailable" });
    const job = await b.prepareAiJob(started.id, "material_analysis");
    expect(job.transport).toBe("manual_handoff");
    await expect(b.sendAiJob(job.id)).rejects.toMatchObject({ code: "blocked" });
    const pasted = await b.submitAiResponse(job.id, "Готово:\n```json\n{\"summary\": \"Итог\", \"promptText\": \"Промт\"}\n```");
    expect(pasted).toMatchObject({ status: "response_ready", parseStatus: "valid", responseSource: "paste" });
    const corrupt = await b.submitAiResponse(job.id, "не могу");
    expect(corrupt.parseStatus).toBe("corrupt");
    await expect(b.importAiResponseFile(job.id)).rejects.toMatchObject({ code: "backend_unavailable" });
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
