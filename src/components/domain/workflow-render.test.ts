import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { OperationsProvider } from "../../app/operations";
import { ToastProvider } from "../../app/toasts";
import type { ReviewRun } from "../../lib/bridge";
import { ProviderCard } from "./ProviderCard";
import { RunMonitor } from "./RunMonitor";
import { WorkflowRail } from "./WorkflowRail";

/** Серверный рендер (без DOM): компоненты показывают реальное состояние из backend. */

// Волна зависит от настроек анимации окна (браузерный контекст) — здесь важна только её подпись.
vi.mock("./AiWaveform", () => ({ AiWaveform: ({ label }: { label: string }) => createElement("div", { role: "img", "aria-label": label }) }));

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
  it("ожидающий шаг честно говорит, что результата нет, — без процентов и волны", () => {
    const html = wrap(createElement(RunMonitor, { run: run({ status: "waiting_for_provider" }), handoff: createElement("button", null, "Передать Codex") }));
    expect(html).toContain("Результата ещё нет");
    expect(html).toContain("Ничего не сохранится без вашего подтверждения");
    expect(html).toContain("Передать Codex");
    expect(html).toContain('data-status="waiting_for_provider"');
    expect(html).not.toMatch(/\d+%/);
    expect(html).not.toContain("Официальный CLI обрабатывает материалы");
  });

  it("ручная передача объясняет, что делать, и позволяет отменить шаг", () => {
    const html = wrap(createElement(RunMonitor, { run: run({ status: "manual_handoff" }) }));
    expect(html).toContain("Ручная передача");
    expect(html).toContain("Отменить шаг");
  });

  it("выполняющееся задание CLI показывает время и кнопку остановки, а не проценты", () => {
    const job = { id: "job-1", runId: "run-1", homeworkId: "hw", task: "solution_review", status: "running", adapterLabel: "Codex CLI · 0.47.0", parseStatus: null, errorMessage: null } as never;
    const html = wrap(createElement(RunMonitor, { run: run({ status: "running" }), job }));
    expect(html).toContain("Codex CLI · 0.47.0");
    expect(html).toContain("0:00");
    expect(html).toContain("Остановить");
    expect(html).toContain("Официальный CLI обрабатывает материалы");
    expect(html).not.toMatch(/\d+%/);
  });

  it("полученный ответ ждёт проверки", () => {
    const job = { id: "job-1", runId: "run-1", homeworkId: "hw", task: "solution_review", status: "response_ready", adapterLabel: null, parseStatus: "partial", errorMessage: null } as never;
    const html = wrap(createElement(RunMonitor, { run: run({ status: "waiting_for_provider", step: "Ответ получен — проверьте и подтвердите результат" }), job }));
    expect(html).toContain("ответ ждёт проверки");
    expect(html).toContain("ответ распознан частично");
  });

  it("завершённый запуск не предлагает отмену", () => {
    const html = wrap(createElement(RunMonitor, { run: run({ status: "completed", progress: 1, provenance: "provider", step: "Ответ провайдера (CLI) подтверждён пользователем" }) }));
    expect(html).not.toContain("Отменить");
    expect(html).toContain("подтверждён пользователем");
  });
});

describe("ProviderCard", () => {
  const status = {
    provider: "claude",
    state: "disconnected",
    accountLabel: null,
    lastCheckedAt: "2026-10-22T10:00:00Z",
    errorMessage: null,
    adapter: "manual_handoff",
    roles: [],
    availability: "manual",
    tasks: ["teacher_review"],
    cli: { enabled: false, detected: true, executable: "~/.local/bin/claude", version: "2.1.0 (Claude Code)", supported: true, missingFlags: [], loggedIn: null, checkedAt: null },
    handoff: { appInstalled: true, webUrl: "https://claude.ai/new" },
  } as const;

  it("найденный CLI не подключён сам — нужна явная кнопка, полей для пароля нет", () => {
    const html = wrap(createElement(ProviderCard, { status: structuredClone(status) as never }));
    expect(html).toContain("Подключить CLI");
    expect(html).toContain("не подключён");
    expect(html).toContain("ручная передача");
    expect(html).toContain("Проверка по комментариям преподавателя");
    expect(html).not.toMatch(/type="password"|<input/);
  });
});
