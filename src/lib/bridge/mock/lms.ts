import { lmsUrlProblem } from "../../lmsLinks";
import type { TypedEmitter } from "../emitter";
import { newOperationId } from "../intake";
import {
  BridgeError,
  type Classification,
  type DownloadCandidate,
  type DownloadsWatcher,
  type ExportEntryPreview,
  type ExportImportRequest,
  type ExportPreview,
  type ImportItemReport,
  type InboxFile,
  type InboxImportReport,
  type ManagedRoots,
  type MaterialCategory,
  type Operation,
  type OperationResult,
  type Subject,
} from "../types";
import { kindFromName, nextFileId } from "./data";

const clone = <T,>(v: T): T => structuredClone(v);

interface MockLmsContext {
  emitter: TypedEmitter;
  wait: (ms: number) => Promise<void>;
  roots: () => ManagedRoots;
  subjects: () => Subject[];
  inbox: () => InboxFile[];
  addInbox: (files: InboxFile[]) => void;
  log: (op: Operation) => void;
}

interface DemoEntry {
  path: string;
  size: number;
  subjectId: string | null;
  week: number | null;
  category: MaterialCategory;
  confidence: number;
  reasons: string[];
}

const OBA_EXPORT: DemoEntry[] = [
  { path: "Основы бизнес-аналитики/Неделя 8/Лекции/Лекция 8. Когорты.pdf", size: 2_400_000, subjectId: "oba", week: 8, category: "lecture", confidence: 1, reasons: ["Курс «Основы бизнес-аналитики» совпадает с предметом «Основы бизнес-аналитики»", "Папка «Неделя 8» → неделя 8", "Папка «Лекции» → лекция"] },
  { path: "Основы бизнес-аналитики/Неделя 8/Семинары/Семинар 8.pdf", size: 980_000, subjectId: "oba", week: 8, category: "seminar", confidence: 1, reasons: ["Папка «Неделя 8» → неделя 8", "Папка «Семинары» → семинар"] },
  { path: "Основы бизнес-аналитики/Неделя 8/Домашнее задание/Условие ДЗ 8.pdf", size: 310_000, subjectId: "oba", week: 8, category: "homework_task", confidence: 1, reasons: ["Папка «Домашнее задание» → домашнее задание; условие по имени файла"] },
  { path: "Основы бизнес-аналитики/Неделя 8/Домашнее задание/orders.xlsx", size: 1_150_000, subjectId: "oba", week: 8, category: "homework_data", confidence: 1, reasons: ["Папка «Домашнее задание» → домашнее задание; данные по типу файла"] },
  { path: "Основы бизнес-аналитики/Неделя 8/Домашнее задание/hw8.ipynb", size: 42_000, subjectId: "oba", week: 8, category: "homework_task", confidence: 0.94, reasons: ["Папка «Домашнее задание» → домашнее задание; ноутбук — проверьте, условие это или данные"] },
  { path: "Основы бизнес-аналитики/Материалы курса/Глоссарий.pdf", size: 120_000, subjectId: "oba", week: null, category: "material", confidence: 0.7, reasons: ["Номер недели в пути не найден", "Папка «Материалы курса» → дополнительный материал"] },
  { path: "Основы бизнес-аналитики/Неделя 8/README", size: 1_200, subjectId: "oba", week: 8, category: "unknown", confidence: 0.7, reasons: ["Тип материала по пути не определён"] },
  { path: "Основы бизнес-аналитики/Неделя 8/Дополнительно/датасеты.zip", size: 8_400_000, subjectId: "oba", week: 8, category: "material", confidence: 1, reasons: ["Папка «Дополнительно» → дополнительный материал"] },
];

/** Имитация этапа 6 для browser/mock: watcher, кандидаты, предпросмотр и импорт экспорта ЦУ. */
export class MockLms {
  private watcher: DownloadsWatcher = { enabled: false, state: "disabled", message: null, lastScanAt: null, candidates: [] };
  private previews = new Map<string, ExportPreview>();
  private results = new Map<string, OperationResult<InboxImportReport>>();
  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly ctx: MockLmsContext) {}

  async getWatcher(): Promise<DownloadsWatcher> {
    await this.ctx.wait(60);
    return clone(this.watcher);
  }

  async setWatcher(enabled: boolean): Promise<DownloadsWatcher> {
    await this.ctx.wait(120);
    if (enabled) {
      if (this.ctx.roots().downloads.state !== "ok") throw new BridgeError("blocked", "Сначала выберите папку загрузок в «Настройки → Управляемые каталоги»");
      this.watcher = { ...this.watcher, enabled: true, state: "watching", message: null, lastScanAt: new Date().toISOString() };
      // Демонстрация: через пару секунд «скачивается» экспорт ЦУ.
      if (this.timer) clearTimeout(this.timer);
      this.timer = setTimeout(() => {
        if (!this.watcher.enabled) return;
        this.addCandidate("Основы бизнес-аналитики — экспорт.zip", 13_400_000, "watcher");
      }, 2000);
    } else {
      if (this.timer) clearTimeout(this.timer);
      this.watcher = { ...this.watcher, enabled: false, state: "disabled", message: null, candidates: this.watcher.candidates.filter((c) => c.origin !== "watcher") };
    }
    this.notify();
    return clone(this.watcher);
  }

  async dismiss(candidateId: string): Promise<DownloadsWatcher> {
    await this.ctx.wait(60);
    this.watcher = { ...this.watcher, candidates: this.watcher.candidates.filter((c) => c.id !== candidateId) };
    this.notify();
    return clone(this.watcher);
  }

  async pick(): Promise<DownloadCandidate | null> {
    await this.ctx.wait(160);
    // В браузере системного диалога нет: имитируем выбор демонстрационного экспорта.
    return clone(this.addCandidate("Основы бизнес-аналитики (1).zip", 13_400_000, "manual"));
  }

  async preview(candidateId: string): Promise<ExportPreview> {
    await this.ctx.wait(420);
    const candidate = this.watcher.candidates.find((c) => c.id === candidateId);
    if (!candidate) throw new BridgeError("not_found", "Кандидат на импорт не найден — обновите список или выберите файл заново");
    const entries: ExportEntryPreview[] = OBA_EXPORT.map((e, index) => {
      const name = e.path.split("/").pop()!;
      const nested = name.endsWith(".zip");
      const complete = !!e.subjectId && !!e.week && e.category !== "unknown";
      const status = complete && e.confidence >= 0.7 && !nested ? "classified" : "ambiguous";
      const warnings = [
        ...(nested ? ["Вложенный архив не распаковывается и не импортируется"] : []),
        ...(status === "ambiguous" && !nested ? ["Предмет, неделя или тип не определены уверенно — проверьте"] : []),
      ];
      return {
        entryId: String(index + 2),
        path: e.path,
        name,
        inboxName: name,
        sizeBytes: e.size,
        kind: kindFromName(name),
        classification: { subjectId: e.subjectId, weekNumber: e.week, category: e.category, confidence: e.confidence, reasons: e.reasons, source: "auto" },
        status,
        selectable: !nested,
        selected: !nested,
        warnings,
      };
    });
    const ambiguous = entries.filter((e) => e.selectable && e.status === "ambiguous").length;
    const preview: ExportPreview = {
      candidateId,
      archiveName: candidate.name,
      archiveSha256: `${"0".repeat(56)}${String(this.seq).padStart(8, "0")}`,
      format: "zip",
      courseTitle: "Основы бизнес-аналитики",
      entries,
      ignored: ["__MACOSX/Основы бизнес-аналитики/._Лекция 8. Когорты.pdf", "Основы бизнес-аналитики/.DS_Store"],
      warnings: [`Требуют уточнения: ${ambiguous} — такие файлы попадут во «Входящие» без раскладки`, "Пропущены служебные файлы архиватора: 2"],
      totalFiles: entries.length,
      ambiguousFiles: ambiguous,
      totalBytes: entries.reduce((sum, e) => sum + e.sizeBytes, 0),
    };
    this.previews.set(candidateId, preview);
    return clone(preview);
  }

  async import(request: ExportImportRequest, operationId: string = newOperationId()): Promise<OperationResult<InboxImportReport>> {
    const stored = this.results.get(operationId);
    if (stored) return clone({ operation: stored.operation, value: { ...stored.value, replayed: true } });
    const preview = this.previews.get(request.candidateId);
    if (!preview) throw new BridgeError("not_found", "Сначала откройте предпросмотр экспорта");
    if (preview.archiveSha256 !== request.archiveSha256) throw new BridgeError("conflict", "Файл экспорта изменился после предпросмотра — откройте предпросмотр заново");
    if (request.entries.length === 0) throw new BridgeError("blocked", "Не выбрано ни одного файла для импорта");
    for (const url of [request.courseUrl, request.assignmentUrl]) {
      const problem = url ? lmsUrlProblem(url) : null;
      if (problem) throw new BridgeError("blocked", problem);
    }
    if (!this.ctx.roots().importReady) throw new BridgeError("blocked", "Хранилище Obsidian не выбрано. Откройте «Настройки → Управляемые каталоги».");

    const total = request.entries.length;
    const op: Operation = { id: operationId, kind: "import", title: "Импорт экспорта ЦУ", detail: `${preview.archiveName} · выбрано файлов: ${total}`, status: "running", startedAt: new Date().toISOString(), finishedAt: null, undoable: false, paths: [], backupPath: null, homeworkId: null };
    this.ctx.log(op);
    const progress = (phase: "inspect" | "extract" | "import" | "cleanup", completed: number, entryName: string | null, fraction: number) => {
      this.ctx.emitter.emit("cu-import:progress", { operationId, phase, completed, total, entryName });
      this.ctx.emitter.emit("operation:progress", { operationId, kind: "import", progress: fraction, message: entryName ?? "Проверка архива" });
    };
    progress("inspect", 0, null, 0);
    await this.ctx.wait(220);

    const reports: ImportItemReport[] = [];
    const added: InboxFile[] = [];
    const via = this.watcher.candidates.find((c) => c.id === request.candidateId)?.origin === "watcher" ? ("watcher" as const) : ("picker" as const);
    for (const [index, choice] of request.entries.entries()) {
      const entry = preview.entries.find((e) => e.entryId === choice.entryId && e.selectable);
      if (!entry) throw new BridgeError("blocked", "Запись не найдена в архиве или не может быть импортирована");
      progress("extract", index, entry.name, index / total);
      await this.ctx.wait(140);
      progress("import", index, entry.name, (index + 0.5) / total);
      const existing = this.ctx.inbox().find((f) => f.status !== "routed" && f.name === entry.inboxName && f.sizeBytes === entry.sizeBytes);
      if (existing) {
        reports.push({ status: "duplicate", name: entry.name, file: clone(existing), restored: false });
        continue;
      }
      const classification: Classification = choice.edited
        ? { subjectId: choice.subjectId, weekNumber: choice.weekNumber, category: choice.category, confidence: 1, reasons: ["Подтверждено пользователем в предпросмотре импорта"], source: "manual" }
        : clone(entry.classification);
      const complete = !!classification.subjectId && !!classification.weekNumber && classification.category !== "unknown";
      const file: InboxFile = {
        id: nextFileId(),
        name: entry.inboxName,
        kind: entry.kind,
        sizeBytes: entry.sizeBytes,
        addedAt: new Date().toISOString(),
        origin: "lms",
        status: complete && (choice.edited || entry.status === "classified") ? "classified" : "ambiguous",
        classification,
        duplicate: null,
        provenance: { source: "cu_export", via, archiveName: preview.archiveName, entryPath: entry.path, courseUrl: request.courseUrl?.trim() || null, assignmentUrl: request.assignmentUrl?.trim() || null },
      };
      added.push(file);
      reports.push({ status: "imported", name: entry.name, file: clone(file), renamedFrom: null, recovered: false });
    }
    progress("cleanup", total, null, 1);
    if (added.length > 0) this.ctx.addInbox(added);
    const count = (status: ImportItemReport["status"]) => reports.filter((r) => r.status === status).length;
    const report: InboxImportReport = { operationId, items: reports, imported: count("imported"), duplicates: count("duplicate"), failed: count("failed"), skipped: preview.entries.length - total, replayed: false };
    op.status = "completed";
    op.finishedAt = new Date().toISOString();
    op.paths = added.map((f) => `Предметы/00 Входящие/${f.name}`);
    op.detail = `Добавлено: ${report.imported} · дублей: ${report.duplicates} · ошибок: ${report.failed} · пропущено: ${report.skipped}`;
    if (added.length > 0) this.ctx.emitter.emit("inbox:file-added", { files: clone(added) });
    this.ctx.emitter.emit("inbox:updated", { reason: "import", fileIds: reports.flatMap((r) => (r.status === "failed" ? [] : [r.file.id])) });
    this.ctx.emitter.emit("operation:completed", { operation: clone(op) });
    this.watcher = { ...this.watcher, candidates: this.watcher.candidates.filter((c) => c.id !== request.candidateId) };
    this.notify();
    const result = { operation: clone(op), value: report };
    this.results.set(operationId, clone(result));
    return clone(result);
  }

  async setCourseUrl(subjectId: string, url: string | null): Promise<Subject> {
    await this.ctx.wait(80);
    const subject = this.ctx.subjects().find((s) => s.id === subjectId);
    if (!subject) throw new BridgeError("not_found", "Предмет не найден");
    const problem = url ? lmsUrlProblem(url) : null;
    if (problem) throw new BridgeError("blocked", problem);
    subject.courseUrl = url?.trim() || null;
    return clone(subject);
  }

  async openCourse(target: { subjectId: string } | { fileId: string }): Promise<void> {
    await this.ctx.wait(60);
    const url = "subjectId" in target ? this.ctx.subjects().find((s) => s.id === target.subjectId)?.courseUrl : this.ctx.inbox().find((f) => f.id === target.fileId)?.provenance?.courseUrl;
    if (!url) throw new BridgeError("not_found", "Ссылка на курс не задана");
    const problem = lmsUrlProblem(url);
    if (problem) throw new BridgeError("blocked", problem);
    // В browser/mock внешний браузер не открывается: проверка allowlist — та же, что в backend.
  }

  async openAssignmentFromFile(fileId: string): Promise<void> {
    await this.ctx.wait(60);
    const url = this.ctx.inbox().find((f) => f.id === fileId)?.provenance?.assignmentUrl;
    if (!url) throw new BridgeError("not_found", "Ссылка на задание не задана");
    const problem = lmsUrlProblem(url);
    if (problem) throw new BridgeError("blocked", problem);
  }

  private addCandidate(name: string, sizeBytes: number, origin: "watcher" | "manual"): DownloadCandidate {
    this.seq += 1;
    const candidate: DownloadCandidate = { id: `cand-${this.seq}`, name, sizeBytes, detectedAt: new Date().toISOString(), origin, kind: name.toLowerCase().endsWith(".zip") ? "zip" : name.toLowerCase().endsWith(".pdf") ? "pdf" : "file" };
    this.watcher = { ...this.watcher, candidates: [candidate, ...this.watcher.candidates] };
    this.notify();
    return candidate;
  }

  private notify(): void {
    this.ctx.emitter.emit("downloads:watcher-updated", { watcher: clone(this.watcher) });
  }
}
