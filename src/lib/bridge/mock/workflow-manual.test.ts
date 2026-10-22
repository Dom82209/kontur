import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";
import { BridgeError } from "../types";
import { isStaleConflict } from "../../workflow-ui";

/**
 * Этап 8: mock поддерживает тот же контракт рабочего цикла, что и Tauri —
 * агрегат getWorkflow, ревизия, ручной режим, фиксация промта, conflict.
 */

const HW = "hw-linal-8";
let b: MockDesktopBridge;
let consoleError: ReturnType<typeof vi.spyOn>;

const revision = async () => (await b.getWorkflow(HW))!.revision;

beforeEach(async () => {
  b = new MockDesktopBridge({ latency: 0 });
  // Mock-автор (демонстрационный адаптер) создаёт версию по переданному промту.
  await b.connectProvider("codex").catch(() => undefined); // первая попытка в mock падает
  await b.connectProvider("codex");
  await b.connectProvider("claude");
  consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
});

afterEach(() => {
  expect(consoleError).not.toHaveBeenCalled();
  consoleError.mockRestore();
});

async function manualAnalysis() {
  const bundle = await b.prepareSourceBundle(HW, "analyze_materials", null);
  return b.runMaterialAnalysis(HW, bundle.id, { mode: "manual" });
}

describe("контракт рабочего цикла (mock, этап 8)", () => {
  it("getWorkflow отдаёт полный агрегат, а для неизвестного ДЗ — null", async () => {
    const workflow = await b.getWorkflow(HW);
    expect(Object.keys(workflow!).sort()).toEqual(
      [
        "bundles",
        "homework",
        "iterations",
        "prompts",
        "providerMode",
        "readiness",
        "revision",
        "runs",
        "stage",
        "styleConfirmations",
        "submissionPackage",
        "submissionReceipt",
        "teacherComments",
        "teacherRequirements",
        "transitions",
      ].sort(),
    );
    expect(workflow!.stage).toBe(workflow!.homework.workflowStage);
    expect(workflow!.readiness.revision).toBe(workflow!.revision);
    expect(await b.getWorkflow("missing")).toBeNull();
  });

  it("ручной запуск ждёт результат и не выдаёт ложный успех AI", async () => {
    const completed = vi.fn();
    const off = b.on("review:completed", completed);
    const run = await manualAnalysis();
    expect(run).toMatchObject({ status: "manual_handoff", provenance: "manual", result: null });
    expect(completed).not.toHaveBeenCalled();
    expect((await b.getWorkflow(HW))!.stage).toBe("codex_analysis");

    const saved = await b.saveManualAnalysis(run.id, { summary: "Разобрал условие и лекции", promptText: "Реши задачу 1 по лекции 8" }, { expectedRevision: await revision() });
    expect(saved.value).toMatchObject({ status: "completed", provenance: "manual" });
    expect(completed).toHaveBeenCalledOnce();
    off();

    const [prompt] = await b.getPrompts(HW);
    expect(prompt).toMatchObject({ status: "draft", provenance: "manual", committedAt: null });
    // Черновик правится, зафиксированный — неизменяем.
    await b.updatePromptText(prompt!.id, "Реши задачу 1 строго по лекции 8");
    const committed = await b.commitPrompt(prompt!.id, { expectedRevision: await revision() });
    expect(committed.value.status).toBe("committed");
    expect((await b.getWorkflow(HW))!.stage).toBe("prompt_ready");
    await expect(b.updatePromptText(prompt!.id, "другое")).rejects.toMatchObject({ code: "immutable" });
    await expect(b.commitPrompt(prompt!.id)).rejects.toMatchObject({ code: "immutable" });
  });

  it("новая версия, ручная проверка и промт исправлений", async () => {
    const run = await manualAnalysis();
    await b.saveManualAnalysis(run.id, { summary: "итог", promptText: "промт" });
    const [prompt] = await b.getPrompts(HW);
    await b.commitPrompt(prompt!.id);
    await b.sendPromptToAuthor(prompt!.id);

    const workflow = (await b.getWorkflow(HW))!;
    expect(workflow.stage).toBe("codex_review");
    const version = workflow.homework.versions.at(-1)!;
    const bundle = await b.prepareSourceBundle(HW, "review_version", version.id);
    const review = await b.runVersionReview(HW, version.id, bundle.id, { mode: "manual" });
    expect(review.status).toBe("manual_handoff");

    // «Пройдено» с критической ошибкой — отклоняется.
    await expect(
      b.saveManualReviewResult(review.id, { verdict: "pass", summary: "ок", correctness: "fail", taskFit: "pass", courseGrounding: "pass", teacher: "na", styleScore: 2, critical: [{ criterion: "correctness", text: "ошибка", location: null }] }),
    ).rejects.toMatchObject({ code: "blocked" });

    const saved = await b.saveManualReviewResult(review.id, {
      verdict: "needs_fixes",
      summary: "Есть ошибка в знаке",
      correctness: "fail",
      taskFit: "pass",
      courseGrounding: "pass",
      teacher: "na",
      styleScore: 3,
      critical: [{ criterion: "correctness", text: "Неверный знак", location: "задача 1" }],
    });
    expect(saved.value.assessment).not.toBeNull();
    expect(saved.value.result?.verdict).toBe("needs_fixes");
    expect((await b.getWorkflow(HW))!.stage).toBe("needs_fixes");

    const created = vi.fn();
    const off = b.on("homework:revision-prompt-created", created);
    const fix = await b.createRevisionPrompt(review.id);
    off();
    expect(fix.value.kind).toBe("revision");
    expect(fix.value.basedOnVersionId).toBe(version.id);
    expect(created).toHaveBeenCalledWith(expect.objectContaining({ homeworkId: HW, promptId: fix.value.id, kind: "revision" }));
  });

  it("стиль выше порога блокирует готовность до подтверждения пользователем", async () => {
    const run = await manualAnalysis();
    await b.saveManualAnalysis(run.id, { summary: "итог", promptText: "промт" });
    const [prompt] = await b.getPrompts(HW);
    await b.commitPrompt(prompt!.id);
    await b.sendPromptToAuthor(prompt!.id);
    const version = (await b.getWorkflow(HW))!.homework.versions.at(-1)!;
    const bundle = await b.prepareSourceBundle(HW, "review_version", version.id);
    const review = await b.runVersionReview(HW, version.id, bundle.id, { mode: "manual" });
    await b.saveManualReviewResult(review.id, { verdict: "pass", summary: "Верно", correctness: "pass", taskFit: "pass", courseGrounding: "pass", teacher: "na", styleScore: 8 });
    await b.saveRunAsIteration(review.id); // готовность считается по закреплённой проверке

    const blocked = (await b.getReadiness(HW)).checks.find((c) => c.id === "style");
    expect(blocked?.ok).toBe(false);
    const confirmation = await b.confirmStyleDivergence(review.id, { expectedRevision: await revision() });
    expect(confirmation.value).toMatchObject({ score: 8, threshold: 4, confirmedBy: "user" });
    expect((await b.getReadiness(HW)).checks.find((c) => c.id === "style")?.ok).toBe(true);
    await expect(b.confirmStyleDivergence(review.id)).rejects.toMatchObject({ code: "conflict" });
  });

  it("устаревшая ревизия даёт conflict; после перечитывания действие проходит", async () => {
    const before = await revision();
    const updated = vi.fn();
    const off = b.on("homework:updated", updated);
    await manualAnalysis();
    expect(updated).toHaveBeenCalled();
    expect(updated.mock.calls.at(-1)![0]).toMatchObject({ homeworkId: HW, revision: await revision() });
    off();

    const error = await b.transitionWorkflow(HW, "materials_collected", { expectedRevision: before }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(BridgeError);
    expect(isStaleConflict(error)).toBe(true);

    const fresh = await b.getWorkflow(HW);
    // Открытый запуск отменяем, затем возвращаемся к сбору материалов.
    const open = fresh!.runs.find((r) => r.status === "manual_handoff")!;
    await b.cancelRun(open.id, { expectedRevision: fresh!.revision });
    const moved = await b.transitionWorkflow(HW, "materials_collected", { expectedRevision: await revision() });
    expect(moved.value.workflowStage).toBe("materials_collected");
  });

  it("переходы идут только по таблице state machine", async () => {
    const workflow = (await b.getWorkflow(HW))!;
    expect(workflow.transitions.every((t) => typeof t.allowed === "boolean" && t.reason.length > 0)).toBe(true);
    await expect(b.transitionWorkflow(HW, "submitted")).rejects.toMatchObject({ code: "blocked" });
    await expect(b.transitionWorkflow(HW, "ready_to_submit")).rejects.toMatchObject({ code: "blocked" });
    await expect(b.transitionWorkflow(HW, "teacher_review")).rejects.toMatchObject({ code: "conflict" });
  });

  it("без подключённого автора промт передаётся вручную, а версию добавляет пользователь", async () => {
    const offline = new MockDesktopBridge({ latency: 0 });
    const bundle = await offline.prepareSourceBundle(HW, "analyze_materials", null);
    const run = await offline.runMaterialAnalysis(HW, bundle.id, { mode: "manual" });
    await offline.saveManualAnalysis(run.id, { summary: "итог", promptText: "промт" });
    const [prompt] = await offline.getPrompts(HW);
    await expect(offline.sendPromptToAuthor(prompt!.id)).rejects.toMatchObject({ code: "blocked" });
    await offline.commitPrompt(prompt!.id);
    const versionsBefore = (await offline.getWorkflow(HW))!.homework.versions.length;
    const sent = await offline.sendPromptToAuthor(prompt!.id);
    expect(sent.value.sentAt).not.toBeNull();
    const after = (await offline.getWorkflow(HW))!;
    expect(after.stage).toBe("claude_solving");
    expect(after.homework.versions).toHaveLength(versionsBefore);

    const [file] = await offline.getHomeworkCandidates(HW);
    const version = await offline.createHomeworkVersion(HW, file!.id, { promptId: prompt!.id, expectedRevision: after.revision });
    expect(version.value.author).toBe("user");
    expect((await offline.getWorkflow(HW))!.stage).toBe("codex_review");
    await expect(offline.createHomeworkVersion(HW, file!.id, { expectedRevision: after.revision })).rejects.toMatchObject({ code: "conflict" });
  });
});
