import { TypedEmitter } from "../emitter";
import { MockAdoption } from "./adoption";
import {
  BridgeError,
  type AutomationRules,
  type BridgeEventName,
  type BridgeListener,
  type Classification,
  type Dashboard,
  type DesktopBridge,
  type DroppedItem,
  type FileRef,
  type Homework,
  type HomeworkVersion,
  type ImportItemReport,
  type InboxFile,
  type InboxImportReport,
  type ManagedRoots,
  type OperationRecord,
  type RootSelection,
  type SelectableRootKind,
  type MaterialCategory,
  type Operation,
  type OperationKind,
  type OperationResult,
  type ProviderId,
  type ProviderStatus,
  type IsolationReport,
  type RouteDestination,
  type RoutePreview,
  type NewHomeworkInput,
  type Semester,
  type SemesterInput,
  type SemesterSaveResult,
  type Subject,
  type SubjectInput,
  type SubjectMergePreview,
  type SubjectMergeResult,
  type Unsubscribe,
  type Week,
  type WorkflowCommandOptions,
  type Workspace,
  type WorkspaceActivation,
  type WorkspaceOverview,
  type WorkspaceTransition,
  type WorkspaceTransitionPreview,
  type WorkspaceTransitionRequest,
} from "../types";
import {
  CATEGORY_FOLDER,
  CURRENT_WEEK,
  DEMO_DROP,
  SUBJECTS,
  at,
  buildAllWeeks,
  buildHomework,
  buildInbox,
  buildProviders,
  buildRules,
  buildSections,
  finalName,
  kindFromName,
  makeFile,
  mockAvailability,
  nextFileId,
  pad2,
  subjectName,
  SEMESTER_START,
  weekFolder,
  weekRange,
} from "./data";
import { AI_TASKS } from "../ai-types";
import { plural } from "../../format";
import { validateNamePattern } from "../../naming";
import { lmsUrlProblem } from "../../lmsLinks";
import { newOperationId } from "../intake";
import { MockWorkflow, type MockContext } from "./workflow";
import { MockLms } from "./lms";
import { MockExtraction } from "./extraction";

/**
 * MockDesktopBridge — имитация desktop-backend.
 * Хранит состояние в памяти, эмулирует задержки, прогресс и ошибки.
 * Ничего не пишет на диск, не ходит в сеть, не хранит секреты.
 */

export const clone = <T,>(v: T): T => structuredClone(v);

const DEMO_ROOT_PATHS: Record<SelectableRootKind, string> = {
  vault: "~/Documents/Учёба",
  backup: "~/Documents/Контур — резервные копии",
  downloads: "~/Downloads",
};

function buildRoots(): ManagedRoots {
  return {
    workspace: {
      id: "demo-browser-workspace",
      kind: "demo",
      vaultCanonical: DEMO_ROOT_PATHS.vault,
      vaultFingerprint: "browser-demo-fingerprint",
      createdAt: "2026-01-01T00:00:00.000Z",
      state: "active",
    },
    vault: { kind: "vault", state: "ok", displayPath: DEMO_ROOT_PATHS.vault, message: null },
    inbox: { kind: "inbox", state: "ok", displayPath: `${DEMO_ROOT_PATHS.vault}/Предметы/00 Входящие`, message: null },
    backup: { kind: "backup", state: "not_configured", displayPath: null, message: "Не выбрана" },
    downloads: { kind: "downloads", state: "not_configured", displayPath: null, message: "Не выбрана — нужна для наблюдения за загрузками" },
    importReady: true,
  };
}

function numberedMockName(name: string, index: number): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? `${name.slice(0, dot)} (${index})${name.slice(dot)}` : `${name} (${index})`;
}

export interface MockOptions {
  /** Множитель задержек: 1 — реалистично, 0 — мгновенно (тесты). */
  latency?: number;
}

let opSeq = 0;

export class MockDesktopBridge implements DesktopBridge {
  readonly kind = "mock" as const;

  private emitter = new TypedEmitter();
  private subjects: Subject[] = clone(SUBJECTS);
  private subjectSeq = 0;
  private weeks = buildAllWeeks();
  private inbox: InboxFile[] = buildInbox();
  private homework: Homework[] = buildHomework();
  private providers: ProviderStatus[] = buildProviders();
  private rules: AutomationRules = buildRules();
  private log: Operation[] = [];
  private undoers = new Map<string, () => void>();
  private homeworkCandidates = new Map<string, FileRef[]>();
  private roots: ManagedRoots = buildRoots();
  private knownWorkspaces: Workspace[] = this.roots.workspace ? [clone(this.roots.workspace)] : [];
  private transitions: WorkspaceTransition[] = [];
  private pendingTransitions = new Map<string, WorkspaceTransitionRequest>();
  private transitionSeq = 0;
  private importResults = new Map<string, OperationResult<InboxImportReport>>();
  private codexAttempts = 0;
  private candidateSeq = 0;
  private readonly latency: number;
  private readonly workflow: MockWorkflow;
  private readonly lms: MockLms;
  private readonly extraction: MockExtraction;
  private readonly adoption = new MockAdoption((ms) => this.wait(ms));

  constructor(options: MockOptions = {}) {
    this.latency = options.latency ?? 1;
    this.workflow = new MockWorkflow(this.context());
    this.lms = new MockLms({
      emitter: this.emitter,
      wait: (ms) => this.wait(ms),
      roots: () => this.roots,
      subjects: () => this.subjects,
      inbox: () => this.inbox,
      addInbox: (files) => {
        this.inbox = [...files, ...this.inbox];
      },
      log: (op) => {
        this.log.unshift(op);
      },
    });
    this.extraction = new MockExtraction({
      emitter: this.emitter,
      wait: (ms) => this.wait(ms),
      inbox: () => this.inbox.filter((f) => f.status !== "routed"),
      weekFiles: () => {
        const short = (id: string) => this.subjects.find((s) => s.id === id)?.shortName ?? id;
        const files: { file: FileRef; context: string }[] = [];
        for (const [subjectId, list] of this.weeks)
          for (const w of list) {
            const base = `${short(subjectId)} · Неделя ${w.number}`;
            files.push(...w.lectures.map((file) => ({ file, context: `${base} · Лекции` })));
            files.push(...w.seminars.map((file) => ({ file, context: `${base} · Семинары` })));
            files.push(...w.materials.map((file) => ({ file, context: `${base} · Дополнительно` })));
          }
        for (const h of this.homework) files.push(...h.taskFiles.map((file) => ({ file, context: `${short(h.subjectId)} · Неделя ${h.weekNumber} · ДЗ · условие и данные` })));
        return files;
      },
      homework: () => this.homework,
      log: (op) => {
        this.log.unshift(op);
      },
    });
    // Привязываем ДЗ к неделям.
    for (const h of this.homework) {
      const w = this.weeks.get(h.subjectId)?.find((x) => x.number === h.weekNumber);
      if (w && !w.homeworkIds.includes(h.id)) w.homeworkIds.push(h.id);
    }
    // Стартовая история, чтобы журнал не был пустым.
    this.log = [
      this.seedOp("route", "Разложено 3 файла", "Математический анализ / Неделя 08", -0.08, true),
      this.seedOp("create_version", "Создана версия v02", "Основы бизнес-аналитики · ДЗ недели 7", -0.6, false),
      this.seedOp("mark_final", "v03 назначена итоговой", "Статистика · ДЗ недели 7", -1.1, false),
      this.seedOp("mark_submitted", "ДЗ отмечено отправленным", "Математический анализ · Неделя 6", -3, false),
    ];
    this.inbox = this.inbox.map((f) => this.applyClassification(f, this.classify(f.name)));
    this.workflow.seed();
  }

  // ───────────── События ─────────────

  on<E extends BridgeEventName>(event: E, listener: BridgeListener<E>): Unsubscribe {
    return this.emitter.on(event, listener);
  }

  // ───────────── Главная и структура ─────────────

  async getDashboard(): Promise<Dashboard> {
    await this.wait((260));
    const active = this.homework.filter((h) => h.stage !== "submitted");
    // Как и backend: ДЗ без известного срока в ленту сроков не попадают.
    const deadlines = active
      .flatMap((h) => (h.deadline ? [{ homeworkId: h.id, subjectId: h.subjectId, title: h.title, deadline: h.deadline, stage: h.stage }] : []))
      .sort((a, b) => a.deadline.localeCompare(b.deadline));
    const allFiles: FileRef[] = [];
    for (const list of this.weeks.values())
      for (const w of list) allFiles.push(...w.lectures, ...w.seminars, ...w.materials);
    for (const h of this.homework)
      for (const v of h.versions)
        allFiles.push({ id: v.id, name: v.fileName, kind: kindFromName(v.fileName), sizeBytes: v.sizeBytes, modifiedAt: v.createdAt, relativePath: `${h.folderPath}/02 Работа/${v.fileName}` });
    const recentFiles = allFiles.sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt)).slice(0, 6);
    const pending = this.inbox.filter((f) => f.status !== "routed");
    const weekItems = this.homework
      .filter((h) => h.weekNumber >= CURRENT_WEEK - 1)
      .map((h) => ({ label: `${subjectName(h.subjectId)} — ДЗ ${h.weekNumber}`, done: h.stage === "submitted" || h.stage === "ready" }));
    weekItems.push({ label: "Входящие разобраны", done: pending.length === 0 });
    const range = weekRange(CURRENT_WEEK);
    return clone({
      today: new Date().toISOString(),
      academicWeek: { number: CURRENT_WEEK, ...range },
      inbox: { total: pending.length, ambiguous: pending.filter((f) => f.status === "ambiguous").length, preview: pending.slice(0, 4) },
      activeHomework: active,
      deadlines,
      recentFiles,
      automation: { state: "active", watchedPath: "~/Downloads", lastRunAt: this.log[0]?.startedAt ?? at(0), pending: pending.length },
      providers: this.providers,
      weekProgress: { done: weekItems.filter((i) => i.done).length, total: weekItems.length, items: weekItems },
    });
  }

  async getSubjects(): Promise<Subject[]> {
    await this.wait((120));
    // Как в backend: ключевые слова — у предмета, правила автоматизации их отражают.
    return clone(this.subjects.map((s) => ({ ...s, keywords: this.keywordsOf(s.id) })));
  }

  // ───────────── Предметы (имитация этапа 3) ─────────────

  private keywordsOf(subjectId: string): string[] {
    return [...(this.rules.subjectRules.find((r) => r.subjectId === subjectId)?.keywords ?? [])].sort();
  }

  private setKeywords(subjectId: string, keywords: string[]): void {
    const others = this.rules.subjectRules.filter((r) => r.subjectId !== subjectId);
    this.rules = { ...this.rules, subjectRules: keywords.length ? [...others, { subjectId, keywords }] : others };
  }

  private normalizeSubject(input: SubjectInput, exceptId: string | null): { name: string; shortName: string; keywords: string[] } {
    const name = input.name.split(/\s+/).filter(Boolean).join(" ");
    if (!name) throw new BridgeError("blocked", "Укажите название предмета");
    if (name.startsWith(".") || name.startsWith("00 ") || /[/\\:]/.test(name)) throw new BridgeError("blocked", "Название не должно начинаться с точки или «00 » и содержать / \\ :");
    const shortName = input.shortName.trim();
    if (!shortName || shortName.length > 12) throw new BridgeError("blocked", "Короткое название: от 1 до 12 символов");
    if (this.subjects.some((s) => s.id !== exceptId && s.name.toLowerCase() === name.toLowerCase())) throw new BridgeError("conflict", `Предмет «${name}» уже существует`);
    const keywords = [...new Set(input.keywords.map((k) => k.split(/\s+/).filter(Boolean).join(" ").toLowerCase()).filter(Boolean))].sort();
    if (keywords.some((k) => k.length < 2 || k.length > 60)) throw new BridgeError("blocked", "Ключевое слово: от 2 до 60 символов");
    this.ensureKeywordsFree(exceptId, keywords);
    return { name, shortName, keywords };
  }

  private ensureKeywordsFree(subjectId: string | null, keywords: string[]): void {
    for (const rule of this.rules.subjectRules) {
      const owner = this.subjects.find((s) => s.id === rule.subjectId);
      if (!owner || owner.id === subjectId || owner.archived) continue;
      const taken = keywords.find((k) => rule.keywords.includes(k));
      if (taken) throw new BridgeError("conflict", `Ключевое слово «${taken}» уже относится к предмету «${owner.name}»`);
    }
  }

  private requireSubject(subjectId: string, revision: number): Subject {
    const subject = this.subjects.find((s) => s.id === subjectId);
    if (!subject) throw new BridgeError("not_found", "Предмет не найден");
    if (subject.revision !== revision) throw new BridgeError("conflict", "Предмет изменился в другом окне. Обновите данные и повторите.");
    return subject;
  }

  private subjectView(subjectId: string): Subject {
    const subject = this.subjects.find((s) => s.id === subjectId)!;
    return clone({ ...subject, keywords: this.keywordsOf(subjectId) });
  }

  async createSubject(input: SubjectInput): Promise<Subject> {
    await this.wait(80);
    const { name, shortName, keywords } = this.normalizeSubject(input, null);
    const id = `subject-${++this.subjectSeq}`;
    this.subjects.push({ id, name, shortName, currentWeek: 0, weekCount: 0, courseUrl: null, folderPath: `Предметы/${name}`, keywords: [], archived: false, mergedIntoId: null, revision: 0 });
    this.weeks.set(id, []);
    this.setKeywords(id, keywords);
    return this.subjectView(id);
  }

  async updateSubject(subjectId: string, revision: number, input: SubjectInput): Promise<Subject> {
    await this.wait(80);
    const subject = this.requireSubject(subjectId, revision);
    if (subject.mergedIntoId) throw new BridgeError("blocked", "Предмет слит с другим и не редактируется");
    const { name, shortName, keywords } = this.normalizeSubject(input, subjectId);
    Object.assign(subject, { name, shortName, revision: subject.revision + 1 });
    this.setKeywords(subjectId, keywords);
    return this.subjectView(subjectId);
  }

  async setSubjectArchived(subjectId: string, revision: number, archived: boolean): Promise<Subject> {
    await this.wait(60);
    const subject = this.requireSubject(subjectId, revision);
    if (subject.mergedIntoId && !archived) throw new BridgeError("blocked", "Слитый предмет нельзя вернуть из архива: его недели принадлежат другому предмету");
    if (!archived) this.ensureKeywordsFree(subjectId, this.keywordsOf(subjectId));
    Object.assign(subject, { archived, revision: subject.revision + 1 });
    return this.subjectView(subjectId);
  }

  private mergePlan(sourceId: string, targetId: string): SubjectMergePreview {
    if (sourceId === targetId) throw new BridgeError("blocked", "Предмет нельзя слить с самим собой");
    const source = this.subjects.find((s) => s.id === sourceId);
    const target = this.subjects.find((s) => s.id === targetId);
    if (!source || !target) throw new BridgeError("not_found", "Предмет не найден");
    if (source.mergedIntoId) throw new BridgeError("blocked", "Этот предмет уже слит с другим");
    if (target.archived) throw new BridgeError("blocked", "Сливать можно только в активный предмет: сначала верните его из архива");
    const sourceWeeks = this.weeks.get(sourceId) ?? [];
    const targetNumbers = new Set((this.weeks.get(targetId) ?? []).map((w) => w.number));
    const conflictingWeeks = sourceWeeks.map((w) => w.number).filter((n) => targetNumbers.has(n));
    const inbox = this.inbox.filter((f) => f.status !== "routed" && f.classification?.subjectId === sourceId);
    const targetKeywords = this.keywordsOf(targetId);
    const addedKeywords = this.keywordsOf(sourceId).filter((k) => !targetKeywords.includes(k));
    const canMerge = conflictingWeeks.length === 0;
    return {
      source: this.subjectView(sourceId),
      target: this.subjectView(targetId),
      movedWeeks: sourceWeeks.map((w) => ({ number: w.number, hasHomework: w.homeworkIds.length > 0, files: w.lectures.length + w.seminars.length + w.materials.length })),
      conflictingWeeks,
      retargetedInbox: inbox.length,
      addedKeywords,
      canMerge,
      effects: [
        `Недели «${source.name}» (${sourceWeeks.length}) станут неделями «${target.name}».`,
        `«${source.name}» уйдёт в архив с пометкой «слит».`,
        "Файлы в vault не перемещаются и не переименовываются: недели сохраняют свои папки.",
      ],
      warnings: canMerge ? [] : [`У обоих предметов есть недели ${conflictingWeeks.join(", ")}. Слияние таких недель не поддерживается: сначала разберите их вручную.`],
      fingerprint: [source.id, source.revision, target.id, target.revision, sourceWeeks.length, targetNumbers.size, inbox.length].join(":"),
    };
  }

  async previewSubjectMerge(sourceId: string, targetId: string): Promise<SubjectMergePreview> {
    await this.wait(80);
    return this.mergePlan(sourceId, targetId);
  }

  async mergeSubjects(sourceId: string, targetId: string, fingerprint: string): Promise<SubjectMergeResult> {
    await this.wait(120);
    const plan = this.mergePlan(sourceId, targetId);
    if (plan.fingerprint !== fingerprint) throw new BridgeError("conflict", "Предметы изменились после предпросмотра. Выполните предпросмотр слияния заново.");
    if (!plan.canMerge) throw new BridgeError("blocked", "Слияние заблокировано: у предметов есть недели с одинаковыми номерами");
    const moved = (this.weeks.get(sourceId) ?? []).map((w) => ({ ...w, subjectId: targetId }));
    this.weeks.set(targetId, [...(this.weeks.get(targetId) ?? []), ...moved].sort((a, b) => a.number - b.number));
    this.weeks.set(sourceId, []);
    const keywords = [...this.keywordsOf(targetId), ...plan.addedKeywords];
    this.setKeywords(sourceId, []);
    this.setKeywords(targetId, keywords);
    let retargeted = 0;
    for (const file of this.inbox) {
      if (file.status !== "routed" && file.classification?.subjectId === sourceId) {
        file.classification = { ...file.classification, subjectId: targetId };
        retargeted += 1;
      }
    }
    const source = this.subjects.find((s) => s.id === sourceId)!;
    const target = this.subjects.find((s) => s.id === targetId)!;
    Object.assign(source, { archived: true, mergedIntoId: targetId, revision: source.revision + 1, weekCount: 0 });
    Object.assign(target, { revision: target.revision + 1, weekCount: this.weeks.get(targetId)!.length });
    return { source: this.subjectView(sourceId), target: this.subjectView(targetId), movedWeeks: moved.length, retargetedInbox: retargeted };
  }

  async getWeeks(subjectId: string): Promise<Week[]> {
    await this.wait((160));
    return clone(this.weeks.get(subjectId) ?? []);
  }

  async getWeek(subjectId: string, weekNumber: number): Promise<Week | null> {
    await this.wait((180));
    return clone(this.weeks.get(subjectId)?.find((w) => w.number === weekNumber) ?? null);
  }

  async createWeek(subjectId: string, weekNumber: number): Promise<OperationResult<Week>> {
    const list = this.weeks.get(subjectId);
    if (!list) throw new BridgeError("not_found", "Предмет не найден");
    if (list.some((w) => w.number === weekNumber)) throw new BridgeError("conflict", `Неделя ${weekNumber} уже существует`);
    const week = this.makeWeek(subjectId, weekNumber);
    const op = await this.runOp(
      "create_week",
      `Создана неделя ${pad2(weekNumber)}`,
      subjectName(subjectId),
      [weekFolder(subjectId, weekNumber)],
      3,
      () => {
        const next = [...(this.weeks.get(subjectId) ?? []), week].sort((a, b) => a.number - b.number);
        this.weeks.set(subjectId, next);
      },
      () => this.weeks.set(subjectId, (this.weeks.get(subjectId) ?? []).filter((w) => w.id !== week.id)),
    );
    return { operation: op, value: clone(week) };
  }

  // ───────────── Входящие ─────────────

  async getInboxFiles(): Promise<InboxFile[]> {
    await this.wait((200));
    return clone(this.inbox.filter((f) => f.status !== "routed"));
  }

  async pickFiles(): Promise<DroppedItem[]> {
    // В браузере/mock выбор делает скрытый HTML input; пустой массив означает отмену.
    return [];
  }

  onNativeFileDrop(_listener: (items: DroppedItem[]) => void): Unsubscribe {
    return () => undefined;
  }

  async importDroppedFiles(items: DroppedItem[]): Promise<InboxFile[]> {
    await this.wait((140));
    const source = items.length > 0 ? items : DEMO_DROP;
    const files: InboxFile[] = source.map((it) => ({
      id: nextFileId(),
      name: it.name,
      kind: kindFromName(it.name),
      sizeBytes: it.sizeBytes,
      addedAt: new Date().toISOString(),
      origin: "drop",
      status: "new",
      classification: null,
      duplicate: null,
      provenance: { source: "manual", via: "drop", archiveName: null, entryPath: null, courseUrl: null, assignmentUrl: null },
    }));
    this.inbox = [...files, ...this.inbox];
    this.emitter.emit("inbox:file-added", { files: clone(files) });
    // Автоклассификация после добавления, как это будет делать backend-наблюдатель.
    void this.classifyFiles(files.map((f) => f.id));
    return clone(files);
  }

  onNativeDragState(_listener: (active: boolean) => void): Unsubscribe {
    return () => undefined;
  }

  /**
   * Имитация импорта с тем же контрактом, что у backend: отчёт по каждому файлу,
   * дубль (то же имя и размер — у HTML5 File нет содержимого для SHA-256),
   * « (N)» при совпадении имени, повтор той же операции возвращает сохранённый итог.
   */
  async importFiles(items: DroppedItem[], operationId: string = newOperationId()): Promise<OperationResult<InboxImportReport>> {
    const stored = this.importResults.get(operationId);
    if (stored) return clone({ operation: stored.operation, value: { ...stored.value, replayed: true } });
    if (items.length === 0) throw new BridgeError("blocked", "Не выбрано ни одного файла");
    if (!this.roots.importReady) throw new BridgeError("blocked", "Хранилище Obsidian не выбрано. Откройте «Настройки → Управляемые каталоги».");

    const op: Operation = { id: operationId, kind: "import", title: "Импорт во «Входящие»", detail: `Файлов: ${items.length}`, status: "running", startedAt: new Date().toISOString(), finishedAt: null, undoable: false, paths: [], backupPath: null, homeworkId: null };
    this.log.unshift(op);
    this.emitter.emit("operation:progress", { operationId, kind: "import", progress: 0, message: "Проверка файлов" });

    const reports: ImportItemReport[] = [];
    const added: InboxFile[] = [];
    for (const [index, item] of items.entries()) {
      await this.wait(160);
      reports.push(this.mockImportOne(item, added));
      this.emitter.emit("operation:progress", { operationId, kind: "import", progress: (index + 1) / items.length, message: item.name });
    }
    if (added.length > 0) this.inbox = [...added, ...this.inbox];
    if (this.roots.inbox.state === "missing") this.roots.inbox = { ...this.roots.inbox, state: "ok", message: null };

    const count = (status: ImportItemReport["status"]) => reports.filter((r) => r.status === status).length;
    const report: InboxImportReport = { operationId, items: reports, imported: count("imported"), duplicates: count("duplicate"), failed: count("failed"), skipped: 0, replayed: false };
    op.paths = added.map((f) => `Предметы/00 Входящие/${f.name}`);
    op.detail = `Добавлено: ${report.imported} · дублей: ${report.duplicates} · ошибок: ${report.failed}`;
    op.finishedAt = new Date().toISOString();
    if (added.length > 0) this.emitter.emit("inbox:file-added", { files: clone(added) });
    this.emitter.emit("inbox:updated", { reason: "import", fileIds: reports.flatMap((r) => (r.status === "failed" ? [] : [r.file.id])) });
    const firstError = reports.find((r) => r.status === "failed");
    if (report.failed === items.length && firstError?.status === "failed") {
      op.status = "failed";
      this.emitter.emit("operation:failed", { operation: clone(op), error: firstError.error });
    } else {
      op.status = "completed";
      this.emitter.emit("operation:completed", { operation: clone(op) });
    }
    const result = { operation: clone(op), value: report };
    this.importResults.set(operationId, clone(result));
    if (added.length > 0) void this.classifyFiles(added.map((f) => f.id));
    return clone(result);
  }

  private mockImportOne(item: DroppedItem, added: InboxFile[]): ImportItemReport {
    const name = item.name.replace(/[/\\:]/g, "-").trim();
    if (!name) return { status: "failed", name: "Файл", error: { code: "blocked", message: "Не указано имя файла" } };
    const active = [...added, ...this.inbox.filter((f) => f.status !== "routed")];
    const same = active.find((f) => f.name === name && f.sizeBytes === item.sizeBytes);
    if (same) return { status: "duplicate", name, file: clone(same), restored: false };
    let candidate = name;
    for (let n = 2; active.some((f) => f.name === candidate); n += 1) candidate = numberedMockName(name, n);
    const renamedFrom = candidate === name ? null : name;
    const file: InboxFile = {
      id: nextFileId(),
      name: candidate,
      kind: kindFromName(candidate),
      sizeBytes: item.sizeBytes,
      addedAt: new Date().toISOString(),
      origin: "drop",
      status: "new",
      classification: null,
      duplicate: renamedFrom ? { existingPath: `Предметы/00 Входящие/${renamedFrom}`, sameContent: false, suggestedName: candidate } : null,
      provenance: { source: "manual", via: item.origin ?? "drop", archiveName: null, entryPath: null, courseUrl: null, assignmentUrl: null },
    };
    added.push(file);
    return { status: "imported", name, file: clone(file), renamedFrom, recovered: false };
  }

  async getOperationResult(operationId: string): Promise<OperationRecord | null> {
    await this.wait(60);
    const stored = this.importResults.get(operationId);
    if (stored) return clone({ operation: stored.operation, importReport: stored.value });
    const op = this.log.find((o) => o.id === operationId);
    return op ? clone({ operation: op, importReport: null }) : null;
  }

  async openInboxFile(fileId: string): Promise<void> {
    await this.wait(60);
    if (!this.inbox.some((f) => f.id === fileId)) throw new BridgeError("not_found", "Файл во «Входящих» не найден");
  }

  async revealInboxFile(fileId: string): Promise<void> {
    await this.openInboxFile(fileId);
  }

  // ───────────── Управляемые каталоги (имитация) ─────────────

  async getManagedRoots(): Promise<ManagedRoots> {
    await this.wait(120);
    return clone(this.roots);
  }

  async chooseManagedRoot(kind: SelectableRootKind): Promise<RootSelection> {
    await this.wait(200);
    // В браузере системного диалога нет: имитируем выбор демонстрационной папки.
    this.roots = { ...this.roots, [kind]: { kind, state: "ok", displayPath: DEMO_ROOT_PATHS[kind], message: null } };
    if (kind === "vault") this.roots.inbox = { ...this.roots.inbox, displayPath: `${DEMO_ROOT_PATHS.vault}/Предметы/00 Входящие` };
    this.roots.importReady = this.roots.vault.state === "ok";
    return clone({ cancelled: false, roots: this.roots });
  }

  async prepareInboxRoot(): Promise<ManagedRoots> {
    await this.wait(120);
    if (this.roots.vault.state !== "ok") throw new BridgeError("blocked", "Сначала выберите хранилище Obsidian");
    this.roots.inbox = { ...this.roots.inbox, state: "ok", message: null };
    return clone(this.roots);
  }

  async getWorkspaces(): Promise<WorkspaceOverview> {
    const active = this.roots.workspace ? clone(this.roots.workspace) : null;
    const others = this.knownWorkspaces.filter((item) => item.id !== active?.id).map((item) => ({ ...clone(item), state: "archived" as const }));
    return { active, workspaces: active ? [active, ...others] : others, recentTransitions: clone(this.transitions) };
  }

  /** Mock повторяет контракт backend: preview ничего не меняет, confirm одноразовый. */
  async previewWorkspaceTransition(request: WorkspaceTransitionRequest): Promise<WorkspaceTransitionPreview> {
    await this.wait(60);
    const current = this.roots.workspace ? clone(this.roots.workspace) : null;
    const target = request.kind === "activate" ? this.knownWorkspaces.find((item) => item.id === request.workspaceId) ?? null : null;
    if (request.kind === "activate" && !target) throw new BridgeError("not_found", "Рабочее пространство не найдено");
    if (target && current && target.id === current.id) throw new BridgeError("conflict", "Это рабочее пространство уже активно");
    const id = `transition-${++this.transitionSeq}`;
    const effects =
      request.kind === "create_demo"
        ? ["Создать демо-хранилище только в каталоге данных приложения.", "Создать отдельную базу SQLite с «Демо-курсом».", "Сделать демо активным и перезапустить приложение."]
        : request.kind === "create_real"
          ? [".kontur-workspace.json в корне vault (только идентификатор).", "Отдельная база SQLite.", "Перезапуск приложения."]
          : ["Сделать это пространство активным и перезапустить приложение.", "Vault и учебные данные не изменяются."];
    this.transitions.unshift({ id, action: request.kind, state: "previewed", fromWorkspaceId: current?.id ?? null, targetWorkspaceId: target?.id ?? null, message: null, createdAt: new Date().toISOString(), finishedAt: null });
    this.pendingTransitions.set(id, request);
    return {
      id,
      action: request.kind,
      currentWorkspace: current,
      targetWorkspace: target ? clone(target) : null,
      targetKind: request.kind === "create_demo" ? "demo" : target?.kind ?? "real",
      vaultDisplayPath: request.kind === "create_demo" ? null : DEMO_ROOT_PATHS.vault,
      effects,
      warnings: request.kind === "create_real" ? ["Данные текущего пространства не переносятся."] : [],
      expiresAt: new Date(Date.now() + 10 * 60_000).toISOString(),
    };
  }

  async confirmWorkspaceTransition(transitionId: string): Promise<WorkspaceActivation> {
    await this.wait(120);
    const request = this.pendingTransitions.get(transitionId);
    const entry = this.transitions.find((item) => item.id === transitionId);
    if (!request || !entry || entry.state !== "previewed") throw new BridgeError("conflict", "Переход уже применён, отменён или не найден");
    this.pendingTransitions.delete(transitionId);
    let workspace: Workspace;
    if (request.kind === "activate") {
      workspace = { ...clone(this.knownWorkspaces.find((item) => item.id === request.workspaceId)!), state: "active" };
    } else {
      workspace = {
        id: `browser-${request.kind}-${this.transitionSeq}`,
        kind: request.kind === "create_demo" ? "demo" : "real",
        vaultCanonical: DEMO_ROOT_PATHS.vault,
        vaultFingerprint: `browser-fingerprint-${this.transitionSeq}`,
        createdAt: new Date().toISOString(),
        state: "active",
      };
      this.knownWorkspaces.push(clone(workspace));
    }
    this.roots = { ...buildRoots(), workspace };
    Object.assign(entry, { state: "applied", targetWorkspaceId: workspace.id, finishedAt: new Date().toISOString() });
    return { workspace: clone(workspace), restartRequired: true };
  }

  async cancelWorkspaceTransition(transitionId: string): Promise<void> {
    const entry = this.transitions.find((item) => item.id === transitionId);
    if (!entry || entry.state !== "previewed") throw new BridgeError("conflict", "Переход уже применён, отменён или не найден");
    this.pendingTransitions.delete(transitionId);
    Object.assign(entry, { state: "cancelled", finishedAt: new Date().toISOString() });
  }

  previewVaultAdoption: DesktopBridge["previewVaultAdoption"] = () => this.adoption.preview();
  applyVaultAdoption: DesktopBridge["applyVaultAdoption"] = (planId, overrides, dryRun) => this.adoption.apply(planId, overrides, dryRun);
  cancelVaultAdoption: DesktopBridge["cancelVaultAdoption"] = (planId) => this.adoption.cancel(planId);

  async exportDiagnosticReport(): Promise<{ cancelled: boolean; fileName: string | null }> {
    await this.wait(80);
    return { cancelled: false, fileName: "kontur-diagnostic-demo.json" };
  }

  async openObsidianNote(_relativePath: string): Promise<void> {
    await this.wait(60);
  }

  // ───────────── Загрузки и экспорт ЦУ (имитация этапа 6) ─────────────

  getDownloadsWatcher: DesktopBridge["getDownloadsWatcher"] = () => this.lms.getWatcher();
  setDownloadsWatcher: DesktopBridge["setDownloadsWatcher"] = (enabled) => this.lms.setWatcher(enabled);
  dismissDownloadCandidate: DesktopBridge["dismissDownloadCandidate"] = (candidateId) => this.lms.dismiss(candidateId);
  pickCuExport: DesktopBridge["pickCuExport"] = () => this.lms.pick();
  previewCuExport: DesktopBridge["previewCuExport"] = (candidateId) => this.lms.preview(candidateId);
  importCuExport: DesktopBridge["importCuExport"] = (request, operationId) => this.lms.import(request, operationId);
  setSubjectCourseUrl: DesktopBridge["setSubjectCourseUrl"] = (subjectId, url) => this.lms.setCourseUrl(subjectId, url);
  openCourse: DesktopBridge["openCourse"] = (target) => this.lms.openCourse(target);

  // ───────────── Извлечение текста (имитация этапа 7) ─────────────

  listMaterialExtractions: DesktopBridge["listMaterialExtractions"] = () => this.extraction.list();
  getMaterialExtraction: DesktopBridge["getMaterialExtraction"] = (material) => this.extraction.get(material);
  analyzeMaterial: DesktopBridge["analyzeMaterial"] = (material, options) => this.extraction.analyze(material, options?.force ?? false);
  analyzeMaterials: DesktopBridge["analyzeMaterials"] = (materials, options) => this.extraction.analyzeBatch(materials, options?.force ?? false, options?.operationId);
  cancelExtraction: DesktopBridge["cancelExtraction"] = (operationId) => this.extraction.cancel(operationId);
  clearExtractionCache: DesktopBridge["clearExtractionCache"] = () => this.extraction.clearCache();
  getExtractionSettings: DesktopBridge["getExtractionSettings"] = () => this.extraction.settings();
  setExtractionSettings: DesktopBridge["setExtractionSettings"] = (patch) => this.extraction.setSettings(patch);
  getCorpusSelection: DesktopBridge["getCorpusSelection"] = (homeworkId) => this.extraction.getSelection(homeworkId);
  setCorpusSelection: DesktopBridge["setCorpusSelection"] = (homeworkId, materials) => this.extraction.setSelection(homeworkId, materials);
  previewCorpus: DesktopBridge["previewCorpus"] = (homeworkId) => this.extraction.preview(homeworkId);

  async setHomeworkAssignmentUrl(homeworkId: string, url: string | null): Promise<Homework> {
    await this.wait(80);
    const h = this.homework.find((x) => x.id === homeworkId);
    if (!h) throw new BridgeError("not_found", "Домашнее задание не найдено");
    if (h.stage === "submitted") throw new BridgeError("immutable", "Отправленную работу изменить нельзя");
    const problem = url ? lmsUrlProblem(url) : null;
    if (problem) throw new BridgeError("blocked", problem);
    h.assignmentUrl = url?.trim() || null;
    return clone(h);
  }

  async openAssignment(target: { homeworkId: string } | { fileId: string }): Promise<void> {
    if ("fileId" in target) return this.lms.openAssignmentFromFile(target.fileId);
    await this.wait(60);
    const url = this.homework.find((x) => x.id === target.homeworkId)?.assignmentUrl;
    if (!url) throw new BridgeError("not_found", "Ссылка на задание не задана");
    const problem = lmsUrlProblem(url);
    if (problem) throw new BridgeError("blocked", problem);
  }

  async classifyFiles(fileIds: string[]): Promise<OperationResult<InboxFile[]>> {
    const targets = this.inbox.filter((f) => fileIds.includes(f.id));
    if (targets.length === 0) throw new BridgeError("not_found", "Нет файлов для классификации");
    const op = this.startOp("classify", `Классификация: ${targets.length} ${plural(targets.length, "файл", "файла", "файлов")}`, null, []);
    for (const f of targets) f.status = "classifying";
    for (let i = 0; i < targets.length; i += 1) {
      const f = targets[i]!;
      await this.wait((380));
      const updated = this.applyClassification(f, this.classify(f.name));
      Object.assign(f, updated);
      this.emitter.emit("inbox:classification-updated", { fileId: f.id, classification: clone(f.classification!), status: f.status });
      this.emitter.emit("operation:progress", { operationId: op.id, kind: op.kind, progress: (i + 1) / targets.length, message: f.name });
    }
    this.finishOp(op, false);
    return { operation: clone(op), value: clone(targets) };
  }

  async updateClassification(fileId: string, patch: import("../types").ClassificationPatch): Promise<InboxFile> {
    await this.wait((120));
    const f = this.inbox.find((x) => x.id === fileId);
    if (!f) throw new BridgeError("not_found", "Файл не найден");
    const prev = f.classification ?? { subjectId: null, weekNumber: null, category: "unknown" as MaterialCategory, confidence: 0, reasons: [], source: "manual" as const };
    const next: Classification = {
      ...prev,
      ...(patch.subjectId !== undefined ? { subjectId: patch.subjectId } : {}),
      ...(patch.weekNumber !== undefined ? { weekNumber: patch.weekNumber } : {}),
      ...(patch.category !== undefined ? { category: patch.category ?? "unknown" } : {}),
      ...(patch.homeworkId !== undefined ? { homeworkId: patch.homeworkId } : {}),
      source: "manual",
      confidence: 1,
      reasons: ["Указано вручную"],
    };
    Object.assign(f, this.applyClassification(f, next));
    this.emitter.emit("inbox:classification-updated", { fileId, classification: clone(next), status: f.status });
    return clone(f);
  }

  // ───────────── Семестр и сроки (имитация этапа 5) ─────────────

  private semester: { input: SemesterInput; revision: number } | null = {
    input: {
      startDate: `${SEMESTER_START.getFullYear()}-${String(SEMESTER_START.getMonth() + 1).padStart(2, "0")}-${String(SEMESTER_START.getDate()).padStart(2, "0")}`,
      utcOffsetMinutes: -SEMESTER_START.getTimezoneOffset(),
      weekCount: 18,
    },
    revision: 0,
  };

  private semesterWeek(n: number): { startsAt: string; endsAt: string } | null {
    if (!this.semester) return null;
    const [y, mo, d] = this.semester.input.startDate.split("-").map(Number) as [number, number, number];
    const start = Date.UTC(y, mo - 1, d) - this.semester.input.utcOffsetMinutes * 60_000 + (n - 1) * 7 * 86_400_000;
    const iso = (ms: number) => new Date(ms).toISOString().replace(".000Z", "Z");
    return { startsAt: iso(start), endsAt: iso(start + 7 * 86_400_000 - 1000) };
  }

  private semesterView(): Semester | null {
    if (!this.semester) return null;
    const { input, revision } = this.semester;
    const weeks = Array.from({ length: input.weekCount }, (_, i) => ({ number: i + 1, ...this.semesterWeek(i + 1)! }));
    const now = Date.now();
    const current = weeks.find((w) => Date.parse(w.startsAt) <= now && now <= Date.parse(w.endsAt))?.number ?? null;
    const [y, mo, d] = input.startDate.split("-").map(Number) as [number, number, number];
    const monday = new Date(Date.UTC(y, mo - 1, d)).getUTCDay() === 1;
    return { ...input, revision, weeks, currentWeek: current, warnings: monday ? [] : ["Семестр начинается не с понедельника: недели будут идти от этого дня недели."] };
  }

  async getSemester(): Promise<Semester | null> {
    await this.wait(60);
    return clone(this.semesterView());
  }

  async setSemester(input: SemesterInput, expectedRevision: number | null): Promise<SemesterSaveResult> {
    await this.wait(80);
    if (!/^\d{4}-\d{2}-\d{2}$/.test(input.startDate) || Number.isNaN(Date.parse(`${input.startDate}T00:00:00Z`))) throw new BridgeError("blocked", "Дата начала семестра должна быть в формате ГГГГ-ММ-ДД");
    if (input.utcOffsetMinutes < -720 || input.utcOffsetMinutes > 840 || input.utcOffsetMinutes % 15 !== 0) throw new BridgeError("blocked", "Часовой пояс: смещение от UTC−12:00 до UTC+14:00 с шагом 15 минут");
    if (!Number.isInteger(input.weekCount) || input.weekCount < 1 || input.weekCount > 60) throw new BridgeError("blocked", "Число недель семестра: от 1 до 60");
    if ((this.semester?.revision ?? null) !== expectedRevision) throw new BridgeError("conflict", "Семестр изменился в другом окне. Обновите данные и повторите.");
    this.semester = { input: { ...input }, revision: this.semester ? this.semester.revision + 1 : 0 };
    let updatedWeeks = 0;
    for (const list of this.weeks.values())
      for (const week of list) {
        Object.assign(week, this.semesterWeek(week.number), { datesSource: "semester" });
        updatedWeeks += 1;
      }
    return { semester: clone(this.semesterView()!), updatedWeeks };
  }

  async setHomeworkDeadline(homeworkId: string, revision: number, deadline: string | null): Promise<Homework> {
    await this.wait(60);
    const homework = this.homework.find((h) => h.id === homeworkId);
    if (!homework) throw new BridgeError("not_found", "ДЗ не найдено");
    if (homework.stage === "submitted") throw new BridgeError("blocked", "Сданное ДЗ неизменяемо: срок нельзя поменять");
    if (homework.revision !== revision) throw new BridgeError("conflict", "ДЗ изменилось в другом окне. Обновите данные и повторите.");
    let normalized: string | null = null;
    if (deadline && deadline.trim()) {
      const parsed = new Date(deadline);
      if (Number.isNaN(parsed.getTime())) throw new BridgeError("blocked", "Срок должен быть датой и временем (RFC 3339)");
      normalized = parsed.toISOString().replace(".000Z", "Z");
    }
    Object.assign(homework, { deadline: normalized, deadlineSource: normalized ? "manual" : null, revision: homework.revision + 1 });
    return clone(homework);
  }

  async createHomework(input: NewHomeworkInput): Promise<Homework> {
    await this.wait(80);
    const title = input.title.split(/\s+/).filter(Boolean).join(" ");
    if (!title || title.length > 120) throw new BridgeError("blocked", "Название ДЗ: от 1 до 120 символов");
    const subject = this.subjects.find((s) => s.id === input.subjectId);
    if (!subject) throw new BridgeError("not_found", "Предмет не найден");
    if (subject.archived) throw new BridgeError("blocked", "Предмет в архиве: верните его из архива, чтобы добавить ДЗ");
    const week = this.weeks.get(input.subjectId)?.find((w) => w.number === input.weekNumber);
    if (!week) throw new BridgeError("not_found", "Неделя не найдена: сначала создайте неделю");
    let deadline: string | null = null;
    if (input.deadline && input.deadline.trim()) {
      const parsed = new Date(input.deadline);
      if (Number.isNaN(parsed.getTime())) throw new BridgeError("blocked", "Срок должен быть датой и временем");
      deadline = parsed.toISOString().replace(".000Z", "Z");
    }
    const ordinal = Math.max(0, ...week.homeworkIds.map((id) => this.homework.find((h) => h.id === id)?.ordinal ?? 0)) + 1;
    const base = weekFolder(input.subjectId, input.weekNumber);
    const homework: Homework = {
      id: `hw-${input.subjectId}-${input.weekNumber}-${ordinal}`,
      subjectId: input.subjectId,
      weekNumber: input.weekNumber,
      ordinal,
      title,
      deadline,
      deadlineSource: deadline ? "manual" : null,
      stage: "task",
      taskFiles: [],
      versions: [],
      finalVersionId: null,
      finalFileName: null,
      submittedAt: null,
      folderPath: ordinal === 1 ? `${base}/03 ДЗ` : `${base}/03 ДЗ (${ordinal})`,
      workflowStage: "materials_collected",
      assignmentUrl: null,
      revision: 0,
    };
    this.homework.push(homework);
    week.homeworkIds = [...week.homeworkIds, homework.id];
    return clone(homework);
  }

  async previewRoute(fileId: string, destination: RouteDestination): Promise<RoutePreview> {
    await this.wait((90));
    const f = this.inbox.find((x) => x.id === fileId);
    if (!f) throw new BridgeError("not_found", "Файл не найден");
    return this.buildPreview(f, destination);
  }

  async routeFiles(fileIds: string[], destination: RouteDestination | null): Promise<OperationResult<RoutePreview[]>> {
    const files = this.inbox.filter((f) => fileIds.includes(f.id) && f.status !== "routed");
    if (files.length === 0) throw new BridgeError("not_found", "Нет файлов для раскладки");
    const unresolved = files.filter((f) => !destination && (!f.classification?.subjectId || !f.classification.weekNumber || f.classification.category === "unknown"));
    if (unresolved.length > 0)
      throw new BridgeError("confirmation_required", `Нужно уточнить ${unresolved.length} ${plural(unresolved.length, "файл", "файла", "файлов")}: предмет, неделю или тип`);

    const reserved: FileRef[] = [];
    const previews = files.map((f) => {
      const preview = this.buildPreview(f, destination ?? this.destOf(f), reserved);
      reserved.push({ id: f.id, name: preview.targetName, kind: f.kind, sizeBytes: f.sizeBytes, modifiedAt: f.addedAt, relativePath: preview.targetPath });
      return preview;
    });
    const weeksBefore = clone(this.weeks);
    const homeworkBefore = clone(this.homework);
    const candidatesBefore = clone(this.homeworkCandidates);
    const inboxBefore = clone(this.inbox);

    const op = await this.runOp(
      "route",
      `Разложено ${files.length} ${plural(files.length, "файл", "файла", "файлов")}`,
      [...new Set(previews.map((p) => p.targetPath.split("/").slice(0, 2).join(" / ")))].join(", "),
      previews.map((p) => p.targetPath),
      Math.max(2, files.length),
      () => {
        for (const f of files) {
          const dest = destination ?? this.destOf(f);
          const preview = previews.find((p) => p.fileId === f.id)!;
          const week = this.ensureWeek(dest.subjectId, dest.weekNumber);
          const ref: FileRef = { id: f.id, name: preview.targetName, kind: f.kind, sizeBytes: f.sizeBytes, modifiedAt: new Date().toISOString(), relativePath: preview.targetPath };
          if (dest.category === "lecture") week.lectures = [ref, ...week.lectures];
          else if (dest.category === "seminar") week.seminars = [ref, ...week.seminars];
          else if (dest.category === "material") week.materials = [ref, ...week.materials];
          else {
            const homework = this.ensureHomework(week, dest.homeworkId);
            if (dest.category === "homework_draft") {
              this.homeworkCandidates.set(homework.id, [ref, ...(this.homeworkCandidates.get(homework.id) ?? [])]);
            } else {
              homework.taskFiles = [ref, ...homework.taskFiles];
            }
          }
          f.status = "routed";
        }
      },
      () => {
        this.weeks = clone(weeksBefore);
        this.homework = clone(homeworkBefore);
        this.homeworkCandidates = clone(candidatesBefore);
        this.inbox = clone(inboxBefore);
      },
    );
    return { operation: clone(op), value: previews };
  }

  // ───────────── ДЗ ─────────────

  async getHomeworkList(): Promise<Homework[]> {
    await this.wait((200));
    return clone(this.homework);
  }

  async getHomework(homeworkId: string): Promise<Homework | null> {
    await this.wait((160));
    return clone(this.homework.find((h) => h.id === homeworkId) ?? null);
  }

  async getHomeworkCandidates(homeworkId: string): Promise<FileRef[]> {
    await this.wait((160));
    const h = this.mustHw(homeworkId);
    const stored = this.homeworkCandidates.get(homeworkId);
    if (stored) return clone(stored);
    const ext = h.versions[0]?.fileName.split(".").pop() ?? (h.taskFiles.some((f) => f.kind === "sheet") ? "xlsx" : "pdf");
    this.candidateSeq += 1;
    const now = new Date();
    const stamp = `${pad2(now.getHours())}-${pad2(now.getMinutes())}`;
    const generated = [
      makeFile(`Работа_черновик_${stamp}.${ext}`, `${h.folderPath}/02 Работа`, now.toISOString(), 120_000 + this.candidateSeq * 7_000),
      makeFile(`Работа (копия).${ext}`, `${h.folderPath}/02 Работа`, at(-0.2), 104_000),
    ];
    this.homeworkCandidates.set(homeworkId, generated);
    return clone(generated);
  }

  async createHomeworkVersion(
    homeworkId: string,
    fileId: string,
    meta: { author?: HomeworkVersion["author"]; promptId?: string | null; attachmentFileIds?: string[]; note?: string | null } & WorkflowCommandOptions = {},
  ): Promise<OperationResult<HomeworkVersion>> {
    const h = this.mustHw(homeworkId);
    if (h.stage === "submitted") throw new BridgeError("immutable", "ДЗ отправлено — снимок неизменяем, новые версии не создаются");
    if (meta.expectedRevision !== undefined && meta.expectedRevision !== h.revision) throw new BridgeError("conflict", "Данные ДЗ изменились — перечитайте состояние и повторите действие");
    const prev = h.versions[h.versions.length - 1] ?? null;
    const number = (prev?.number ?? 0) + 1;
    const ext = prev?.fileName.split(".").pop() ?? (h.taskFiles.some((f) => f.kind === "sheet") ? "xlsx" : "pdf");
    const label = `v${pad2(number)}`;
    const fileName = `Работа_Неделя${h.weekNumber}_${label}.${ext}`;
    const now = new Date().toISOString();
    const workDir = `${h.folderPath}/02 Работа`;
    const version: HomeworkVersion = {
      id: `${h.id}-v${number}-${Date.now().toString(36)}`,
      homeworkId,
      number,
      label,
      fileName,
      sizeBytes: (prev?.sizeBytes ?? 90_000) + 14_000,
      createdAt: now,
      sourceFileId: fileId,
      note: meta.author && meta.author !== "user" ? `Создана по промту ${meta.promptId ? "из истории" : ""}`.trim() : null,
      sections: buildSections(h.subjectId, prev?.sections ?? null),
      author: meta.author ?? "user",
      promptId: meta.promptId ?? null,
      sourceBundleId: null,
      files: [
        makeFile(fileName, workDir, now, (prev?.sizeBytes ?? 90_000) + 14_000),
        makeFile(`Пояснения_${label}.pdf`, workDir, now, 64_000 + number * 4_000),
      ],
      frozen: true,
    };
    // Версии неизменяемы и не удаляются — операция не отменяется.
    const op = await this.runOp(
      "create_version",
      `Создана версия ${label}`,
      `${subjectName(h.subjectId)} · ДЗ недели ${h.weekNumber}`,
      [`${workDir}/${version.fileName}`],
      3,
      () => {
        h.versions.push(version);
        if (h.stage === "task") h.stage = "in_progress";
        this.workflow.onVersionCreated(h, version);
        this.emitter.emit("homework:version-created", { homeworkId, version: clone(version) });
      },
      null,
      undefined,
      h.id,
    );
    return { operation: clone(op), value: clone(version) };
  }

  async markVersionFinal(versionId: string, options?: WorkflowCommandOptions): Promise<OperationResult<Homework>> {
    const h = this.homework.find((x) => x.versions.some((v) => v.id === versionId));
    if (!h) throw new BridgeError("not_found", "Версия не найдена");
    if (options?.expectedRevision !== undefined && options.expectedRevision !== h.revision) throw new BridgeError("conflict", "Данные ДЗ изменились — перечитайте состояние и повторите действие");
    if (h.stage === "submitted") throw new BridgeError("immutable", "ДЗ отправлено — итоговую версию менять нельзя");
    if (h.finalVersionId === versionId) throw new BridgeError("conflict", "Эта версия уже итоговая");
    const v = h.versions.find((x) => x.id === versionId)!;
    const ext = v.fileName.split(".").pop() ?? "pdf";
    const name = finalName(this.rules.finalNamePattern, h.weekNumber, h.subjectId, ext, v.label);
    const snapshot = { finalVersionId: h.finalVersionId, finalFileName: h.finalFileName, stage: h.stage };
    const op = await this.runOp(
      "mark_final",
      `${v.label} назначена итоговой`,
      `→ 03 На сдачу/${name}`,
      [`${h.folderPath}/03 На сдачу/${name}`],
      3,
      () => {
        h.finalVersionId = versionId;
        h.finalFileName = name;
        h.stage = "ready";
      },
      () => Object.assign(h, snapshot),
      null,
      h.id,
    );
    return { operation: clone(op), value: clone(h) };
  }

  async connectProvider(provider: ProviderId): Promise<ProviderStatus> {
    const s = this.providers.find((p) => p.provider === provider)!;
    this.setProvider(s, { state: "connecting", errorMessage: null });
    await this.wait((1400));
    if (provider === "codex") {
      this.codexAttempts += 1;
      if (this.codexAttempts === 1) {
        // Реалистичная первая ошибка — повторная попытка проходит.
        this.setProvider(s, { state: "error", errorMessage: "Локальный клиент Codex не ответил за 10 секунд. Убедитесь, что он запущен, и повторите." });
        return clone(s);
      }
    }
    this.setProvider(s, {
      state: "connected",
      errorMessage: null,
      accountLabel: provider === "codex" ? "Codex CLI · демо" : "Claude Code CLI · демо",
      cli: { ...s.cli, enabled: true, loggedIn: true, checkedAt: new Date().toISOString() },
      isolation: { ...s.isolation, consentProfileVersion: 1, consentValid: true, executionPolicy: "isolated_automatic", blockingReason: null },
    });
    return clone(s);
  }

  async checkProvider(provider: ProviderId): Promise<ProviderStatus> {
    const s = this.providers.find((p) => p.provider === provider)!;
    await this.wait(300);
    this.setProvider(s, { cli: { ...s.cli, checkedAt: new Date().toISOString() } });
    return clone(s);
  }

  async runAiIsolationSelfTest(): Promise<IsolationReport> {
    await this.wait(300);
    const checkedAt = new Date().toISOString();
    const report: IsolationReport = {
      platform: "macos", runnerAvailable: true, profileVersion: 1, passed: true,
      checks: [{ id: "mock", passed: true, message: "Демонстрационная проверка" }],
      checkedAt,
      explanation: "Демонстрационный bridge не запускает внешний CLI.",
    };
    for (const status of this.providers) {
      this.setProvider(status, { isolation: { ...status.isolation, report } });
    }
    return clone(report);
  }

  async getProviderStatuses(): Promise<ProviderStatus[]> {
    await this.wait(100);
    return clone(this.providers);
  }

  async disconnectProvider(provider: ProviderId): Promise<ProviderStatus> {
    const s = this.providers.find((p) => p.provider === provider)!;
    await this.wait((400));
    this.setProvider(s, { state: "disconnected", accountLabel: null, errorMessage: null, cli: { ...s.cli, enabled: false }, isolation: { ...s.isolation, consentProfileVersion: null, consentValid: false, executionPolicy: "manual_only" } });
    return clone(s);
  }

  async getProviderStatus(provider: ProviderId): Promise<ProviderStatus> {
    await this.wait((100));
    return clone(this.providers.find((p) => p.provider === provider)!);
  }

  // ───────────── Правила и журнал ─────────────

  async getAutomationRules(): Promise<AutomationRules> {
    await this.wait((150));
    return clone(this.rules);
  }

  async updateAutomationRules(rules: AutomationRules): Promise<OperationResult<AutomationRules>> {
    const patternError = validateNamePattern(rules.finalNamePattern);
    if (patternError) throw new BridgeError("conflict", patternError);
    if (rules.inboxPath !== this.rules.inboxPath) throw new BridgeError("conflict", "Папка «Входящих» фиксирована и не задаётся правилами");
    if (!rules.confirm.lowConfidenceRoute || !rules.confirm.markFinal || !rules.confirm.markSubmitted || rules.confirm.allowOverwrite) throw new BridgeError("database", "automation confirmation invariants are invalid");
    if (!Number.isInteger(rules.workflow.styleThreshold) || rules.workflow.styleThreshold < 0 || rules.workflow.styleThreshold > 10) {
      throw new BridgeError("conflict", "Порог расхождения со стилем должен быть целым числом от 0 до 10");
    }
    const applyRules = (next: AutomationRules) => {
      this.rules = clone(next);
      for (const p of this.providers) {
        const roles = (Object.entries(this.rules.workflow.roles) as [keyof AutomationRules["workflow"]["roles"], ProviderId][]).filter(([, prov]) => prov === p.provider).map(([role]) => role);
        const tasks = AI_TASKS.filter((task) => this.rules.workflow.tasks[task] === p.provider);
        if (roles.join() !== p.roles.join() || tasks.join() !== p.tasks.join()) this.setProvider(p, { roles, tasks });
      }
    };
    const op = await this.runOp(
      "update_rules",
      "Правила обновлены",
      null,
      [".kontur/rules.json"],
      2,
      () => {
        applyRules({ ...clone(rules), confirm: { lowConfidenceRoute: true, markFinal: true, markSubmitted: true, allowOverwrite: false } });
      },
      // Как в backend: сохранение правил не отменяется через журнал.
      null,
    );
    return { operation: clone(op), value: clone(this.rules) };
  }

  async getActivityLog(): Promise<Operation[]> {
    await this.wait((140));
    return clone(this.log);
  }

  async undoOperation(operationId: string): Promise<OperationResult<null>> {
    const target = this.log.find((o) => o.id === operationId);
    if (!target) throw new BridgeError("not_found", "Операция не найдена");
    const undo = this.undoers.get(operationId);
    if (!target.undoable || !undo || target.status !== "completed") throw new BridgeError("not_undoable", "Эту операцию нельзя отменить");
    const hw = target.homeworkId ? this.homework.find((h) => h.id === target.homeworkId) : null;
    if (hw?.stage === "submitted") throw new BridgeError("immutable", "ДЗ отправлено — отменять операции по нему нельзя");
    const op = this.startOp("undo", `Отменено: ${target.title}`, target.detail, target.paths);
    for (let i = 1; i <= 2; i += 1) {
      await this.wait((220));
      this.emitter.emit("operation:progress", { operationId: op.id, kind: op.kind, progress: i / 2, message: "Восстановление из резервной копии" });
    }
    undo();
    this.undoers.delete(operationId);
    target.status = "undone";
    target.undoable = false;
    this.finishOp(op, false);
    return { operation: clone(op), value: null };
  }

  // ───────────── Рабочий цикл ДЗ (делегируется MockWorkflow) ─────────────

  getWorkflow: DesktopBridge["getWorkflow"] = (...a) => this.workflow.getWorkflow(...a);
  saveManualAnalysis: DesktopBridge["saveManualAnalysis"] = (...a) => this.workflow.saveManualAnalysis(...a);
  saveManualReviewResult: DesktopBridge["saveManualReviewResult"] = (...a) => this.workflow.saveManualReviewResult(...a);
  commitPrompt: DesktopBridge["commitPrompt"] = (...a) => this.workflow.commitPrompt(...a);
  addTeacherRequirement: DesktopBridge["addTeacherRequirement"] = (...a) => this.workflow.addTeacherRequirement(...a);
  setCommentAuthority: DesktopBridge["setCommentAuthority"] = (...a) => this.workflow.setCommentAuthority(...a);
  withdrawRequirement: DesktopBridge["withdrawRequirement"] = (...a) => this.workflow.withdrawRequirement(...a);
  confirmStyleDivergence: DesktopBridge["confirmStyleDivergence"] = (...a) => this.workflow.confirmStyleDivergence(...a);
  transitionWorkflow: DesktopBridge["transitionWorkflow"] = (...a) => this.workflow.transitionWorkflow(...a);
  getStyleProfile: DesktopBridge["getStyleProfile"] = (...a) => this.workflow.getStyleProfile(...a);
  prepareSourceBundle: DesktopBridge["prepareSourceBundle"] = (...a) => this.workflow.prepareSourceBundle(...a);
  setSourceIncluded: DesktopBridge["setSourceIncluded"] = (...a) => this.workflow.setSourceIncluded(...a);
  runMaterialAnalysis: DesktopBridge["runMaterialAnalysis"] = (...a) => this.workflow.runMaterialAnalysis(...a);
  runVersionReview: DesktopBridge["runVersionReview"] = (...a) => this.workflow.runVersionReview(...a);
  runRevisionPrompt: DesktopBridge["runRevisionPrompt"] = (...a) => this.workflow.runRevisionPrompt(...a);
  createRevisionPrompt: DesktopBridge["createRevisionPrompt"] = (...a) => this.workflow.createRevisionPrompt(...a);
  sendPromptToAuthor: DesktopBridge["sendPromptToAuthor"] = (...a) => this.workflow.sendPromptToAuthor(...a);
  updatePromptText: DesktopBridge["updatePromptText"] = (...a) => this.workflow.updatePromptText(...a);
  getPrompts: DesktopBridge["getPrompts"] = (...a) => this.workflow.getPrompts(...a);
  cancelRun: DesktopBridge["cancelRun"] = (...a) => this.workflow.cancelRun(...a);
  getReviewRuns: DesktopBridge["getReviewRuns"] = (...a) => this.workflow.getReviewRuns(...a);
  saveRunAsIteration: DesktopBridge["saveRunAsIteration"] = (...a) => this.workflow.saveRunAsIteration(...a);
  getIterations: DesktopBridge["getIterations"] = (...a) => this.workflow.getIterations(...a);
  compareVersions: DesktopBridge["compareVersions"] = (...a) => this.workflow.compareVersions(...a);
  getTeacherComments: DesktopBridge["getTeacherComments"] = (...a) => this.workflow.getTeacherComments(...a);
  saveTeacherComments: DesktopBridge["saveTeacherComments"] = (...a) => this.workflow.saveTeacherComments(...a);
  importTeacherTranscript: DesktopBridge["importTeacherTranscript"] = (...a) => this.workflow.importTeacherTranscript(...a);
  previewTeacherTranscript: DesktopBridge["previewTeacherTranscript"] = (...a) => this.workflow.previewTeacherTranscript(...a);
  removeTeacherComment: DesktopBridge["removeTeacherComment"] = (...a) => this.workflow.removeTeacherComment(...a);
  runTeacherReview: DesktopBridge["runTeacherReview"] = (...a) => this.workflow.runTeacherReview(...a);
  getTeacherRequirements: DesktopBridge["getTeacherRequirements"] = (...a) => this.workflow.getTeacherRequirements(...a);
  resolveRequirement: DesktopBridge["resolveRequirement"] = (...a) => this.workflow.resolveRequirement(...a);
  getReadiness: DesktopBridge["getReadiness"] = (...a) => this.workflow.getReadiness(...a);
  markReadyToSubmit: DesktopBridge["markReadyToSubmit"] = (...a) => this.workflow.markReadyToSubmit(...a);
  buildSubmissionPackage: DesktopBridge["buildSubmissionPackage"] = (...a) => this.workflow.buildSubmissionPackage(...a);
  getSubmissionPackage: DesktopBridge["getSubmissionPackage"] = (...a) => this.workflow.getSubmissionPackage(...a);
  setPackageCheck: DesktopBridge["setPackageCheck"] = (...a) => this.workflow.setPackageCheck(...a);
  confirmSubmission: DesktopBridge["confirmSubmission"] = (...a) => this.workflow.confirmSubmission(...a);
  getSubmissionReceipt: DesktopBridge["getSubmissionReceipt"] = (...a) => this.workflow.getSubmissionReceipt(...a);
  listAiJobs: DesktopBridge["listAiJobs"] = (...a) => this.workflow.listAiJobs(...a);
  getAiJob: DesktopBridge["getAiJob"] = (...a) => this.workflow.getAiJob(...a);
  prepareAiJob: DesktopBridge["prepareAiJob"] = (...a) => this.workflow.prepareAiJob(...a);
  sendAiJob: DesktopBridge["sendAiJob"] = (...a) => this.workflow.sendAiJob(...a);
  cancelAiJob: DesktopBridge["cancelAiJob"] = (...a) => this.workflow.cancelAiJob(...a);
  submitAiResponse: DesktopBridge["submitAiResponse"] = (...a) => this.workflow.submitAiResponse(...a);
  importAiResponseFile: DesktopBridge["importAiResponseFile"] = () => this.workflow.importAiResponseFile();
  discardAiJob: DesktopBridge["discardAiJob"] = (...a) => this.workflow.discardAiJob(...a);
  confirmAiResult: DesktopBridge["confirmAiResult"] = (...a) => this.workflow.confirmAiResult(...a);

  async revealInFinder(relativePath: string): Promise<void> {
    await this.wait(80);
    if (!relativePath) throw new BridgeError("not_found", "Папка не указана");
  }

  async openProviderApp(provider: ProviderId): Promise<void> {
    await this.wait(80);
    const s = this.providers.find((p) => p.provider === provider);
    if (!s) throw new BridgeError("not_found", "Провайдер не найден");
  }

  // ───────────── Внутреннее ─────────────

  /** Задержка с разбросом ±40 %, масштабируется опцией latency. */
  private wait(ms: number): Promise<void> {
    const t = ms * this.latency * (1 + Math.random() * 0.4);
    // latency 0: всё равно уступаем цикл событий, чтобы отмена и подписки успевали сработать.
    return new Promise((r) => setTimeout(r, Math.max(0, t)));
  }

  private context(): MockContext {
    return {
      homework: () => this.homework,
      weeks: () => this.weeks,
      rules: () => this.rules,
      providers: () => this.providers,
      emitter: this.emitter,
      wait: (ms) => this.wait(ms),
      runOp: (kind, title, detail, paths, steps, commit, undo, homeworkId) => this.runOp(kind, title, detail, paths, steps, commit, undo, undefined, homeworkId),
      startOp: (kind, title, detail, paths, homeworkId) => this.startOp(kind, title, detail, paths, null, homeworkId),
      finishOp: (op) => this.finishOp(op, false),
      failOp: (op, error) => {
        op.status = "failed";
        op.finishedAt = new Date().toISOString();
        this.emitter.emit("operation:failed", { operation: clone(op), error });
      },
      createVersion: (hwId, fileId, meta) => this.createHomeworkVersion(hwId, fileId, meta),
      candidates: (hwId) => this.getHomeworkCandidates(hwId),
    };
  }

  private seedOp(kind: OperationKind, title: string, detail: string, days: number, undoable: boolean): Operation {
    opSeq += 1;
    const id = `op-seed-${opSeq}`;
    if (undoable) this.undoers.set(id, () => undefined);
    return { id, kind, title, detail, status: "completed", startedAt: at(days, 14), finishedAt: at(days, 14), undoable, paths: [], backupPath: null, homeworkId: null };
  }

  private startOp(kind: OperationKind, title: string, detail: string | null, paths: string[], backupPath: string | null = null, homeworkId: string | null = null): Operation {
    opSeq += 1;
    const op: Operation = { id: `op-${Date.now().toString(36)}-${opSeq}`, kind, title, detail, status: "running", startedAt: new Date().toISOString(), finishedAt: null, undoable: false, paths, backupPath, homeworkId };
    this.log.unshift(op);
    return op;
  }

  private finishOp(op: Operation, undoable: boolean): void {
    op.status = "completed";
    op.finishedAt = new Date().toISOString();
    op.undoable = undoable;
    this.emitter.emit("operation:completed", { operation: clone(op) });
  }

  /** start → progress → commit → регистрация undo → completed → resolve. */
  private async runOp(kind: OperationKind, title: string, detail: string | null, paths: string[], steps: number, commit: () => void, undo: (() => void) | null, backupPath?: string | null, homeworkId: string | null = null): Promise<Operation> {
    const backup = backupPath ?? null;
    const op = this.startOp(kind, title, detail, paths, backup, homeworkId);
    const messages = ["Резервная копия", "Проверка конфликтов", "Перемещение", "Обновление Obsidian", "Готово"];
    for (let i = 1; i <= steps; i += 1) {
      await this.wait((200));
      this.emitter.emit("operation:progress", { operationId: op.id, kind, progress: i / steps, message: messages[Math.min(i - 1, messages.length - 1)]! });
    }
    try {
      commit();
    } catch (error) {
      op.status = "failed";
      op.finishedAt = new Date().toISOString();
      const payload = { code: "unknown" as const, message: error instanceof Error ? error.message : "Ошибка фиксации операции" };
      this.emitter.emit("operation:failed", { operation: clone(op), error: payload });
      throw error;
    }
    if (undo) this.undoers.set(op.id, undo);
    this.finishOp(op, !!undo);
    return op;
  }

  private setProvider(s: ProviderStatus, patch: Partial<ProviderStatus>): void {
    Object.assign(s, patch, { lastCheckedAt: new Date().toISOString() });
    s.availability = mockAvailability(s.state);
    this.emitter.emit("provider:status-changed", { status: clone(s) });
  }

  private mustHw(id: string): Homework {
    const h = this.homework.find((x) => x.id === id);
    if (!h) throw new BridgeError("not_found", "ДЗ не найдено");
    return h;
  }

  private ensureWeek(subjectId: string, n: number): Week {
    const list = this.weeks.get(subjectId) ?? [];
    let w = list.find((x) => x.number === n);
    if (!w) {
      w = this.makeWeek(subjectId, n);
      list.push(w);
      list.sort((a, b) => a.number - b.number);
      this.weeks.set(subjectId, list);
    }
    return w;
  }

  private makeWeek(subjectId: string, n: number): Week {
    const dates = this.semesterWeek(n);
    return { id: `${subjectId}-w${n}`, subjectId, number: n, ...(dates ?? { startsAt: null, endsAt: null }), datesSource: dates ? "semester" : "unknown", topic: null, lectures: [], seminars: [], materials: [], homeworkIds: [] };
  }

  /** Как backend: одно ДЗ — оно; ни одного — будет создано; несколько — нужен выбор. */
  private homeworkFor(dest: RouteDestination): Homework | null {
    const week = this.weeks.get(dest.subjectId)?.find((w) => w.number === dest.weekNumber);
    const list = (week?.homeworkIds ?? []).map((id) => this.homework.find((h) => h.id === id)).filter((h): h is Homework => !!h);
    if (dest.homeworkId) {
      const chosen = list.find((h) => h.id === dest.homeworkId);
      if (!chosen) throw new BridgeError("conflict", "Выбранное ДЗ не относится к этой неделе");
      return chosen;
    }
    if (list.length > 1) throw new BridgeError("confirmation_required", "В этой неделе несколько ДЗ — выберите, к какому относится файл");
    return list[0] ?? null;
  }

  private ensureHomework(week: Week, homeworkId?: string | null): Homework {
    const existing = this.homeworkFor({ subjectId: week.subjectId, weekNumber: week.number, category: "homework_task", homeworkId });
    if (existing) return existing;
    const id = `hw-${week.subjectId}-${week.number}`;
    const homework: Homework = {
      id,
      subjectId: week.subjectId,
      weekNumber: week.number,
      ordinal: 1,
      title: `Домашняя работа — неделя ${week.number}`,
      // Как backend (этап 5): срок не выдумывается.
      deadline: null,
      deadlineSource: null,
      stage: "task",
      taskFiles: [],
      versions: [],
      finalVersionId: null,
      finalFileName: null,
      submittedAt: null,
      folderPath: `${weekFolder(week.subjectId, week.number)}/03 ДЗ`,
      workflowStage: "materials_collected",
      assignmentUrl: null,
      revision: 0,
    };
    this.homework.push(homework);
    week.homeworkIds = [...week.homeworkIds, id];
    return homework;
  }

  private destOf(f: InboxFile): RouteDestination {
    const c = f.classification;
    return { subjectId: c?.subjectId ?? "", weekNumber: c?.weekNumber ?? CURRENT_WEEK, category: c?.category ?? "unknown", homeworkId: c?.homeworkId ?? null };
  }

  private buildPreview(f: InboxFile, dest: RouteDestination, reserved: FileRef[] = []): RoutePreview {
    const exists = (this.weeks.get(dest.subjectId) ?? []).some((w) => w.number === dest.weekNumber);
    const ext = f.name.includes(".") ? `.${f.name.split(".").pop()}` : "";
    const stem = f.name.slice(0, f.name.length - ext.length);
    const isHomework = dest.category === "homework_task" || dest.category === "homework_data" || dest.category === "homework_draft";
    const homework = isHomework ? this.homeworkFor(dest) : null;
    const folder = homework
      ? `${homework.folderPath}/${dest.category === "homework_draft" ? "02 Работа" : "01 Условие и данные"}`
      : `${weekFolder(dest.subjectId, dest.weekNumber)}/${CATEGORY_FOLDER[dest.category]}`;
    const dup = this.findDuplicate(f, dest, reserved);
    const targetName = dup ? dup.suggestedName : `${stem}${ext}`;
    const targetPath = `${folder}/${targetName}`;
    return {
      fileId: f.id,
      sourceName: f.name,
      sourceRelativePath: `Предметы/00 Входящие/${f.name}`,
      targetPath,
      targetName,
      createsWeek: !exists,
      createsDirectories: !exists ? [weekFolder(dest.subjectId, dest.weekNumber), folder] : [folder],
      sha256Match: dup?.sameContent ?? false,
      nameConflict: Boolean(dup && !dup.sameContent),
      renamed: Boolean(dup && !dup.sameContent),
      action: dup?.sameContent ? "skip" : dup ? "rename" : "copy",
      undoable: true,
      duplicate: dup,
    };
  }

  private findDuplicate(f: InboxFile, dest: RouteDestination, reserved: FileRef[] = []): InboxFile["duplicate"] {
    const week = this.weeks.get(dest.subjectId)?.find((w) => w.number === dest.weekNumber);
    const homeworkId = dest.homeworkId ?? week?.homeworkIds[0];
    const homework = homeworkId ? this.homework.find((h) => h.id === homeworkId) : undefined;
    const candidates = homework ? (this.homeworkCandidates.get(homework.id) ?? []) : [];
    const versionFiles = homework?.versions.flatMap((version) => version.files) ?? [];
    const all = [...(week?.lectures ?? []), ...(week?.seminars ?? []), ...(week?.materials ?? []), ...(homework?.taskFiles ?? []), ...candidates, ...versionFiles, ...reserved];
    const norm = (s: string) => s.toLowerCase().replace(/\.[^.]+$/, "").replace(/[_\s.\-—]+/g, " ").trim();
    const hit = all.find((x) => norm(x.name) === norm(f.name));
    if (!hit) return null;
    const ext = f.name.includes(".") ? `.${f.name.split(".").pop()}` : "";
    const stem = f.name.slice(0, f.name.length - ext.length);
    const names = new Set(all.map((x) => x.name.toLocaleLowerCase("ru")));
    let copy = 2;
    let suggestedName = `${stem} (${copy})${ext}`;
    while (names.has(suggestedName.toLocaleLowerCase("ru"))) {
      copy += 1;
      suggestedName = `${stem} (${copy})${ext}`;
    }
    return { existingPath: hit.relativePath, sameContent: Math.abs(hit.sizeBytes - f.sizeBytes) < 200_000, suggestedName };
  }

  private applyClassification(f: InboxFile, c: Classification): InboxFile {
    const ambiguous = !c.subjectId || !c.weekNumber || c.category === "unknown" || c.confidence < this.rules.lowConfidenceThreshold;
    const next: InboxFile = { ...f, classification: c, status: ambiguous ? "ambiguous" : "classified" };
    next.duplicate = c.subjectId && c.weekNumber ? this.findDuplicate(next, { subjectId: c.subjectId, weekNumber: c.weekNumber, category: c.category }) : null;
    return next;
  }

  /** Эвристика-заглушка. Настоящий классификатор — задача backend. */
  private classify(name: string): Classification {
    const lower = name.toLowerCase();
    const reasons: string[] = [];
    let subjectId: string | null = null;
    let best = 0;
    for (const rule of this.rules.subjectRules) {
      const hits = rule.keywords.filter((k) => lower.includes(k));
      const score = hits.reduce((s, k) => s + k.length, 0);
      if (score > best) {
        best = score;
        subjectId = rule.subjectId;
        reasons.splice(0, reasons.length, `имя содержит «${hits[0]}»`);
      }
    }
    if (subjectId === "matan" && /продв|adv|\+/.test(lower)) subjectId = "matan-adv";
    if (subjectId === "oba" && /продв|adv|\+/.test(lower)) subjectId = "oba-adv";

    let weekNumber: number | null = null;
    const m = lower.match(/(?:нед(?:еля)?|week|лекция|семинар|seminar|hw|дз|листок|lec)[\s_\-.]*(\d{1,2})/) ?? lower.match(/(?:^|[^\d])(\d{1,2})(?:[^\d]|$)/);
    if (m?.[1] && Number(m[1]) >= 1 && Number(m[1]) <= 18) {
      weekNumber = Number(m[1]);
      reasons.push(`номер недели ${weekNumber} из имени`);
    } else if (subjectId) {
      weekNumber = CURRENT_WEEK;
      reasons.push("неделя по календарю семестра");
    }

    let category: MaterialCategory = "unknown";
    if (/лекци|lecture|lec/.test(lower)) category = "lecture";
    else if (/семинар|seminar/.test(lower)) category = "seminar";
    else if (/услов|hw|дз|листок/.test(lower)) category = /данн|data|\.csv$/.test(lower) ? "homework_data" : "homework_task";
    else if (/черновик|draft/.test(lower)) category = "homework_draft";
    else if (subjectId) category = "material";
    if (category !== "unknown") reasons.push(`тип: ${categoryReason(category)}`);

    let confidence = 0.15;
    if (subjectId) confidence += 0.4;
    if (m) confidence += 0.22;
    if (category !== "unknown" && category !== "material") confidence += 0.2;
    if (/^(scan|img|image|document|file)[_\s-]?\d*/.test(lower)) confidence = Math.min(confidence, 0.2);
    if (!subjectId) reasons.push("предмет не распознан");
    return { subjectId, weekNumber, category, confidence: Math.min(0.97, Math.round(confidence * 100) / 100), reasons, source: "auto" };
  }
}

function categoryReason(c: MaterialCategory): string {
  const map: Record<MaterialCategory, string> = {
    lecture: "лекция",
    seminar: "семинар",
    material: "материал",
    homework_task: "условие ДЗ",
    homework_data: "данные ДЗ",
    homework_draft: "черновик",
    unknown: "неизвестно",
  };
  return map[c];
}
