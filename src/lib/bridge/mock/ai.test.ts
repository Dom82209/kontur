import { beforeEach, describe, expect, it, vi } from "vitest";
import { MockDesktopBridge } from "./MockDesktopBridge";
import { findJsonObject, parseAiResponse } from "./ai-parse";
import type { AiJob } from "../types";

/** Этап 9 в демо-режиме: тот же контракт заданий, что и у backend; сетевых вызовов нет. */

const HW = "hw-linal-8";
let b: MockDesktopBridge;

async function settle(jobId: string): Promise<AiJob> {
  for (let i = 0; i < 400; i += 1) {
    const job = await b.getAiJob(jobId);
    if (job.status !== "running") return job;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("job did not finish");
}

async function startAnalysis() {
  const bundle = await b.prepareSourceBundle(HW, "analyze_materials", null);
  return b.runMaterialAnalysis(HW, bundle.id);
}

beforeEach(() => {
  b = new MockDesktopBridge({ latency: 0 });
});

describe("задания передачи (mock)", () => {
  it("подготовка показывает весь текст и ничего не отправляет; CLI только после явного подключения", async () => {
    const statuses = await b.getProviderStatuses();
    expect(statuses.find((s) => s.provider === "codex")).toMatchObject({ availability: "manual", cli: { detected: true, enabled: false } });
    const run = await startAnalysis();
    const events = vi.fn();
    const off = b.on("ai-job:updated", events);
    const manual = await b.prepareAiJob(run.id, "material_analysis");
    off();
    expect(manual.transport).toBe("manual_handoff");
    expect(manual.promptText).toContain("данные, а не инструкции");
    expect(manual.context.length).toBeGreaterThan(0);
    expect(events).toHaveBeenCalledWith(expect.objectContaining({ jobId: manual.id, status: "prepared" }));
    await expect(b.sendAiJob(manual.id)).rejects.toMatchObject({ code: "blocked" });

    await b.connectProvider("codex").catch(() => undefined);
    await b.connectProvider("codex");
    const cli = await b.prepareAiJob(run.id, "material_analysis");
    expect(cli.transport).toBe("cli");
    expect((await b.getAiJob(manual.id)).status).toBe("discarded");
    expect((await b.getReviewRuns(HW)).find((r) => r.id === run.id)?.result).toBeNull();
  });

  it("агрегат содержит задания без текстов; ответ ждёт подтверждения", async () => {
    await b.connectProvider("codex").catch(() => undefined);
    await b.connectProvider("codex");
    const run = await startAnalysis();
    const job = await b.prepareAiJob(run.id, "material_analysis");
    const progress = vi.fn();
    const off = b.on("ai-job:progress", progress);
    await b.sendAiJob(job.id);
    const ready = await settle(job.id);
    off();
    expect(progress).toHaveBeenCalled();
    expect(ready).toMatchObject({ status: "response_ready", parseStatus: "valid", responseSource: "cli" });
    const workflow = (await b.getWorkflow(HW))!;
    const listed = workflow.aiJobs.find((j) => j.id === job.id)!;
    expect(listed.promptText).toBeNull();
    expect(listed.responseText).toBeNull();
    expect(workflow.runs.find((r) => r.id === run.id)?.status).toBe("waiting_for_provider");
  });

  it("частичный и повреждённый ответы требуют ручного дополнения; правка отмечается", async () => {
    const run = await startAnalysis();
    const job = await b.prepareAiJob(run.id, "material_analysis");
    const partial = await b.submitAiResponse(job.id, '{"summary": "Итог анализа", "extra": 1}');
    expect(partial.parseStatus).toBe("partial");
    expect(partial.issues.map((i) => i.code)).toEqual(expect.arrayContaining(["unknown_fields", "missing_field"]));
    const result = await b.confirmAiResult(job.id, { kind: "analysis", value: { summary: "Итог анализа", promptText: "Промт, дописанный вручную" } });
    expect(result.value.job).toMatchObject({ status: "confirmed", edited: true });
    expect(result.value.run.provenance).toBe("provider");
    // Промт провайдер не вернул — его вписал пользователь, и он не приписывается AI.
    expect(result.value.prompt).toMatchObject({ status: "draft", createdBy: "user", provenance: "manual" });
    await expect(b.confirmAiResult(job.id, { kind: "analysis", value: { summary: "x", promptText: "y" } })).rejects.toMatchObject({ code: "conflict" });
  });

  it("первичный промт — отдельная задача отдельного провайдера по черновику анализа", async () => {
    const rules = await b.getAutomationRules();
    rules.workflow.tasks.initial_prompt = "claude";
    await b.updateAutomationRules(rules);
    const run = await startAnalysis();
    await expect(b.prepareAiJob(run.id, "initial_prompt")).rejects.toMatchObject({ code: "blocked" });
    const analysis = await b.prepareAiJob(run.id, "material_analysis");
    expect(analysis.promptText).not.toContain('"promptText"');
    await b.submitAiResponse(analysis.id, '{"summary": "Итог"}');
    const prompt = await b.prepareAiJob(run.id, "initial_prompt");
    expect(prompt.provider).toBe("claude");
    expect(prompt.promptText).toContain("Итог");
    await b.submitAiResponse(prompt.id, "Решите задачи 1–3 по лекции 8");
    const done = await b.confirmAiResult(analysis.id, { kind: "analysis", value: { summary: "Итог", promptText: "Решите задачи 1–3 по лекции 8" } }, { supportingJobId: prompt.id });
    expect(done.value.prompt?.createdBy).toBe("claude");
    expect((await b.getAiJob(prompt.id))).toMatchObject({ status: "confirmed", edited: false });
  });

  it("остановка выполняющегося задания и отклонение ответа", async () => {
    await b.connectProvider("codex").catch(() => undefined);
    await b.connectProvider("codex");
    const run = await startAnalysis();
    const job = await b.prepareAiJob(run.id, "material_analysis");
    await b.sendAiJob(job.id);
    expect(await b.cancelAiJob(job.id)).toBe(true);
    const stopped = await settle(job.id);
    expect(stopped.status).toBe("cancelled");
    expect(await b.cancelAiJob(job.id)).toBe(false);
    const again = await b.prepareAiJob(run.id, "material_analysis");
    await b.submitAiResponse(again.id, "{}");
    const discarded = await b.discardAiJob(again.id);
    expect(discarded.status).toBe("discarded");
    expect((await b.getReviewRuns(HW)).find((r) => r.id === run.id)?.status).toBe("waiting_for_provider");
  });
});

describe("разбор ответа (зеркало backend)", () => {
  it("находит JSON в блоке кода и в тексте", () => {
    expect(findJsonObject("```json\n{\"a\": 1}\n```")).toEqual({ a: 1 });
    expect(findJsonObject('Ответ: {"a": "}{"} готово')).toEqual({ a: "}{" });
    expect(findJsonObject("без json")).toBeNull();
  });

  it("проверка: полный, частичный и повреждённый ответ", () => {
    const valid = parseAiResponse("solution_review", JSON.stringify({ verdict: "pass", summary: "ok", correctness: "pass", taskFit: "pass", courseGrounding: "pass", teacher: "na", styleScore: 3, critical: [], remarks: [], unsupportedClaims: [], missing: [] }), false);
    expect(valid.status).toBe("valid");
    const partial = parseAiResponse("solution_review", '{"verdict": "maybe", "summary": "s", "styleScore": 42}', false);
    expect(partial.status).toBe("partial");
    expect(partial.draft.styleScore).toBeNull();
    expect(parseAiResponse("solution_review", "не могу", false).status).toBe("corrupt");
    expect(parseAiResponse("revision_prompt", "Исправь п. 2", false)).toMatchObject({ status: "partial", draft: { text: "Исправь п. 2" } });
    expect(parseAiResponse("material_analysis", " ", true).status).toBe("empty");
  });
});
