import type { TypedEmitter } from "../emitter";
import {
  BridgeError,
  type AutomationRules,
  type BridgeErrorPayload,
  type CriterionStatus,
  type DroppedItem,
  type EvidenceReference,
  type FileRef,
  type Homework,
  type HomeworkIteration,
  type HomeworkVersion,
  type HomeworkWorkflowStage,
  type MissingSource,
  type Operation,
  type OperationKind,
  type OperationResult,
  type PackageCheck,
  type PackageCheckId,
  type ProviderId,
  type ProviderStatus,
  type Readiness,
  type ReadinessCheck,
  type ReviewCriterion,
  type ReviewFinding,
  type ReviewResult,
  type ReviewRun,
  type RevisionPrompt,
  type RunKind,
  type SourceBundle,
  type SourceItem,
  type SourceKind,
  type StyleDivergence,
  type StyleProfile,
  type SubmissionConfirmation,
  type SubmissionPackage,
  type SubmissionReceipt,
  type TeacherComment,
  type TeacherCommentInput,
  type TeacherRequirement,
  type UnsupportedClaim,
  type VersionComparison,
  type Week,
  type WorkflowRole,
} from "../types";
import { applyNamePattern } from "../../naming";
import { kindFromName, pad2, providerName, SOURCE_KIND_LABEL, STYLE_REASON_LABEL } from "../../format";
import { at, buildSections, makeFile, previousWorks, STYLE_PROFILE_FILE, subjectName } from "./data";

/**
 * Mock-движок рабочего цикла ДЗ. Результаты проверок детерминированы номером версии,
 * чтобы сценарий «v01 → ошибки → v02 → замечание → комментарии → v03 → сдача»
 * воспроизводился одинаково в интерфейсе и в тестах.
 */

export interface MockContext {
  homework(): Homework[];
  weeks(): Map<string, Week[]>;
  rules(): AutomationRules;
  providers(): ProviderStatus[];
  emitter: TypedEmitter;
  wait(ms: number): Promise<void>;
  runOp(kind: OperationKind, title: string, detail: string | null, paths: string[], steps: number, commit: () => void, undo: (() => void) | null, homeworkId: string | null): Promise<Operation>;
  startOp(kind: OperationKind, title: string, detail: string | null, paths: string[], homeworkId: string | null): Operation;
  finishOp(op: Operation): void;
  failOp(op: Operation, error: BridgeErrorPayload): void;
  createVersion(homeworkId: string, fileId: string, meta: { author?: HomeworkVersion["author"]; promptId?: string | null }): Promise<OperationResult<HomeworkVersion>>;
  candidates(homeworkId: string): Promise<FileRef[]>;
}

const clone = <T,>(v: T): T => structuredClone(v);
/** Статус может смениться из cancelRun во время await — читаем без сужения типа. */
const isCancelled = (r: ReviewRun): boolean => r.status === "cancelled";
let seq = 0;
const uid = (p: string): string => `${p}-${Date.now().toString(36)}-${(seq += 1)}`;

const ROLE_OF: Record<RunKind, WorkflowRole> = {
  analyze_materials: "analyst",
  review_version: "reviewer",
  revision_prompt: "reviewer",
  review_teacher: "reviewer",
};

const KINDS_FOR: Record<RunKind, SourceKind[]> = {
  analyze_materials: ["lecture", "seminar", "task", "data", "extra", "style", "previous_work"],
  review_version: ["lecture", "seminar", "task", "data", "style", "previous_work", "current_version", "previous_version"],
  revision_prompt: ["lecture", "seminar", "task", "data", "style", "current_version", "previous_version", "teacher_comments"],
  review_teacher: ["lecture", "seminar", "task", "style", "current_version", "teacher_comments"],
};

const STEPS: Record<RunKind, string[]> = {
  analyze_materials: ["Чтение условия", "Разбор лекций и семинаров", "Сверка с профилем стиля", "Сборка промта для автора"],
  review_version: ["Сверка с условием", "Проверка решения", "Поиск опоры в материалах", "Сравнение со стилем и прошлыми работами", "Проверка оформления и файлов"],
  revision_prompt: ["Сбор замечаний", "Формулировка исправлений"],
  review_teacher: ["Разбор комментариев", "Выделение требований", "Сверка версии с требованиями"],
};


interface ScriptFinding {
  key: "A" | "B" | "C";
  severity: ReviewFinding["severity"];
  criterion: ReviewFinding["criterion"];
  text: string;
  location: string;
  evidence: (b: { task: FileRef | null; lecture: FileRef | null; seminar: FileRef | null }) => EvidenceReference[];
  /** Номер версии, в которой появляется, и номер, в которой исправлено. */
  from: number;
  until: number;
}

function ev(f: FileRef | null, locator: string): EvidenceReference[] {
  return f ? [{ sourceId: f.id, fileName: f.name, locator }] : [];
}

function scriptFor(sheet: boolean): ScriptFinding[] {
  return sheet
    ? [
        { key: "A", severity: "critical", criterion: "correctness", text: "Сводная по регионам считает сумму вместо среднего — в условии нужен средний чек", location: "лист «Сводная по регионам», столбец C", evidence: (b) => [...ev(b.task, "п. 2 условия"), ...ev(b.lecture, "слайд 14 — параметры значений")], from: 1, until: 2 },
        { key: "B", severity: "critical", criterion: "task_fit", text: "Доля продаж в выводах посчитана от итога по региону, а не от общего итога", location: "лист «Выводы», абзац 2", evidence: (b) => [...ev(b.task, "п. 4 условия"), ...ev(b.seminar, "задача 3")], from: 1, until: 2 },
        { key: "C", severity: "normal", criterion: "completeness", text: "У диаграммы нет единиц измерения на оси значений (руб.)", location: "лист «Диаграмма»", evidence: (b) => ev(b.seminar, "пример оформления диаграммы"), from: 2, until: 3 },
      ]
    : [
        { key: "A", severity: "critical", criterion: "correctness", text: "В задаче 2 потерян множитель при дифференцировании — итоговый ответ неверен", location: "задача 2, шаг 3", evidence: (b) => [...ev(b.task, "задача 2"), ...ev(b.lecture, "стр. 6 — правило цепочки")], from: 1, until: 2 },
        { key: "B", severity: "critical", criterion: "task_fit", text: "Не выполнен пункт 4: нет проверки ответа подстановкой", location: "задача 4", evidence: (b) => [...ev(b.task, "п. 4"), ...ev(b.seminar, "задача 5")], from: 1, until: 2 },
        { key: "C", severity: "normal", criterion: "completeness", text: "Переход к пределу без ссылки на теорему из лекции", location: "задача 3", evidence: (b) => ev(b.lecture, "стр. 9 — теорема"), from: 2, until: 3 },
      ];
}

function styleFor(n: number, threshold: number): StyleDivergence {
  if (n <= 1)
    return {
      score: 6,
      threshold,
      reasons: [
        { kind: "template", text: "Шаблонные связки в каждом выводе", example: "«В заключение стоит отметить, что…»" },
        { kind: "formal_tone", text: "Тон заметно официальнее прошлых работ", example: "«Представляется целесообразным…»" },
        { kind: "repetition", text: "Одинаковое начало абзацев", example: "«Таким образом,» — 4 раза" },
        { kind: "unsupported", text: "Обобщение без ссылки на материал", example: "«как известно, сезонность всегда…»" },
      ],
    };
  if (n === 2)
    return {
      score: 4,
      threshold,
      reasons: [
        { kind: "uncharacteristic_words", text: "Слова, которых нет в прошлых работах", example: "«ключевой инсайт», «драйвер»" },
        { kind: "repetition", text: "Повтор конструкции в двух выводах", example: "«что говорит о…»" },
      ],
    };
  return { score: 3, threshold, reasons: [{ kind: "template", text: "Одна устойчивая формула в выводе", example: "«в целом можно сказать»" }] };
}

function firstSentence(t: string): string {
  const m = t.trim().match(/^(.+?[.!?])(\s|$)/);
  return (m?.[1] ?? t).trim();
}

export function parseTranscript(text: string): { question: string; answer: string }[] {
  const pairs: { question: string; answer: string }[] = [];
  let q: string | null = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const mq = line.match(/^(?:В|Вопрос|Q|Я)\s*[:.\-—]\s*(.+)$/i);
    const ma = line.match(/^(?:О|Ответ|A|Преподаватель|П)\s*[:.\-—]\s*(.+)$/i);
    if (mq) q = mq[1]!;
    else if (ma) {
      pairs.push({ question: q ?? "", answer: ma[1]! });
      q = null;
    } else if (pairs.length > 0 && q === null) pairs[pairs.length - 1]!.answer += ` ${line}`;
    else q = q ? `${q} ${line}` : line;
  }
  if (pairs.length === 0 && text.trim()) pairs.push({ question: q ?? "", answer: q ? "" : text.trim() });
  return pairs.filter((p) => p.answer || p.question);
}

export class MockWorkflow {
  private bundles = new Map<string, SourceBundle>();
  private runs: ReviewRun[] = [];
  private prompts: RevisionPrompt[] = [];
  private comments: TeacherComment[] = [];
  private requirements: TeacherRequirement[] = [];
  private findings = new Map<string, ReviewFinding[]>();
  private packages = new Map<string, SubmissionPackage>();
  private receipts = new Map<string, SubmissionReceipt>();
  private runOps = new Map<string, { op: Operation; prevStage: HomeworkWorkflowStage }>();
  private readonly profile: StyleProfile;

  constructor(private ctx: MockContext) {
    this.profile = {
      id: "style-1",
      fileName: STYLE_PROFILE_FILE,
      relativePath: STYLE_PROFILE_FILE,
      updatedAt: at(-9, 21),
      traits: [
        "короткие предложения, без вводных «стоит отметить»",
        "выводы — 2–3 предложения на пункт",
        "термины — как на лекциях курса",
        "в Jupyter решение только в ячейках решения, без комментариев в коде",
        "письменная часть — формулы в LaTeX",
      ],
    };
  }

  // ───────────── Источники ─────────────

  async getStyleProfile(): Promise<StyleProfile | null> {
    await this.ctx.wait(90);
    return clone({ ...this.profile, relativePath: this.ctx.rules().workflow.styleProfilePath });
  }

  async prepareSourceBundle(homeworkId: string, purpose: RunKind, versionId: string | null = null): Promise<SourceBundle> {
    await this.ctx.wait(260);
    const h = this.hw(homeworkId);
    const bundle = this.buildBundle(h, purpose, versionId);
    this.bundles.set(bundle.id, bundle);
    return clone(bundle);
  }

  async setSourceIncluded(bundleId: string, itemId: string, included: boolean): Promise<SourceBundle> {
    await this.ctx.wait(60);
    const b = this.bundles.get(bundleId);
    if (!b) throw new BridgeError("not_found", "Набор источников не найден");
    const item = b.items.find((i) => i.id === itemId);
    if (!item) throw new BridgeError("not_found", "Источник не найден");
    item.included = included;
    this.recomputeMissing(b);
    return clone(b);
  }

  // ───────────── Запуски ─────────────

  async runMaterialAnalysis(homeworkId: string, bundleId: string): Promise<ReviewRun> {
    return this.startRun(homeworkId, "analyze_materials", bundleId, null);
  }

  async runVersionReview(homeworkId: string, versionId: string, bundleId: string): Promise<ReviewRun> {
    return this.startRun(homeworkId, "review_version", bundleId, versionId);
  }

  async runRevisionPrompt(homeworkId: string, versionId: string, bundleId: string): Promise<ReviewRun> {
    return this.startRun(homeworkId, "revision_prompt", bundleId, versionId);
  }

  async runTeacherReview(homeworkId: string, versionId: string, bundleId: string): Promise<ReviewRun> {
    if (this.comments.filter((c) => c.homeworkId === homeworkId).length === 0) throw new BridgeError("blocked", "Сначала добавьте комментарии преподавателя");
    return this.startRun(homeworkId, "review_teacher", bundleId, versionId);
  }

  async cancelRun(runId: string): Promise<ReviewRun> {
    const run = this.mustRun(runId);
    if (run.status !== "queued" && run.status !== "running") throw new BridgeError("conflict", "Запуск уже завершён");
    run.status = "cancelled";
    run.finishedAt = new Date().toISOString();
    const meta = this.runOps.get(run.id);
    if (meta) {
      this.setStage(this.hw(run.homeworkId), meta.prevStage, "Запуск отменён пользователем");
      meta.op.title = `${meta.op.title} — отменено`;
      this.ctx.finishOp(meta.op);
    }
    this.ctx.emitter.emit("review:failed", { run: clone(run), error: { code: "cancelled", message: "Запуск отменён" } });
    return clone(run);
  }

  async getReviewRuns(homeworkId: string): Promise<ReviewRun[]> {
    await this.ctx.wait(120);
    return clone(this.runs.filter((r) => r.homeworkId === homeworkId).sort((a, b) => b.startedAt.localeCompare(a.startedAt)));
  }

  async saveRunAsIteration(runId: string): Promise<ReviewRun> {
    await this.ctx.wait(120);
    const run = this.mustRun(runId);
    this.assertMutable(this.hw(run.homeworkId));
    if (run.status !== "completed") throw new BridgeError("conflict", "Сохранить можно только завершённый запуск");
    run.savedAsIteration = true;
    return clone(run);
  }

  // ───────────── Промты ─────────────

  async createRevisionPrompt(reviewRunId: string, editedText?: string): Promise<OperationResult<RevisionPrompt>> {
    const run = this.mustRun(reviewRunId);
    const h = this.hw(run.homeworkId);
    this.assertMutable(h);
    const draft = run.result?.prompt;
    if (run.status !== "completed" || !draft) throw new BridgeError("conflict", "У этого запуска нет промта исправлений");
    const existing = this.prompts.find((p) => p.reviewRunId === run.id);
    if (existing) {
      const op = await this.ctx.runOp(
        "revision_prompt",
        "Промт исправлений обновлён",
        subjectName(h.subjectId),
        [],
        1,
        () => {
          if (editedText !== undefined && !existing.sentAt) existing.text = editedText;
        },
        null,
        h.id,
      );
      return { operation: op, value: clone(existing) };
    }
    const prompt: RevisionPrompt = { ...clone(draft), id: uid("prm"), text: editedText ?? draft.text, createdAt: new Date().toISOString() };
    const op = await this.ctx.runOp(
      "revision_prompt",
      `Промт исправлений для ${this.authorName()}`,
      `${subjectName(h.subjectId)} · по проверке ${this.versionLabel(h, run.versionId)}`,
      [`${h.folderPath}/.kontur/prompts/${prompt.id}.md`],
      2,
      () => {
        this.prompts.push(prompt);
        this.setStage(h, "prompt_ready", `Промт исправлений по проверке ${this.versionLabel(h, run.versionId)} готов`);
      },
      null,
      h.id,
    );
    return { operation: op, value: clone(prompt) };
  }

  async updatePromptText(promptId: string, text: string): Promise<RevisionPrompt> {
    await this.ctx.wait(80);
    const p = this.mustPrompt(promptId);
    this.assertMutable(this.hw(p.homeworkId));
    if (p.sentAt) throw new BridgeError("immutable", "Промт уже передан автору — создайте новый");
    p.text = text;
    return clone(p);
  }

  async getPrompts(homeworkId: string): Promise<RevisionPrompt[]> {
    await this.ctx.wait(100);
    return clone(this.prompts.filter((p) => p.homeworkId === homeworkId).sort((a, b) => a.createdAt.localeCompare(b.createdAt)));
  }

  async sendPromptToAuthor(promptId: string): Promise<OperationResult<RevisionPrompt>> {
    const p = this.mustPrompt(promptId);
    const h = this.hw(p.homeworkId);
    this.assertMutable(h);
    if (p.sentAt) throw new BridgeError("conflict", "Этот промт уже передан — по нему создана версия");
    const provider = this.ctx.rules().workflow.roles.author;
    this.ensureConnected(provider);
    const prev = h.workflowStage;
    this.setStage(h, "claude_solving", `Промт передан: ${providerName(provider)} создаёт решение`);
    const op = this.ctx.startOp("send_prompt", `${providerName(provider)} создаёт решение`, subjectName(h.subjectId), [`${h.folderPath}/02 Работа`], h.id);
    const steps = ["Передача промта и источников", "Автор читает материалы", "Создание решения", "Запись файла в «02 Работа»"];
    try {
      for (let i = 0; i < steps.length; i += 1) {
        await this.ctx.wait(650);
        this.ensureConnected(provider);
        this.ctx.emitter.emit("operation:progress", { operationId: op.id, kind: "ai_run", progress: (i + 1) / steps.length, message: steps[i]! });
      }
    } catch (e) {
      this.setStage(h, prev, "Автор недоступен — решение не создано");
      this.ctx.failOp(op, { code: e instanceof BridgeError ? e.code : "unknown", message: e instanceof Error ? e.message : "Ошибка" });
      throw e;
    }
    p.sentAt = new Date().toISOString();
    this.ctx.finishOp(op);
    const [file] = await this.ctx.candidates(h.id);
    await this.ctx.createVersion(h.id, file?.id ?? uid("file"), { author: provider, promptId: p.id });
    return { operation: clone(op), value: clone(p) };
  }

  /** Вызывается bridge при создании любой версии. */
  onVersionCreated(h: Homework, v: HomeworkVersion): void {
    if (v.promptId) {
      const p = this.prompts.find((x) => x.id === v.promptId);
      v.sourceBundleId = p?.reviewRunId ? (this.runs.find((r) => r.id === p.reviewRunId)?.sourceBundleId ?? null) : null;
    }
    this.setStage(h, "codex_review", `${v.label} создана — ожидает проверки`);
    this.packages.delete(h.id);
  }

  // ───────────── Итерации и сравнение ─────────────

  async getIterations(homeworkId: string): Promise<HomeworkIteration[]> {
    await this.ctx.wait(160);
    const h = this.hw(homeworkId);
    const receipt = this.receipts.get(h.id);
    const registry = this.findings.get(h.id) ?? [];
    return clone(
      h.versions.map((v, i) => {
        const review = this.pinnedRun(h.id, v.id, "review_version");
        const teacherReview = this.pinnedRun(h.id, v.id, "review_teacher");
        const revisionPrompt = [...this.prompts].reverse().find((p) => p.basedOnVersionId === v.id && p.kind !== "initial") ?? null;
        const running = this.runs.some((r) => r.versionId === v.id && (r.status === "running" || r.status === "queued"));
        const verdict = review?.result?.verdict;
        const teacherFailed = teacherReview?.result?.verdict === "needs_fixes";
        const status: HomeworkIteration["status"] =
          receipt?.versionId === v.id ? "submitted" : h.finalVersionId === v.id ? "final" : running ? "reviewing" : !review ? "draft" : verdict === "needs_fixes" || teacherFailed ? "failed" : "passed";
        return {
          versionId: v.id,
          label: v.label,
          author: v.author,
          createdAt: v.createdAt,
          files: v.files,
          prompt: this.prompts.find((p) => p.id === v.promptId) ?? null,
          sourceBundleId: v.sourceBundleId,
          review,
          teacherReview,
          revisionPrompt,
          nextVersionId: h.versions[i + 1]?.id ?? null,
          fixedFindings: registry.filter((f) => f.fixedIn === v.id),
          openFindings: review?.result ? [...review.result.critical, ...review.result.remarks] : [],
          status,
        };
      }),
    );
  }

  async compareVersions(fromVersionId: string, toVersionId: string): Promise<VersionComparison> {
    await this.ctx.wait(200);
    const h = this.ctx.homework().find((x) => x.versions.some((v) => v.id === fromVersionId));
    const a = h?.versions.find((v) => v.id === fromVersionId);
    const b = h?.versions.find((v) => v.id === toVersionId);
    if (!h || !a || !b) throw new BridgeError("not_found", "Версии не найдены");
    const [older, newer] = a.number < b.number ? [a, b] : [b, a];
    const ra = this.latestRun(h.id, older.id, "review_version")?.result ?? null;
    const rb = this.latestRun(h.id, newer.id, "review_version")?.result ?? null;
    const open = (r: ReviewResult | null) => (r ? [...r.critical, ...r.remarks] : []);
    const ida = new Set(open(ra).map((f) => f.id));
    const idb = new Set(open(rb).map((f) => f.id));
    const titles = [...new Set([...older.sections.map((s) => s.title), ...newer.sections.map((s) => s.title)])];
    const teacherStat = (vid: string) => {
      const reqs = this.requirements.filter((r) => r.checkedVersionId === vid);
      return reqs.length ? { done: reqs.filter((r) => r.status === "done" || r.status === "na").length, total: reqs.length } : null;
    };
    const grounding = (r: ReviewResult | null): CriterionStatus | null => r?.criteria.find((c) => c.id === "course_grounding")?.status ?? null;
    return {
      fromVersionId: older.id,
      toVersionId: newer.id,
      fixed: open(ra).filter((f) => !idb.has(f.id)),
      introduced: open(rb).filter((f) => !ida.has(f.id)),
      sections: titles.map((t) => {
        const o = older.sections.find((s) => s.title === t);
        const n = newer.sections.find((s) => s.title === t);
        return { title: t, diff: !o ? "added" : !n ? "removed" : o.hash === n.hash ? "same" : "changed" };
      }),
      style: { from: ra?.style?.score ?? null, to: rb?.style?.score ?? null },
      grounding: { from: grounding(ra), to: grounding(rb), unsupportedFrom: ra?.unsupportedClaims.length ?? 0, unsupportedTo: rb?.unsupportedClaims.length ?? 0 },
      teacher: { from: teacherStat(older.id), to: teacherStat(newer.id) },
    };
  }

  // ───────────── Преподаватель ─────────────

  async getTeacherComments(homeworkId: string): Promise<TeacherComment[]> {
    await this.ctx.wait(100);
    return clone(this.comments.filter((c) => c.homeworkId === homeworkId));
  }

  async saveTeacherComments(homeworkId: string, inputs: TeacherCommentInput[]): Promise<OperationResult<TeacherComment[]>> {
    const h = this.hw(homeworkId);
    this.assertMutable(h);
    const valid = inputs.filter((c) => c.question.trim() || c.answer.trim());
    if (valid.length === 0) throw new BridgeError("conflict", "Нет текста комментария");
    const now = new Date().toISOString();
    const added: TeacherComment[] = valid.map((c) => ({ ...c, question: c.question.trim(), answer: c.answer.trim(), id: uid("tc"), homeworkId, addedAt: now, attachment: null }));
    const op = await this.ctx.runOp(
      "save_teacher_comments",
      `Добавлено комментариев: ${added.length}`,
      subjectName(h.subjectId),
      [`${h.folderPath}/.kontur/teacher.md`],
      2,
      () => this.comments.push(...added),
      () => {
        this.comments = this.comments.filter((c) => !added.some((a) => a.id === c.id));
      },
      h.id,
    );
    return { operation: op, value: clone(added) };
  }

  async importTeacherTranscript(homeworkId: string, input: { text?: string; file?: DroppedItem; source: TeacherComment["source"]; date: string }): Promise<OperationResult<TeacherComment[]>> {
    const h = this.hw(homeworkId);
    this.assertMutable(h);
    await this.ctx.wait(input.file ? 700 : 200);
    let text = input.text ?? "";
    let attachment: FileRef | null = null;
    if (input.file) {
      const kind = kindFromName(input.file.name);
      if (!["text", "pdf", "image"].includes(kind)) throw new BridgeError("io_error", "Поддерживаются Markdown, TXT, PDF и изображения");
      attachment = makeFile(input.file.name, `${h.folderPath}/01 Условие и данные/Переписка`, new Date().toISOString(), input.file.sizeBytes);
      // Mock: текст «извлечён» из файла. Backend выполняет настоящее извлечение/OCR.
      text =
        kind === "text"
          ? "В: Можно ли использовать данные за весь год?\nО: Да, но выводы делайте по последнему кварталу.\nВ: Нужен ли титульный лист?\nО: Нет, достаточно имени файла по шаблону."
          : `О: [распознано из «${input.file.name}» — проверьте текст] Добавьте краткий вывод в конце каждого пункта.`;
    }
    const pairs = parseTranscript(text);
    if (pairs.length === 0) throw new BridgeError("conflict", "Не удалось выделить вопросы и ответы");
    const now = new Date().toISOString();
    const added: TeacherComment[] = pairs.map((pair) => ({
      ...pair,
      source: input.source,
      date: input.date,
      id: uid("tc"),
      homeworkId,
      addedAt: now,
      attachment,
    }));
    const op = await this.ctx.runOp(
      "save_teacher_comments",
      `Добавлено комментариев: ${added.length}`,
      subjectName(h.subjectId),
      [`${h.folderPath}/.kontur/teacher.md`, ...(attachment ? [attachment.relativePath] : [])],
      2,
      () => this.comments.push(...added),
      () => {
        this.comments = this.comments.filter((comment) => !added.some((item) => item.id === comment.id));
      },
      h.id,
    );
    return { operation: op, value: clone(added) };
  }

  async removeTeacherComment(commentId: string): Promise<void> {
    await this.ctx.wait(80);
    const c = this.comments.find((x) => x.id === commentId);
    if (!c) throw new BridgeError("not_found", "Комментарий не найден");
    this.assertMutable(this.hw(c.homeworkId));
    this.comments = this.comments.filter((x) => x.id !== commentId);
  }

  async getTeacherRequirements(homeworkId: string): Promise<TeacherRequirement[]> {
    await this.ctx.wait(100);
    const h = this.hw(homeworkId);
    const vid = this.latestTeacherCheckedVersion(h);
    return clone(this.requirements.filter((r) => r.checkedVersionId === vid && this.comments.some((c) => c.id === r.commentId && c.homeworkId === h.id)));
  }

  async resolveRequirement(requirementId: string, status: "done" | "not_done" | "na", note: string): Promise<TeacherRequirement> {
    await this.ctx.wait(80);
    const r = this.requirements.find((x) => x.id === requirementId);
    if (!r) throw new BridgeError("not_found", "Требование не найдено");
    const c = this.comments.find((x) => x.id === r.commentId);
    if (c) this.assertMutable(this.hw(c.homeworkId));
    r.status = status;
    r.note = note || "Проверено вручную";
    return clone(r);
  }

  // ───────────── Готовность и сдача ─────────────

  async getReadiness(homeworkId: string): Promise<Readiness> {
    await this.ctx.wait(140);
    return clone(this.readiness(this.hw(homeworkId)));
  }

  async markReadyToSubmit(homeworkId: string, options: { noTeacherComments: boolean }): Promise<OperationResult<Homework>> {
    const h = this.hw(homeworkId);
    this.assertMutable(h);
    const r = this.readiness(h);
    if (!r.ready) throw new BridgeError("blocked", `Не выполнено: ${r.checks.filter((c) => !c.ok).map((c) => c.label.toLowerCase()).join("; ")}`);
    const noComments = this.comments.every((c) => c.homeworkId !== h.id);
    if (noComments && this.ctx.rules().workflow.requireTeacherReview && !options.noTeacherComments)
      throw new BridgeError("confirmation_required", "Подтвердите, что комментариев преподавателя по этой работе нет");
    const prev = h.workflowStage;
    const op = await this.ctx.runOp(
      "mark_ready",
      "Отмечено «Готово к сдаче»",
      subjectName(h.subjectId),
      [],
      1,
      () => this.setStage(h, "ready_to_submit", "Все обязательные проверки пройдены, подтверждено пользователем"),
      () => this.setStage(h, prev, "Отметка «Готово к сдаче» отменена"),
      h.id,
    );
    return { operation: op, value: clone(h) };
  }

  async buildSubmissionPackage(homeworkId: string): Promise<OperationResult<SubmissionPackage>> {
    const h = this.hw(homeworkId);
    this.assertMutable(h);
    const r = this.readiness(h);
    if (!r.ready) throw new BridgeError("blocked", `Комплект собирается после проверок. Не выполнено: ${r.checks.filter((c) => !c.ok).map((c) => c.label.toLowerCase()).join("; ")}`);
    const v = h.versions.find((x) => x.id === h.finalVersionId)!;
    const rules = this.ctx.rules();
    const ext = v.fileName.split(".").pop() ?? "pdf";
    const base = applyNamePattern(rules.finalNamePattern, { ...rules.studentName, week: h.weekNumber, subject: subjectName(h.subjectId), version: v.label });
    const main = v.files[0] ?? null;
    const notes = v.files.find((f) => f.name.startsWith("Пояснения")) ?? null;
    const sheet = ext === "xlsx" || ext === "ipynb";
    const prev = this.packages.get(h.id);
    const items: SubmissionPackage["items"] = [
      { id: "main", role: "main", file: main, finalName: `${base}.${ext}`, required: true, present: !!main },
      { id: "notes", role: "attachment", file: notes, finalName: `${base} Пояснения.pdf`, required: true, present: !!notes },
      ...h.taskFiles.filter((f) => f.kind === "sheet").map((f, i) => ({ id: `extra-${i}`, role: "extra" as const, file: f, finalName: f.name, required: false, present: true })),
      ...(sheet ? [{ id: "cloud", role: "cloud" as const, file: main, finalName: `${base}.${ext}`, required: true, present: !!main }] : []),
    ];
    const links: SubmissionPackage["links"] = sheet ? [{ id: "cloud-link", label: "Папка курса на облачном диске", url: `https://disk.example.ru/kontur/${h.subjectId}-w${h.weekNumber}`, required: true, verified: prev?.links[0]?.verified ?? false }] : [];
    const keep = (id: PackageCheckId) => (prev && prev.versionId === v.id ? (prev.checklist.find((c) => c.id === id)?.done ?? false) : false);
    const checklist: PackageCheck[] = [
      { id: "final_selected", label: "Итоговая версия выбрана", done: true, auto: true },
      { id: "name_checked", label: "Название файла проверено", done: keep("name_checked"), auto: false },
      { id: "attachments", label: "Приложения приложены", done: items.filter((i) => i.required).every((i) => i.present), auto: true },
      { id: "links", label: "Ссылки доступны", done: links.length === 0 ? true : keep("links"), auto: links.length === 0 },
      { id: "teacher", label: "Комментарии преподавателя учтены", done: r.checks.find((c) => c.id === "teacher")?.ok ?? false, auto: true },
      { id: "manual_review", label: "Ручная проверка выполнена", done: keep("manual_review"), auto: false },
    ];
    const warnings = [
      ...items.filter((i) => i.required && !i.present).map((i) => `Нет обязательного файла: ${i.finalName}`),
      ...links.filter((l) => l.required && !l.verified).map((l) => `Проверьте доступ по ссылке «${l.label}»`),
      ...(sheet ? ["Файл больше лимита вложения — загрузите копию на облачный диск и приложите ссылку"] : []),
    ];
    const pkg: SubmissionPackage = {
      id: uid("pkg"),
      homeworkId: h.id,
      versionId: v.id,
      folderPath: `${h.folderPath}/03 На сдачу`,
      items,
      links,
      warnings,
      checklist,
      uploadList: [...items.filter((i) => i.role === "main" || i.role === "attachment").map((i) => i.finalName), ...links.map((l) => `Ссылка: ${l.url}`)],
      builtAt: new Date().toISOString(),
    };
    const op = await this.ctx.runOp(
      "build_package",
      "Комплект на сдачу собран",
      `${items.length} файлов · ${subjectName(h.subjectId)}`,
      items.map((i) => `${pkg.folderPath}/${i.finalName}`),
      3,
      () => this.packages.set(h.id, pkg),
      () => {
        if (prev) this.packages.set(h.id, prev);
        else this.packages.delete(h.id);
      },
      h.id,
    );
    return { operation: op, value: clone(pkg) };
  }

  async getSubmissionPackage(homeworkId: string): Promise<SubmissionPackage | null> {
    await this.ctx.wait(100);
    return clone(this.packages.get(homeworkId) ?? null);
  }

  async setPackageCheck(packageId: string, checkId: PackageCheckId, done: boolean): Promise<SubmissionPackage> {
    await this.ctx.wait(60);
    const pkg = [...this.packages.values()].find((p) => p.id === packageId);
    if (!pkg) throw new BridgeError("not_found", "Комплект не найден");
    this.assertMutable(this.hw(pkg.homeworkId));
    const c = pkg.checklist.find((x) => x.id === checkId);
    if (!c) throw new BridgeError("not_found", "Пункт не найден");
    if (c.auto) throw new BridgeError("conflict", "Этот пункт проверяется автоматически");
    c.done = done;
    if (checkId === "links") {
      pkg.links.forEach((l) => (l.verified = done));
      pkg.warnings = pkg.warnings.filter((w) => !w.startsWith("Проверьте доступ") || !done);
    }
    return clone(pkg);
  }

  async confirmSubmission(homeworkId: string, c: SubmissionConfirmation): Promise<OperationResult<SubmissionReceipt>> {
    const h = this.hw(homeworkId);
    this.assertMutable(h);
    if (c.userConfirmed !== true) throw new BridgeError("confirmation_required", "Нужно подтвердить, что работа отправлена вручную");
    if (h.workflowStage !== "ready_to_submit") throw new BridgeError("blocked", "Сначала отметьте работу «Готово к сдаче»");
    const pkg = this.packages.get(h.id);
    if (!pkg) throw new BridgeError("blocked", "Сначала соберите комплект на сдачу");
    const open = pkg.checklist.filter((x) => !x.done);
    if (open.length) throw new BridgeError("blocked", `Не отмечено в чек-листе: ${open.map((x) => x.label.toLowerCase()).join("; ")}`);
    const sentDir = `${h.folderPath}/04 Отправлено`;
    const now = new Date().toISOString();
    const files = pkg.items.filter((i) => i.role === "main" || i.role === "attachment").map((i) => makeFile(i.finalName, sentDir, now, i.file?.sizeBytes ?? 0));
    const proof = c.proof ? makeFile(c.proof.name, `${sentDir}/Подтверждение`, now, c.proof.sizeBytes) : null;
    const receipt: SubmissionReceipt = {
      id: uid("rcpt"),
      homeworkId: h.id,
      versionId: pkg.versionId,
      packageId: pkg.id,
      submittedAt: c.submittedAt,
      confirmedAt: now,
      assignmentUrl: c.assignmentUrl.trim(),
      proof,
      snapshotPath: sentDir,
      files,
      snapshotHash: `sha256:${[...`${pkg.id}${now}`].reduce((a, ch) => (a * 31 + ch.charCodeAt(0)) >>> 0, 7).toString(16).padStart(8, "0")}…`,
    };
    // Снимок неизменяем: операция не отменяется.
    const op = await this.ctx.runOp(
      "confirm_submission",
      "Отправка подтверждена вручную",
      `${subjectName(h.subjectId)} · ${files[0]?.name ?? ""}`,
      files.map((f) => f.relativePath),
      3,
      () => {
        this.receipts.set(h.id, receipt);
        h.stage = "submitted";
        h.submittedAt = c.submittedAt;
        h.assignmentUrl = receipt.assignmentUrl || null;
        h.finalFileName = pkg.items[0]?.finalName ?? h.finalFileName;
        this.setStage(h, "submitted", "Пользователь подтвердил отправку в систему вуза");
        this.ctx.emitter.emit("submission:confirmed", { receipt: clone(receipt) });
      },
      null,
      h.id,
    );
    return { operation: op, value: clone(receipt) };
  }

  async getSubmissionReceipt(homeworkId: string): Promise<SubmissionReceipt | null> {
    await this.ctx.wait(80);
    return clone(this.receipts.get(homeworkId) ?? null);
  }

  // ───────────── Сид сценария ─────────────

  seed(): void {
    const hws = this.ctx.homework();
    // Нет условия — показывает блокировку из-за недостающих материалов.
    const matanAdv = hws.find((h) => h.id === "hw-matanadv-8");
    if (matanAdv) matanAdv.taskFiles = [];

    const submitted = hws.find((h) => h.id === "hw-matan-6");
    if (submitted?.finalVersionId) {
      const v = submitted.versions.find((x) => x.id === submitted.finalVersionId)!;
      const name = submitted.finalFileName ?? v.fileName;
      this.receipts.set(submitted.id, {
        id: "rcpt-seed",
        homeworkId: submitted.id,
        versionId: v.id,
        packageId: "pkg-seed",
        submittedAt: submitted.submittedAt ?? at(-3),
        confirmedAt: submitted.submittedAt ?? at(-3),
        assignmentUrl: "https://lms.example.ru/course/matan/assign/6",
        proof: makeFile("Скриншот отправки.png", `${submitted.folderPath}/04 Отправлено/Подтверждение`, at(-3, 21, 41), 182_000),
        snapshotPath: `${submitted.folderPath}/04 Отправлено`,
        files: [makeFile(name, `${submitted.folderPath}/04 Отправлено`, at(-3, 21, 40), v.sizeBytes)],
        snapshotHash: "sha256:9f2c71ab…",
      });
    }

    const h = hws.find((x) => x.id === "hw-oba-7");
    if (!h) return;
    const workDir = `${h.folderPath}/02 Работа`;
    const mkVersion = (n: number, promptId: string, created: string, prevSections: HomeworkVersion["sections"] | null): HomeworkVersion => {
      const label = `v${pad2(n)}`;
      const fileName = `Работа_Неделя${h.weekNumber}_${label}.xlsx`;
      return {
        id: `${h.id}-v${n}`,
        homeworkId: h.id,
        number: n,
        label,
        fileName,
        sizeBytes: 96_000 + n * 18_000,
        createdAt: created,
        sourceFileId: uid("file"),
        note: n === 1 ? "Первое решение по промту анализа" : "Исправления по проверке v01",
        sections: buildSections(h.subjectId, prevSections),
        author: "claude",
        promptId,
        sourceBundleId: null,
        files: [makeFile(fileName, workDir, created, 96_000 + n * 18_000), makeFile(`Пояснения_${label}.pdf`, workDir, created, 61_000 + n * 4_000)],
        frozen: true,
      };
    };

    const b1 = this.buildBundle(h, "analyze_materials", null);
    b1.createdAt = at(-4, 18);
    this.bundles.set(b1.id, b1);
    const r1 = this.completedRun(h, "analyze_materials", b1, null, at(-4, 18, 5));
    const p1 = r1.result!.prompt!;
    p1.sentAt = at(-4, 18, 20);
    this.prompts.push(p1);

    const v1 = mkVersion(1, p1.id, at(-4, 19, 10), null);
    v1.sourceBundleId = b1.id;
    const v2 = mkVersion(2, "", at(-2, 20, 30), v1.sections);
    h.versions = [v1];

    const b2 = this.buildBundle(h, "review_version", v1.id);
    this.bundles.set(b2.id, b2);
    const r2 = this.completedRun(h, "review_version", b2, v1.id, at(-4, 20));
    const p2: RevisionPrompt = { ...r2.result!.prompt!, id: "prm-seed-2", createdAt: at(-4, 20, 15), sentAt: at(-3, 10) };
    this.prompts.push(p2);
    v2.promptId = p2.id;
    v2.sourceBundleId = b2.id;
    h.versions = [v1, v2];

    const b3 = this.buildBundle(h, "review_version", v2.id);
    this.bundles.set(b3.id, b3);
    // Неудачный запуск в истории: клиент закрыли во время проверки.
    this.runs.push({ ...this.completedRun(h, "review_version", b3, v2.id, at(-2, 21, 0), false), status: "failed", result: null, progress: 0.4, step: "Поиск опоры в материалах", errorMessage: "Сессия Codex прервана: клиент закрыт во время обработки", savedAsIteration: false });
    this.completedRun(h, "review_version", b3, v2.id, at(-2, 21, 30));

    const commentDate = at(-1, 13, 20);
    const seedComments: TeacherCommentInput[] = [
      { source: "chat", date: commentDate, question: "Можно ли строить сводную прямо по исходному листу, не копируя данные?", answer: "Да, но сам исходный лист не меняйте — он должен остаться как в файле задания." },
      { source: "chat", date: commentDate, question: "Нужно ли округлять средний чек?", answer: "Округлите до рублей, формат — с разделителем разрядов." },
      { source: "chat", date: commentDate, question: "Динамику по месяцам показывать на том же листе?", answer: "Нет, вынесите срез по месяцам на отдельный лист и добавьте к нему вывод в 2–3 предложения." },
    ];
    for (const c of seedComments) this.comments.push({ ...c, id: uid("tc"), homeworkId: h.id, addedAt: at(-1, 13, 30), attachment: null });
    const b4 = this.buildBundle(h, "review_teacher", v2.id);
    this.bundles.set(b4.id, b4);
    const r4 = this.completedRun(h, "review_teacher", b4, v2.id, at(-1, 14));
    const p3: RevisionPrompt = { ...r4.result!.prompt!, id: "prm-seed-3", createdAt: at(-1, 14, 10) };
    this.prompts.push(p3);
    h.workflowStage = "prompt_ready";
    h.stage = "in_progress";
  }

  // ───────────── Внутреннее ─────────────

  private hw(id: string): Homework {
    const h = this.ctx.homework().find((x) => x.id === id);
    if (!h) throw new BridgeError("not_found", "ДЗ не найдено");
    return h;
  }

  private mustRun(id: string): ReviewRun {
    const r = this.runs.find((x) => x.id === id);
    if (!r) throw new BridgeError("not_found", "Запуск не найден");
    return r;
  }

  private mustPrompt(id: string): RevisionPrompt {
    const p = this.prompts.find((x) => x.id === id);
    if (!p) throw new BridgeError("not_found", "Промт не найден");
    return p;
  }

  private assertMutable(h: Homework): void {
    if (h.stage === "submitted" || h.workflowStage === "submitted") throw new BridgeError("immutable", "Работа отправлена — снимок неизменяем");
  }

  private authorName(): string {
    return providerName(this.ctx.rules().workflow.roles.author);
  }

  private versionLabel(h: Homework, versionId: string | null): string {
    return h.versions.find((v) => v.id === versionId)?.label ?? "—";
  }

  private ensureConnected(p: ProviderId): void {
    const s = this.ctx.providers().find((x) => x.provider === p);
    if (s?.state === "reauth_required") throw new BridgeError("reauth_required", `${providerName(p)}: требуется повторный вход в приложении провайдера`);
    if (s?.state !== "connected") throw new BridgeError("provider_unavailable", `${providerName(p)} не подключён`);
  }

  private setStage(h: Homework, to: HomeworkWorkflowStage, reason: string): void {
    const from = h.workflowStage;
    if (from === to) return;
    h.workflowStage = to;
    this.ctx.emitter.emit("workflow:stage-changed", { homeworkId: h.id, from, to, reason });
  }

  private buildBundle(h: Homework, purpose: RunKind, versionId: string | null): SourceBundle {
    const week = this.ctx.weeks().get(h.subjectId)?.find((w) => w.number === h.weekNumber) ?? null;
    const kinds = KINDS_FOR[purpose];
    const items: SourceItem[] = [];
    const add = (kind: SourceKind, file: FileRef, required: boolean) => {
      if (kinds.includes(kind)) items.push({ id: `${kind}:${file.id}`, kind, file, required, included: true });
    };
    week?.lectures.forEach((f) => add("lecture", f, false));
    week?.seminars.forEach((f) => add("seminar", f, false));
    h.taskFiles.forEach((f) => add(/услов/i.test(f.name) ? "task" : "data", f, /услов/i.test(f.name)));
    week?.materials.forEach((f) => add("extra", f, false));
    add("style", { id: this.profile.id, name: this.profile.fileName, kind: "text", sizeBytes: 4_200, modifiedAt: this.profile.updatedAt, relativePath: this.ctx.rules().workflow.styleProfilePath }, true);
    previousWorks(h.subjectId, h.weekNumber).forEach((f) => add("previous_work", f, false));
    const target = versionId ? h.versions.find((v) => v.id === versionId) : h.versions[h.versions.length - 1];
    if (target) {
      target.files.forEach((f) => add("current_version", f, true));
      const prev = h.versions.find((v) => v.number === target.number - 1);
      prev?.files.slice(0, 1).forEach((f) => add("previous_version", f, false));
    }
    const comments = this.comments.filter((c) => c.homeworkId === h.id);
    if (comments.length) add("teacher_comments", { id: `tc-${h.id}`, name: `Комментарии преподавателя (${comments.length}).md`, kind: "text", sizeBytes: 900 * comments.length, modifiedAt: comments[comments.length - 1]!.addedAt, relativePath: `${h.folderPath}/.kontur/teacher.md` }, true);
    const bundle: SourceBundle = { id: uid("bnd"), homeworkId: h.id, purpose, versionId: target?.id ?? null, items, missing: [], complete: true, createdAt: new Date().toISOString() };
    this.recomputeMissing(bundle);
    return bundle;
  }

  private recomputeMissing(b: SourceBundle): void {
    const kinds = KINDS_FOR[b.purpose];
    const has = (k: SourceKind) => b.items.some((i) => i.kind === k && i.included);
    const missing: MissingSource[] = [];
    if (kinds.includes("task") && !has("task")) missing.push({ kind: "task", label: "Условие ДЗ", required: true, hint: "Загрузите условие во «Входящие» с типом «Условие ДЗ»" });
    if (!has("lecture") && !has("seminar")) missing.push({ kind: "lecture", label: "Лекции или семинары недели", required: true, hint: "Без материалов курса нельзя проверить опору на лекции" });
    if (kinds.includes("style") && !has("style")) missing.push({ kind: "style", label: "Файл «Мой стиль»", required: true, hint: `Укажите путь к ${STYLE_PROFILE_FILE} в настройках` });
    if (kinds.includes("current_version") && !has("current_version")) missing.push({ kind: "current_version", label: "Проверяемая версия", required: true, hint: "Создайте версию в «02 Работа»" });
    if (kinds.includes("teacher_comments") && b.purpose === "review_teacher" && !has("teacher_comments")) missing.push({ kind: "teacher_comments", label: "Комментарии преподавателя", required: true, hint: "Вставьте переписку или добавьте вопрос и ответ" });
    if (kinds.includes("previous_work") && !has("previous_work")) missing.push({ kind: "previous_work", label: "Предыдущие работы", required: false, hint: "Без них оценка стиля будет менее точной" });
    if (kinds.includes("data") && !has("data") && b.items.some((i) => i.kind === "data")) missing.push({ kind: "data", label: "Данные к заданию исключены", required: false, hint: "Решение может не сойтись с данными" });
    b.missing = missing;
    b.complete = missing.every((m) => !m.required);
  }

  private async startRun(homeworkId: string, kind: RunKind, bundleId: string, versionId: string | null): Promise<ReviewRun> {
    const h = this.hw(homeworkId);
    this.assertMutable(h);
    const bundle = this.bundles.get(bundleId);
    if (!bundle || bundle.homeworkId !== homeworkId) throw new BridgeError("not_found", "Набор источников не найден — подготовьте его заново");
    if (!bundle.complete) throw new BridgeError("blocked", `Не хватает обязательных материалов: ${bundle.missing.filter((m) => m.required).map((m) => m.label).join(", ")}`);
    if (kind !== "analyze_materials" && !h.versions.some((v) => v.id === versionId)) throw new BridgeError("not_found", "Версия не найдена");
    if (this.runs.some((r) => r.homeworkId === homeworkId && (r.status === "running" || r.status === "queued"))) throw new BridgeError("conflict", "По этой работе уже идёт запуск — дождитесь или отмените его");
    const provider = this.ctx.rules().workflow.roles[ROLE_OF[kind]];
    this.ensureConnected(provider);
    await this.ctx.wait(120);
    const run = this.newRun(h, kind, bundle, versionId, new Date().toISOString());
    this.runs.push(run);
    const prevStage = h.workflowStage;
    const stageFor: Partial<Record<RunKind, HomeworkWorkflowStage>> = { analyze_materials: "codex_analysis", review_version: "codex_review", review_teacher: "teacher_review" };
    const to = stageFor[kind];
    if (to) this.setStage(h, to, `${providerName(provider)}: ${STEPS[kind][0]!.toLowerCase()}`);
    const op = this.ctx.startOp("ai_run", `${providerName(provider)}: ${RUN_TITLE[kind]}`, `${subjectName(h.subjectId)}${versionId ? ` · ${this.versionLabel(h, versionId)}` : ""}`, [], h.id);
    this.runOps.set(run.id, { op, prevStage });
    void this.execute(run, h, provider, op, prevStage);
    return clone(run);
  }

  private async execute(run: ReviewRun, h: Homework, provider: ProviderId, op: Operation, prevStage: HomeworkWorkflowStage): Promise<void> {
    const steps = STEPS[run.kind];
    await this.ctx.wait(250);
    if (isCancelled(run)) return;
    run.status = "running";
    for (let i = 0; i < steps.length; i += 1) {
      await this.ctx.wait(480);
      if (isCancelled(run)) return;
      const s = this.ctx.providers().find((x) => x.provider === provider);
      if (s?.state !== "connected") {
        run.status = "failed";
        run.finishedAt = new Date().toISOString();
        run.errorMessage = `${providerName(provider)} отключился во время обработки`;
        this.setStage(h, prevStage, "Запуск прерван");
        const error = { code: "provider_unavailable" as const, message: run.errorMessage };
        this.ctx.failOp(op, error);
        this.ctx.emitter.emit("review:failed", { run: clone(run), error });
        return;
      }
      run.progress = (i + 1) / steps.length;
      run.step = steps[i]!;
      this.ctx.emitter.emit("review:progress", { runId: run.id, homeworkId: h.id, progress: run.progress, step: run.step });
      this.ctx.emitter.emit("operation:progress", { operationId: op.id, kind: "ai_run", progress: run.progress, message: run.step });
    }
    const bundle = this.bundles.get(run.sourceBundleId)!;
    run.result = this.computeResult(h, run, bundle);
    run.status = "completed";
    run.finishedAt = new Date().toISOString();
    this.applyStageAfter(h, run);
    this.ctx.finishOp(op);
    this.ctx.emitter.emit("review:completed", { run: clone(run) });
  }

  /** Синхронно создаёт завершённый запуск (для сида). */
  private completedRun(h: Homework, kind: RunKind, bundle: SourceBundle, versionId: string | null, startedAt: string, push = true): ReviewRun {
    const run = this.newRun(h, kind, bundle, versionId, startedAt);
    run.status = "completed";
    run.progress = 1;
    run.step = STEPS[kind][STEPS[kind].length - 1]!;
    run.finishedAt = startedAt;
    run.savedAsIteration = true;
    run.result = this.computeResult(h, run, bundle);
    if (run.result.prompt) run.result.prompt.createdAt = startedAt;
    if (push) this.runs.push(run);
    return run;
  }

  private newRun(h: Homework, kind: RunKind, bundle: SourceBundle, versionId: string | null, startedAt: string): ReviewRun {
    return {
      id: uid("run"),
      homeworkId: h.id,
      kind,
      role: ROLE_OF[kind],
      provider: this.ctx.rules().workflow.roles[ROLE_OF[kind]],
      versionId,
      sourceBundleId: bundle.id,
      sourcesUsed: bundle.items.filter((i) => i.included).map((i) => ({ sourceId: i.id, fileName: i.file.name, locator: SOURCE_KIND_LABEL[i.kind].toLowerCase() })),
      status: "queued",
      progress: 0,
      step: "В очереди",
      startedAt,
      finishedAt: null,
      savedAsIteration: false,
      result: null,
      errorMessage: null,
    };
  }

  private applyStageAfter(h: Homework, run: ReviewRun): void {
    const r = run.result!;
    const v = this.versionLabel(h, run.versionId);
    if (run.kind === "analyze_materials") {
      if (r.prompt) this.prompts.push(clone(r.prompt));
      this.setStage(h, "prompt_ready", `Промт для ${this.authorName()} создан по ${run.sourcesUsed.length} источникам`);
    } else if (run.kind === "review_version") {
      if (r.verdict === "needs_fixes") this.setStage(h, "needs_fixes", `Проверка ${v}: критических ошибок ${r.critical.length}, расхождение со стилем ${r.style?.score ?? "—"}/10`);
      else this.setStage(h, "teacher_review", `Проверка ${v} пройдена — сверьте с комментариями преподавателя`);
    } else if (run.kind === "review_teacher") {
      const notDone = r.teacherRequirements.filter((x) => x.status === "not_done").length;
      if (notDone) this.setStage(h, "needs_fixes", `Не выполнено требований преподавателя: ${notDone}`);
    }
  }

  private computeResult(h: Homework, run: ReviewRun, bundle: SourceBundle): ReviewResult {
    const included = bundle.items.filter((i) => i.included);
    const pick = (k: SourceKind) => included.find((i) => i.kind === k)?.file ?? null;
    const refs = { task: pick("task"), lecture: pick("lecture"), seminar: pick("seminar") };
    const threshold = this.ctx.rules().workflow.styleThreshold;
    const sources = included.map((i) => ({ sourceId: i.id, fileName: i.file.name, locator: SOURCE_KIND_LABEL[i.kind].toLowerCase() }));
    const sheet = h.taskFiles.some((f) => f.kind === "sheet") || h.versions.some((v) => v.fileName.endsWith(".xlsx") || v.fileName.endsWith(".ipynb"));

    if (run.kind === "analyze_materials") {
      const prompt: RevisionPrompt = {
        id: uid("prm"),
        homeworkId: h.id,
        kind: "initial",
        text: this.initialPromptText(h, included, sheet),
        createdAt: new Date().toISOString(),
        createdBy: run.provider,
        basedOnVersionId: null,
        reviewRunId: run.id,
        addresses: [],
        sources,
        sentAt: null,
      };
      return {
        verdict: "pass",
        summary: `Материалы разобраны: ${included.length} источников. Сформирован первичный промт для ${this.authorName()}.`,
        criteria: [
          { id: "completeness", label: "Комплектность материалов", status: bundle.missing.length ? "warn" : "pass", note: bundle.missing.length ? bundle.missing.map((m) => m.label).join(", ") : "Все материалы на месте" },
        ],
        critical: [],
        remarks: [],
        unsupportedClaims: [],
        style: null,
        teacherRequirements: [],
        recommendedAction: { kind: "send_prompt_to_author", label: `Передать промт ${this.authorName()}` },
        prompt,
      };
    }

    const v = h.versions.find((x) => x.id === run.versionId)!;

    if (run.kind === "review_teacher") {
      const comments = this.comments.filter((c) => c.homeworkId === h.id);
      const reqs: TeacherRequirement[] = comments.map((c, i) => {
        const isLast = i === comments.length - 1;
        const status: TeacherRequirement["status"] = c.source === "in_person" ? "manual" : /по желанию|необязательно/i.test(c.answer) ? "na" : v.number <= 2 && isLast ? "not_done" : "done";
        const origin = (c.question || c.answer).slice(0, 70);
        return {
          id: uid("req"),
          text: firstSentence(c.answer || c.question),
          commentId: c.id,
          origin: origin.length < (c.question || c.answer).length ? `${origin}…` : origin,
          status,
          note: status === "done" ? `Выполнено в ${v.label}` : status === "not_done" ? `В ${v.label} не найдено` : status === "manual" ? "Устный комментарий — проверьте сами" : "Не обязательно для сдачи",
          checkedVersionId: v.id,
        };
      });
      this.requirements = [...this.requirements.filter((r) => r.checkedVersionId !== v.id), ...reqs];
      const notDone = reqs.filter((r) => r.status === "not_done");
      const manual = reqs.filter((r) => r.status === "manual");
      return {
        verdict: notDone.length ? "needs_fixes" : "pass",
        summary: notDone.length ? `Не выполнено требований: ${notDone.length} из ${reqs.length}.` : `Все требования преподавателя учтены${manual.length ? `, ${manual.length} — на ручную проверку` : ""}.`,
        criteria: [{ id: "teacher", label: "Требования преподавателя", status: notDone.length ? "fail" : manual.length ? "manual" : "pass", note: `${reqs.length - notDone.length - manual.length} из ${reqs.length} выполнено` }],
        critical: [],
        remarks: [],
        unsupportedClaims: [],
        style: null,
        teacherRequirements: reqs,
        recommendedAction: notDone.length ? { kind: "create_revision_prompt", label: "Создать промт исправлений по комментариям" } : { kind: "mark_ready", label: "Перейти к комплекту на сдачу" },
        prompt: notDone.length ? this.revisionPrompt(h, v, run, [], [], null, notDone, sources) : null,
      };
    }

    // review_version / revision_prompt
    const registry = this.findings.get(h.id) ?? [];
    this.findings.set(h.id, registry);
    const open: ReviewFinding[] = [];
    for (const sf of scriptFor(sheet)) {
      let f = registry.find((x) => x.id === `${h.id}-${sf.key}`);
      if (v.number >= sf.from && !f) {
        f = { id: `${h.id}-${sf.key}`, severity: sf.severity, criterion: sf.criterion, text: sf.text, location: sf.location, evidence: sf.evidence(refs), introducedIn: v.id, fixedIn: null };
        registry.push(f);
      }
      if (!f) continue;
      if (v.number >= sf.until) {
        if (!f.fixedIn) f.fixedIn = v.id;
      } else if (v.number >= sf.from) open.push(clone(f));
    }
    const unsupported: UnsupportedClaim[] =
      v.number <= 1 ? [{ id: `${h.id}-U1`, text: sheet ? "«Сезонность продаж связана с праздниками» — этого нет ни в данных, ни в лекции" : "«Этот метод всегда быстрее» — в материалах не обосновано", location: sheet ? "лист «Выводы», абзац 3" : "задача 1, комментарий" }] : [];
    const style = styleFor(v.number, threshold);
    const critical = open.filter((f) => f.severity === "critical");
    const remarks = open.filter((f) => f.severity === "normal");
    const teacherReqs = this.requirements.filter((r) => r.checkedVersionId === v.id);
    const comments = this.comments.filter((c) => c.homeworkId === h.id);
    const teacherStatus: CriterionStatus = comments.length === 0 ? "manual" : teacherReqs.length === 0 ? "manual" : teacherReqs.some((r) => r.status === "not_done") ? "fail" : teacherReqs.some((r) => r.status === "manual") ? "manual" : "pass";
    const criteria: ReviewCriterion[] = [
      { id: "correctness", label: "Корректность решения", status: critical.some((f) => f.criterion === "correctness") ? "fail" : "pass", note: critical.some((f) => f.criterion === "correctness") ? "Есть ошибка в расчёте" : "Расчёты сходятся с данными" },
      { id: "task_fit", label: "Соответствие заданию", status: critical.some((f) => f.criterion === "task_fit") ? "fail" : "pass", note: critical.some((f) => f.criterion === "task_fit") ? "Пункт задания выполнен не так" : "Все пункты условия покрыты" },
      { id: "course_grounding", label: "Опора на материалы курса", status: unsupported.length ? "warn" : "pass", note: unsupported.length ? `Неподтверждённых утверждений: ${unsupported.length}` : "Методы взяты из лекций и семинаров" },
      { id: "style", label: "Соответствие личному стилю", status: style.score > threshold ? "fail" : "pass", note: `Расхождение с личным стилем: ${style.score}/10 (порог ${threshold})` },
      { id: "teacher", label: "Требования преподавателя", status: teacherStatus, note: comments.length === 0 ? "Комментарии не добавлены" : teacherReqs.length ? `${teacherReqs.filter((r) => r.status === "done" || r.status === "na").length} из ${teacherReqs.length} выполнено` : "Не сверено с этой версией" },
      { id: "completeness", label: "Комплектность файлов", status: remarks.some((f) => f.criterion === "completeness") ? "warn" : v.files.length >= 2 ? "pass" : "warn", note: v.files.length >= 2 ? `${v.files.length} файла в версии` : "Нет файла пояснений" },
    ];
    const needsFixes = critical.length > 0 || style.score > threshold;
    const teacherNotDone = teacherReqs.filter((r) => r.status === "not_done");
    const wantPrompt = needsFixes || (run.kind === "revision_prompt" && (remarks.length > 0 || unsupported.length > 0 || teacherNotDone.length > 0));
    const verdict = needsFixes ? "needs_fixes" : teacherStatus === "pass" ? "pass" : "needs_teacher_review";
    return {
      verdict,
      summary: needsFixes
        ? `Найдено критических ошибок: ${critical.length}. Расхождение с личным стилем ${style.score}/10 при пороге ${threshold}.`
        : remarks.length
          ? `Критических ошибок нет, осталось замечаний: ${remarks.length}. Расхождение со стилем ${style.score}/10.`
          : `Проверка пройдена. Расхождение со стилем ${style.score}/10.`,
      criteria,
      critical,
      remarks,
      unsupportedClaims: unsupported,
      style,
      teacherRequirements: teacherReqs,
      recommendedAction: needsFixes
        ? { kind: "create_revision_prompt", label: `Создать промт исправлений для ${this.authorName()}` }
        : verdict === "needs_teacher_review"
          ? { kind: "add_teacher_comments", label: "Добавить комментарии преподавателя и сверить" }
          : { kind: "mark_ready", label: "Перейти к комплекту на сдачу" },
      prompt: wantPrompt ? this.revisionPrompt(h, v, run, critical, unsupported, style, run.kind === "revision_prompt" ? teacherNotDone : [], sources, remarks) : null,
    };
  }

  private initialPromptText(h: Homework, items: SourceItem[], sheet: boolean): string {
    const by = (k: SourceKind) => items.filter((i) => i.kind === k).map((i) => i.file.name);
    const line = (label: string, names: string[]) => (names.length ? [`— ${label}: ${names.join(", ")}`] : []);
    return [
      `Выполни домашнюю работу по курсу «${subjectName(h.subjectId)}», неделя ${h.weekNumber}: «${h.title}».`,
      "",
      "Источники — используй только их:",
      ...line("условие", by("task")),
      ...line("данные", by("data")),
      ...line("лекции", by("lecture")),
      ...line("семинары", by("seminar")),
      ...line("доп. материалы", by("extra")),
      ...line("мой стиль", by("style")),
      ...line("мои прошлые работы (образец стиля)", by("previous_work")),
      "",
      "Требования:",
      "1. Решай строго по условию; каждый пункт — отдельный раздел с тем же номером.",
      "2. Методы и формулы — только из лекций и семинаров этой недели. Указывай, откуда взят метод (файл, слайд или задача).",
      "3. Не добавляй утверждений, которых нет в материалах или в данных.",
      `4. Пиши в моём стиле: ${this.profile.traits.slice(0, 3).join("; ")}. Сверяйся с прошлыми работами.`,
      sheet
        ? "5. Excel: исходный лист с данными не изменяй, расчёты — на отдельных листах, формулы видимы, у диаграмм подписаны оси и единицы."
        : "5. Оформление: решение по задачам, итоговый ответ в конце каждой, формулы — в LaTeX.",
      "",
      "Результат: файл решения и короткие пояснения (PDF) по каждому пункту.",
    ].join("\n");
  }

  private revisionPrompt(
    h: Homework,
    v: HomeworkVersion,
    run: ReviewRun,
    critical: ReviewFinding[],
    unsupported: UnsupportedClaim[],
    style: StyleDivergence | null,
    reqs: TeacherRequirement[],
    sources: EvidenceReference[],
    remarks: ReviewFinding[] = [],
  ): RevisionPrompt {
    const fmt = (f: ReviewFinding, i: number) => `${i + 1}. ${f.text}${f.location ? ` (${f.location})` : ""}.${f.evidence.length ? ` Опора: ${f.evidence.map((e) => `${e.fileName}, ${e.locator}`).join("; ")}.` : ""}`;
    const same = v.sections.slice(0, 2).map((s) => s.title);
    const text = [
      `Исправь ${v.label} домашней работы «${h.title}» и создай новую версию. Предыдущую версию не изменяй.`,
      ...(critical.length ? ["", "Критические ошибки:", ...critical.map(fmt)] : []),
      ...(remarks.length ? ["", "Замечания:", ...remarks.map(fmt)] : []),
      ...(unsupported.length ? ["", "Утверждения без опоры — удали или подтверди ссылкой на материал:", ...unsupported.map((u, i) => `${i + 1}. ${u.text} (${u.location})`)] : []),
      ...(style && style.score > style.threshold
        ? ["", `Стиль: расхождение с моим стилем ${style.score}/10 при пороге ${style.threshold}. Убери:`, ...style.reasons.map((r) => `— ${STYLE_REASON_LABEL[r.kind].toLowerCase()}${r.example ? `: ${r.example}` : ""}`)]
        : []),
      ...(reqs.length ? ["", "Требования преподавателя, которые не выполнены:", ...reqs.map((r, i) => `${i + 1}. ${r.text} (из вопроса: ${r.origin})`)] : []),
      "",
      same.length ? `Не трогай то, что уже верно: ${same.join(", ")}.` : "Не трогай то, что уже верно.",
      "Источники те же, что в пакете; новые утверждения — только со ссылкой на материал.",
    ].join("\n");
    return {
      id: uid("prm-draft"),
      homeworkId: h.id,
      kind: reqs.length ? "teacher" : "revision",
      text,
      createdAt: new Date().toISOString(),
      createdBy: run.provider,
      basedOnVersionId: v.id,
      reviewRunId: run.id,
      addresses: [...critical.map((f) => f.id), ...remarks.map((f) => f.id), ...unsupported.map((u) => u.id), ...reqs.map((r) => r.id)],
      sources,
      sentAt: null,
    };
  }

  private latestRun(hwId: string, versionId: string, kind: RunKind): ReviewRun | null {
    return [...this.runs].reverse().find((r) => r.homeworkId === hwId && r.versionId === versionId && r.kind === kind && r.status === "completed") ?? null;
  }

  /** Закреплённый (сохранённый в истории) запуск; если нет — последний завершённый. */
  private pinnedRun(hwId: string, versionId: string, kind: RunKind): ReviewRun | null {
    const done = this.runs.filter((r) => r.homeworkId === hwId && r.versionId === versionId && r.kind === kind && r.status === "completed");
    return [...done].reverse().find((r) => r.savedAsIteration) ?? done[done.length - 1] ?? null;
  }

  private latestTeacherCheckedVersion(h: Homework): string | null {
    const run = [...this.runs].reverse().find((r) => r.homeworkId === h.id && r.kind === "review_teacher" && r.status === "completed");
    return run?.versionId ?? null;
  }

  private readiness(h: Homework): Readiness {
    const rules = this.ctx.rules();
    const target = h.versions.find((v) => v.id === h.finalVersionId) ?? h.versions[h.versions.length - 1] ?? null;
    const materials = this.buildBundle(h, "analyze_materials", null);
    const saved = target ? [...this.runs].reverse().find((r) => r.homeworkId === h.id && r.versionId === target.id && r.kind === "review_version" && r.status === "completed" && r.savedAsIteration) ?? null : null;
    const res = saved?.result ?? null;
    const comments = this.comments.filter((c) => c.homeworkId === h.id);
    const lastComment = comments.map((c) => c.addedAt).sort().pop() ?? "";
    const teacherRun = target ? [...this.runs].reverse().find((r) => r.homeworkId === h.id && r.versionId === target.id && r.kind === "review_teacher" && r.status === "completed") ?? null : null;
    const reqs = target ? this.requirements.filter((r) => r.checkedVersionId === target.id) : [];
    const pendingReqs = reqs.filter((r) => r.status === "not_done" || r.status === "manual");
    const teacherOk = comments.length === 0 ? true : !!teacherRun && teacherRun.startedAt >= lastComment && pendingReqs.length === 0;
    const score = res?.style?.score ?? null;
    const checks: ReadinessCheck[] = [
      { id: "materials", label: "Обязательные материалы собраны", ok: materials.complete, detail: materials.complete ? "Условие, материалы курса и файл стиля на месте" : `Нет: ${materials.missing.filter((m) => m.required).map((m) => m.label).join(", ")}` },
      { id: "review_passed", label: "Проверка версии пройдена и сохранена", ok: !!res && res.verdict !== "needs_fixes" && res.verdict !== "blocked", detail: !target ? "Версий нет" : !saved ? `Проверка ${target.label} не сохранена в истории итераций` : res && res.verdict !== "needs_fixes" ? `${target.label} прошла проверку` : `${target.label} требует исправлений` },
      { id: "no_critical", label: "Нет критических ошибок", ok: !!res && res.critical.length === 0, detail: res ? `Критических: ${res.critical.length}, замечаний: ${res.remarks.length}` : "Нет результата проверки" },
      { id: "style", label: `Расхождение со стилем не выше ${rules.workflow.styleThreshold}/10`, ok: score !== null && score <= rules.workflow.styleThreshold, detail: score === null ? "Нет оценки" : `Сейчас ${score}/10` },
      {
        id: "teacher",
        label: "Комментарии преподавателя учтены",
        ok: teacherOk,
        detail: comments.length === 0 ? "Комментариев нет — потребуется подтверждение" : !teacherRun || teacherRun.startedAt < lastComment ? "Есть комментарии, не сверенные с этой версией" : pendingReqs.length ? `Требований не закрыто: ${pendingReqs.length}` : `Все ${reqs.length} требования выполнены`,
      },
      { id: "final_version", label: "Итоговой назначена проверенная версия", ok: !!target && h.finalVersionId === target.id, detail: !target ? "Версий нет" : h.finalVersionId === target.id ? `${target.label} — итоговая` : `Сделайте ${target.label} итоговой` },
    ];
    return { homeworkId: h.id, ready: checks.every((c) => c.ok), checks };
  }
}

const RUN_TITLE: Record<RunKind, string> = {
  analyze_materials: "анализ материалов",
  review_version: "проверка версии",
  revision_prompt: "промт исправлений",
  review_teacher: "проверка по комментариям",
};
