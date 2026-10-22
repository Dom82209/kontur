import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { OperationsProvider } from "../../app/operations";
import { ToastProvider } from "../../app/toasts";
import type { ReviewRun } from "../../lib/bridge";
import { RunMonitor } from "./RunMonitor";
import { WorkflowRail } from "./WorkflowRail";

/** Серверный рендер (без DOM): компоненты показывают реальное состояние из backend. */

let consoleError: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
  consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
});
afterEach(() => {
  expect(consoleError).not.toHaveBeenCalled();
  consoleError.mockRestore();
});

const wrap = (node: ReturnType<typeof createElement>) => renderToStaticMarkup(createElement(ToastProvider, null, createElement(OperationsProvider, null, node)));

const run = (patch: Partial<ReviewRun>): ReviewRun => ({
  id: "run-1",
  homeworkId: "hw",
  kind: "review_version",
  role: "reviewer",
  provider: "codex",
  versionId: "v1",
  sourceBundleId: "b1",
  sourcesUsed: [],
  status: "waiting_for_provider",
  progress: 0,
  step: "",
  startedAt: "2026-10-22T10:00:00Z",
  finishedAt: null,
  savedAsIteration: false,
  result: null,
  errorMessage: null,
  provenance: "manual",
  styleThreshold: 4,
  assessment: null,
  ...patch,
});

describe("WorkflowRail", () => {
  it("отмечает текущий этап из агрегата и пройденные этапы", () => {
    const html = renderToStaticMarkup(createElement(WorkflowRail, { homeworkId: "hw", stage: "codex_review", cycle: 2 }));
    const current = html.match(/data-status="current"[^>]*aria-current="step"/g) ?? [];
    expect(current).toHaveLength(1);
    expect(html).toContain("цикл исправлений 2");
    expect((html.match(/data-status="done"/g) ?? []).length).toBe(4);
  });

  it("показывает заблокированный этап", () => {
    const html = renderToStaticMarkup(createElement(WorkflowRail, { homeworkId: "hw", stage: "teacher_review", blockedStages: ["ready_to_submit"] }));
    expect(html).toContain('data-status="blocked"');
  });
});

describe("RunMonitor", () => {
  it("без AI-адаптера честно показывает ожидание провайдера, без процентов и волны", () => {
    const html = wrap(createElement(RunMonitor, { run: run({ status: "waiting_for_provider" }), handoff: createElement("button", null, "Ввести результат") }));
    expect(html).toContain("AI-адаптер ещё не подключён");
    expect(html).toContain("Ввести результат");
    expect(html).toContain('data-status="waiting_for_provider"');
    expect(html).not.toMatch(/\d+%/);
    expect(html).not.toContain("Прогресс запуска");
  });

  it("ручная передача шага отмечается как «вручную»", () => {
    const html = wrap(createElement(RunMonitor, { run: run({ status: "manual_handoff" }) }));
    expect(html).toContain("Вручную");
    expect(html).toContain("ручное выполнение");
    expect(html).toContain("Отменить");
  });

  it("завершённый запуск не предлагает отмену", () => {
    const html = wrap(createElement(RunMonitor, { run: run({ status: "completed", progress: 1, provenance: "provider", step: "Готово" }) }));
    expect(html).not.toContain("Отменить");
    expect(html).toContain("100%");
  });
});
