import { TypedEmitter } from "../emitter";
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
  type InboxFile,
  type MaterialCategory,
  type Operation,
  type OperationKind,
  type OperationResult,
  type ProviderId,
  type ProviderStatus,
  type RouteDestination,
  type RoutePreview,
  type Subject,
  type Unsubscribe,
  type Week,
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
  nextFileId,
  pad2,
  subjectName,
  weekFolder,
  weekRange,
} from "./data";
import { plural } from "../../format";
import { MockWorkflow, type MockContext } from "./workflow";

/**
 * MockDesktopBridge — имитация desktop-backend.
 * Хранит состояние в памяти, эмулирует задержки, прогресс и ошибки.
 * Ничего не пишет на диск, не ходит в сеть, не хранит секреты.
 */

export const clone = <T,>(v: T): T => structuredClone(v);

export interface MockOptions {
  /** Множитель задержек: 1 — реалистично, 0 — мгновенно (тесты). */
  latency?: number;
}

let opSeq = 0;

export class MockDesktopBridge implements DesktopBridge {
  readonly kind = "mock" as const;

  private emitter = new TypedEmitter();
  private subjects: Subject[] = clone(SUBJECTS);
  private weeks = buildAllWeeks();
  private inbox: InboxFile[] = buildInbox();
  private homework: Homework[] = buildHomework();
  private providers: ProviderStatus[] = buildProviders();
  private rules: AutomationRules = buildRules();
  private log: Operation[] = [];
  private undoers = new Map<string, () => void>();
  private homeworkCandidates = new Map<string, FileRef[]>();
  private codexAttempts = 0;
  private candidateSeq = 0;
  private readonly latency: number;
  private readonly workflow: MockWorkflow;

  constructor(options: MockOptions = {}) {
    this.latency = options.latency ?? 1;
    this.workflow = new MockWorkflow(this.context());
    // Привязываем ДЗ к неделям.
    for (const h of this.homework) {
      const w = this.weeks.get(h.subjectId)?.find((x) => x.number === h.weekNumber);
      if (w) w.homeworkId = h.id;
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
    const deadlines = [...active]
      .sort((a, b) => a.deadline.localeCompare(b.deadline))
      .map((h) => ({ homeworkId: h.id, subjectId: h.subjectId, title: h.title, deadline: h.deadline, stage: h.stage }));
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
      automation: { state: "active", watchedPath: this.rules.downloadsWatchPath, lastRunAt: this.log[0]?.startedAt ?? at(0), pending: pending.length },
      providers: this.providers,
      weekProgress: { done: weekItems.filter((i) => i.done).length, total: weekItems.length, items: weekItems },
    });
  }

  async getSubjects(): Promise<Subject[]> {
    await this.wait((120));
    return clone(this.subjects);
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
    }));
    this.inbox = [...files, ...this.inbox];
    this.emitter.emit("inbox:file-added", { files: clone(files) });
    // Автоклассификация после добавления, как это будет делать backend-наблюдатель.
    void this.classifyFiles(files.map((f) => f.id));
    return clone(files);
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
      source: "manual",
      confidence: 1,
      reasons: ["Указано вручную"],
    };
    Object.assign(f, this.applyClassification(f, next));
    this.emitter.emit("inbox:classification-updated", { fileId, classification: clone(next), status: f.status });
    return clone(f);
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
            const homework = this.ensureHomework(week);
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

  async createHomeworkVersion(homeworkId: string, fileId: string, meta: { author?: HomeworkVersion["author"]; promptId?: string | null } = {}): Promise<OperationResult<HomeworkVersion>> {
    const h = this.mustHw(homeworkId);
    if (h.stage === "submitted") throw new BridgeError("immutable", "ДЗ отправлено — снимок неизменяем, новые версии не создаются");
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

  async markVersionFinal(versionId: string): Promise<OperationResult<Homework>> {
    const h = this.homework.find((x) => x.versions.some((v) => v.id === versionId));
    if (!h) throw new BridgeError("not_found", "Версия не найдена");
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
      this.rules.backupBeforeChange ? `.kontur/backup/${Date.now()}/03 На сдачу` : null,
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
    this.setProvider(s, { state: "connected", errorMessage: null, accountLabel: provider === "codex" ? "Codex CLI · локально" : "Claude (desktop)" });
    return clone(s);
  }

  async disconnectProvider(provider: ProviderId): Promise<ProviderStatus> {
    const s = this.providers.find((p) => p.provider === provider)!;
    await this.wait((400));
    this.setProvider(s, { state: "disconnected", accountLabel: null, errorMessage: null });
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
    if (!rules.finalNamePattern.includes("{Неделя}")) throw new BridgeError("conflict", "Формат имени должен содержать {Неделя}, иначе имена недель совпадут");
    if (!Number.isInteger(rules.workflow.styleThreshold) || rules.workflow.styleThreshold < 0 || rules.workflow.styleThreshold > 10) {
      throw new BridgeError("conflict", "Порог расхождения со стилем должен быть целым числом от 0 до 10");
    }
    const prev = clone(this.rules);
    const applyRules = (next: AutomationRules) => {
      this.rules = clone(next);
      for (const p of this.providers) {
        const roles = (Object.entries(this.rules.workflow.roles) as [keyof AutomationRules["workflow"]["roles"], ProviderId][]).filter(([, prov]) => prov === p.provider).map(([role]) => role);
        if (roles.join() !== p.roles.join()) this.setProvider(p, { roles });
      }
    };
    const op = await this.runOp(
      "update_rules",
      "Правила обновлены",
      null,
      [".kontur/rules.json"],
      2,
      () => {
        applyRules({ ...clone(rules), confirm: { ...rules.confirm, markSubmitted: true, allowOverwrite: false } });
      },
      () => {
        applyRules(prev);
      },
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
    const backup = backupPath === undefined ? (this.rules.backupBeforeChange ? `.kontur/backup/${Date.now()}` : null) : backupPath;
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
    return { id: `${subjectId}-w${n}`, subjectId, number: n, ...weekRange(n), topic: null, lectures: [], seminars: [], materials: [], homeworkId: null };
  }

  private ensureHomework(week: Week): Homework {
    const linked = week.homeworkId ? this.homework.find((h) => h.id === week.homeworkId) : null;
    const existing = linked ?? this.homework.find((h) => h.subjectId === week.subjectId && h.weekNumber === week.number);
    if (existing) {
      week.homeworkId = existing.id;
      return existing;
    }
    const id = `hw-${week.subjectId}-${week.number}`;
    const homework: Homework = {
      id,
      subjectId: week.subjectId,
      weekNumber: week.number,
      title: `Домашняя работа — неделя ${week.number}`,
      deadline: week.endsAt,
      stage: "task",
      taskFiles: [],
      versions: [],
      finalVersionId: null,
      finalFileName: null,
      submittedAt: null,
      folderPath: `${weekFolder(week.subjectId, week.number)}/03 ДЗ`,
      workflowStage: "materials_collected",
      assignmentUrl: null,
    };
    this.homework.push(homework);
    week.homeworkId = id;
    return homework;
  }

  private destOf(f: InboxFile): RouteDestination {
    const c = f.classification;
    return { subjectId: c?.subjectId ?? "", weekNumber: c?.weekNumber ?? CURRENT_WEEK, category: c?.category ?? "unknown" };
  }

  private buildPreview(f: InboxFile, dest: RouteDestination, reserved: FileRef[] = []): RoutePreview {
    const exists = (this.weeks.get(dest.subjectId) ?? []).some((w) => w.number === dest.weekNumber);
    const ext = f.name.includes(".") ? `.${f.name.split(".").pop()}` : "";
    const stem = f.name.slice(0, f.name.length - ext.length);
    const folder = `${weekFolder(dest.subjectId, dest.weekNumber)}/${CATEGORY_FOLDER[dest.category]}`;
    const dup = this.findDuplicate(f, dest, reserved);
    const targetName = dup ? dup.suggestedName : `${stem}${ext}`;
    return { fileId: f.id, targetPath: `${folder}/${targetName}`, targetName, createsWeek: !exists, duplicate: dup };
  }

  private findDuplicate(f: InboxFile, dest: RouteDestination, reserved: FileRef[] = []): InboxFile["duplicate"] {
    const week = this.weeks.get(dest.subjectId)?.find((w) => w.number === dest.weekNumber);
    const homework = week?.homeworkId ? this.homework.find((h) => h.id === week.homeworkId) : this.homework.find((h) => h.subjectId === dest.subjectId && h.weekNumber === dest.weekNumber);
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
