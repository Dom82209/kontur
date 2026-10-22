import type { TypedEmitter } from "../emitter";
import { newOperationId } from "../intake";
import {
  BridgeError,
  type CorpusItem,
  type CorpusPreview,
  type CorpusSelection,
  type ExtractedBlock,
  type ExtractionBatchItem,
  type ExtractionBatchReport,
  type ExtractionFormat,
  type ExtractionMetadata,
  type ExtractionSettings,
  type ExtractionStatus,
  type ExtractionWarning,
  type FileKind,
  type FileRef,
  type Homework,
  type InboxFile,
  type MaterialExtraction,
  type MaterialExtractionItem,
  type MaterialRef,
  type Operation,
  type OperationResult,
} from "../types";

const clone = <T,>(v: T): T => structuredClone(v);

const MAX_CORPUS_CHARS = 600_000;

const LIMITS: ExtractionSettings["limits"] = {
  maxInputBytes: 200 * 1024 * 1024,
  maxTextChars: 2_000_000,
  timeLimitSeconds: 60,
  maxPdfPages: 2000,
  maxSheets: 100,
  maxCellsPerSheet: 200_000,
  maxSlides: 1000,
  maxCorpusChars: MAX_CORPUS_CHARS,
};

interface MockExtractionContext {
  emitter: TypedEmitter;
  wait: (ms: number) => Promise<void>;
  inbox: () => InboxFile[];
  weekFiles: () => { file: FileRef; context: string }[];
  homework: () => Homework[];
  log: (op: Operation) => void;
}

interface Material {
  ref: MaterialRef;
  name: string;
  kind: FileKind;
  sizeBytes: number;
  context: string;
}

type Stored = Omit<MaterialExtraction, "material" | "name" | "context" | "stale" | "cached">;

const key = (ref: MaterialRef) => `${ref.kind}:${ref.id}`;

/** Детерминированный псевдо-SHA-256: в браузере содержимого файла нет. */
function pseudoSha(name: string, size: number): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (const ch of `${name}:${size}`) {
    h1 = Math.imul(h1 ^ ch.charCodeAt(0), 16777619) >>> 0;
    h2 = Math.imul(h2 + ch.charCodeAt(0), 2246822519) >>> 0;
  }
  return (h1.toString(16).padStart(8, "0") + h2.toString(16).padStart(8, "0")).repeat(4);
}

const emptyMetadata = (): ExtractionMetadata => ({
  title: null,
  author: null,
  subject: null,
  application: null,
  producer: null,
  created: null,
  modified: null,
  pageCount: null,
  sheetCount: null,
  slideCount: null,
  wordCount: null,
  pdfVersion: null,
  imageFormat: null,
  imageWidth: null,
  imageHeight: null,
  encoding: null,
  delimiter: null,
  rowCount: null,
  columnCount: null,
  language: null,
});

function render(blocks: ExtractedBlock[]): string {
  return blocks
    .map((b) => {
      switch (b.type) {
        case "heading":
          return `${"#".repeat(b.level)} ${b.text}`;
        case "paragraph":
          return b.text;
        case "list_item":
          return `${"  ".repeat(b.level)}${b.ordered ? "1." : "-"} ${b.text}`;
        case "table":
          return b.rows.map((r) => `| ${r.join(" | ")} |`).join("\n");
        case "page":
          return `--- Страница ${b.number} ---\n${b.text}`;
        case "slide":
          return [`--- Слайд ${b.number} ---`, b.title ? `# ${b.title}` : null, ...b.body, b.notes ? `Заметки докладчика: ${b.notes}` : null].filter(Boolean).join("\n");
        case "sheet":
          return [`## Лист «${b.name}»`, ...b.cells.map((c) => `${c.ref}: ${c.value}${c.formula ? ` [${c.formula}]` : ""}`)].join("\n");
        case "code":
          return `\`\`\`${b.language ?? ""}\n${b.text}\n\`\`\``;
      }
    })
    .join("\n\n");
}

function demoExtraction(material: Material, ocrEnabled: boolean): { format: ExtractionFormat; status: ExtractionStatus; statusMessage: string | null; metadata: ExtractionMetadata; blocks: ExtractedBlock[]; warnings: ExtractionWarning[] } {
  const metadata = emptyMetadata();
  const stem = material.name.replace(/\.[^.]+$/, "");
  const lower = material.name.toLowerCase();
  if (material.kind === "pdf") {
    metadata.pageCount = 3;
    metadata.title = stem;
    metadata.pdfVersion = "1.7";
    const blocks: ExtractedBlock[] = [1, 2, 3].map((n) => ({ type: "page", number: n, text: `${stem}\nДемонстрационный текст страницы ${n}: определения, примеры и выводы.` }));
    return { format: "pdf", status: "complete", statusMessage: null, metadata, blocks, warnings: [] };
  }
  if (material.kind === "doc") {
    metadata.title = stem;
    metadata.wordCount = 180;
    const blocks: ExtractedBlock[] = [
      { type: "heading", level: 1, text: stem },
      { type: "paragraph", text: "Цель работы — применить методы из лекции к данным курса." },
      { type: "list_item", level: 0, ordered: true, text: "Загрузить данные" },
      { type: "list_item", level: 0, ordered: true, text: "Посчитать показатели" },
      { type: "table", rows: [["Показатель", "Значение"], ["Среднее", "42"]], truncated: false },
    ];
    return { format: "docx", status: "complete", statusMessage: null, metadata, blocks, warnings: [{ code: "images_skipped", message: "Изображения и фигуры без текста не извлекаются (2)", location: null }] };
  }
  if (material.kind === "sheet" && (lower.endsWith(".csv") || lower.endsWith(".tsv"))) {
    metadata.delimiter = lower.endsWith(".tsv") ? "tab" : ";";
    metadata.rowCount = 3;
    metadata.columnCount = 2;
    const blocks: ExtractedBlock[] = [{ type: "table", rows: [["Месяц", "Выручка"], ["Январь", "120"], ["Февраль", "135"]], truncated: false }];
    return { format: lower.endsWith(".tsv") ? "tsv" : "csv", status: "complete", statusMessage: null, metadata, blocks, warnings: [] };
  }
  if (material.kind === "sheet") {
    metadata.sheetCount = 2;
    const blocks: ExtractedBlock[] = [
      {
        type: "sheet",
        name: "Данные",
        hidden: false,
        declaredRange: "A1:C4",
        usedRange: "A1:C4",
        rowCount: 4,
        columnCount: 3,
        cellCount: 8,
        formulaCount: 1,
        cells: [
          { ref: "A1", value: "Месяц", formula: null },
          { ref: "B1", value: "Выручка", formula: null },
          { ref: "A2", value: "Январь", formula: null },
          { ref: "B2", value: "120", formula: null },
          { ref: "A3", value: "Февраль", formula: null },
          { ref: "B3", value: "135", formula: null },
          { ref: "A4", value: "Итого", formula: null },
          { ref: "B4", value: "255", formula: "=SUM(B2:B3)" },
        ],
        truncated: false,
      },
    ];
    return { format: "xlsx", status: "complete", statusMessage: null, metadata, blocks, warnings: [{ code: "formulas_not_evaluated", message: "Формулы не вычисляются: показан текст формулы и значение, сохранённое в файле", location: null }] };
  }
  if (material.kind === "slides") {
    metadata.slideCount = 2;
    const blocks: ExtractedBlock[] = [
      { type: "slide", number: 1, title: stem, body: ["Неделя курса"], notes: null, hidden: false },
      { type: "slide", number: 2, title: "Ключевые метрики", body: ["Retention", "Конверсия"], notes: "Напомнить определения", hidden: false },
    ];
    return { format: "pptx", status: "complete", statusMessage: null, metadata, blocks, warnings: [] };
  }
  if (material.kind === "image") {
    metadata.imageFormat = lower.endsWith(".png") ? "png" : "jpeg";
    metadata.imageWidth = 1600;
    metadata.imageHeight = 900;
    return {
      format: "image",
      status: "needs_ocr",
      statusMessage: ocrEnabled ? "OCR включён, но в демонстрационном режиме распознавание недоступно. Метаданные сохранены" : "Текст на изображении не распознаётся: OCR выключен в Настройках. Метаданные сохранены",
      metadata,
      blocks: [],
      warnings: [{ code: ocrEnabled ? "ocr_unavailable" : "ocr_disabled", message: ocrEnabled ? "Локальный OCR недоступен" : "OCR выключен", location: null }],
    };
  }
  if (material.kind === "notebook") {
    metadata.language = "python";
    const blocks: ExtractedBlock[] = [
      { type: "heading", level: 1, text: "Задание" },
      { type: "code", language: "python", text: "import pandas as pd\ndf = pd.read_csv('data.csv')" },
    ];
    return { format: "notebook", status: "complete", statusMessage: null, metadata, blocks, warnings: [{ code: "outputs_skipped", message: "Результаты выполнения ячеек не извлекаются", location: null }] };
  }
  if (material.kind === "text") {
    metadata.encoding = "UTF-8";
    const blocks: ExtractedBlock[] = [{ type: "paragraph", text: `Текст файла «${material.name}»` }];
    return { format: lower.endsWith(".md") ? "markdown" : "text", status: "complete", statusMessage: null, metadata, blocks, warnings: [] };
  }
  return {
    format: "unknown",
    status: "unsupported",
    statusMessage: material.kind === "archive" ? "ZIP-архив: при извлечении текста архивы не распаковываются" : "Формат не поддерживается",
    metadata,
    blocks: [],
    warnings: [],
  };
}

/** Имитация этапа 7 для browser/mock: извлечение, cache по «SHA-256», пакет с отменой и подборки. */
export class MockExtraction {
  private ocrEnabled = false;
  private cache = new Map<string, Stored>();
  private links = new Map<string, string>();
  private selections = new Map<string, { updatedAt: string; items: { ref: MaterialRef; sha256: string }[] }>();
  private results = new Map<string, OperationResult<ExtractionBatchReport>>();
  private cancels = new Map<string, boolean>();

  constructor(private readonly ctx: MockExtractionContext) {}

  private materials(): Material[] {
    return [
      ...this.ctx.inbox().map((f): Material => ({ ref: { kind: "inbox", id: f.id }, name: f.name, kind: f.kind, sizeBytes: f.sizeBytes, context: "Входящие" })),
      ...this.ctx.weekFiles().map(({ file, context }): Material => ({ ref: { kind: "file", id: file.id }, name: file.name, kind: file.kind, sizeBytes: file.sizeBytes, context })),
    ];
  }

  private find(ref: MaterialRef): Material {
    const found = this.materials().find((m) => m.ref.kind === ref.kind && m.ref.id === ref.id);
    if (!found) throw new BridgeError("not_found", "Материал не найден — обновите список");
    return found;
  }

  private shaOf(m: Material) {
    return pseudoSha(m.name, m.sizeBytes);
  }

  private cacheKey(m: Material, sha: string) {
    return m.kind === "image" ? `${sha}:image${this.ocrEnabled ? "+no-ocr" : ""}` : `${sha}:plain`;
  }

  private byPrefix(sha: string): Stored | undefined {
    return [...this.cache.entries()].find(([k]) => k.startsWith(`${sha}:`))?.[1];
  }

  private toDto(m: Material, stored: Stored, cached: boolean): MaterialExtraction {
    return clone({ ...stored, material: m.ref, name: m.name, context: m.context, stale: false, cached });
  }

  async list(): Promise<MaterialExtractionItem[]> {
    await this.ctx.wait(140);
    return this.materials().map((m) => {
      const sha = this.links.get(key(m.ref));
      const stored = sha ? this.byPrefix(sha) : undefined;
      return {
        material: clone(m.ref),
        name: m.name,
        fileKind: m.kind,
        sizeBytes: m.sizeBytes,
        context: m.context,
        extraction: stored
          ? { sha256: stored.sha256, format: stored.format, status: stored.status, statusMessage: stored.statusMessage, textChars: stored.textChars, truncated: stored.truncated, warningCount: stored.warnings.length, analyzedAt: stored.extractedAt, stale: false, ocrUsed: stored.ocrUsed }
          : null,
      };
    });
  }

  async get(ref: MaterialRef): Promise<MaterialExtraction | null> {
    await this.ctx.wait(90);
    const m = this.find(ref);
    const sha = this.links.get(key(ref));
    const stored = sha ? this.byPrefix(sha) : undefined;
    return stored ? this.toDto(m, stored, true) : null;
  }

  private async extractOne(ref: MaterialRef, force: boolean, cancelled: () => boolean): Promise<MaterialExtraction> {
    const m = this.find(ref);
    const sha = this.shaOf(m);
    const cacheKey = this.cacheKey(m, sha);
    const hit = this.cache.get(cacheKey);
    if (hit && !force) {
      this.links.set(key(ref), sha);
      await this.ctx.wait(40);
      return this.toDto(m, hit, true);
    }
    const started = Date.now();
    await this.ctx.wait(320 + Math.min(900, m.sizeBytes / 4000));
    if (cancelled()) throw new BridgeError("cancelled", "Извлечение остановлено пользователем");
    const demo = demoExtraction(m, this.ocrEnabled);
    const text = render(demo.blocks);
    const stored: Stored = {
      sha256: sha,
      extractorVersion: 1,
      format: demo.format,
      status: demo.status,
      statusMessage: demo.statusMessage,
      metadata: demo.metadata,
      blocks: demo.blocks,
      blocksTruncated: false,
      text,
      textChars: [...text].length,
      truncated: false,
      warnings: demo.warnings,
      ocrUsed: false,
      sourceSizeBytes: m.sizeBytes,
      durationMs: Date.now() - started,
      extractedAt: new Date().toISOString(),
    };
    this.cache.set(cacheKey, stored);
    this.links.set(key(ref), sha);
    return this.toDto(m, stored, false);
  }

  async analyze(ref: MaterialRef, force = false): Promise<MaterialExtraction> {
    const result = await this.extractOne(ref, force, () => false);
    this.ctx.emitter.emit("extraction:updated", { reason: "analyzed", materials: [clone(ref)] });
    return result;
  }

  async analyzeBatch(refs: MaterialRef[], force = false, operationId: string = newOperationId()): Promise<OperationResult<ExtractionBatchReport>> {
    const stored = this.results.get(operationId);
    if (stored) return clone({ operation: stored.operation, value: { ...stored.value, replayed: true } });
    if (this.cancels.has(operationId)) throw new BridgeError("conflict", "Извлечение с этим идентификатором уже выполняется");
    const unique = refs.filter((r, i) => refs.findIndex((x) => x.kind === r.kind && x.id === r.id) === i);
    if (unique.length === 0) throw new BridgeError("blocked", "Выберите хотя бы один материал");
    const total = unique.length;
    const op: Operation = { id: operationId, kind: "extract", title: "Извлечение текста", detail: `Материалов: ${total}`, status: "running", startedAt: new Date().toISOString(), finishedAt: null, undoable: false, paths: [], backupPath: null, homeworkId: null };
    this.ctx.log(op);
    this.cancels.set(operationId, false);
    const emit = (completed: number, material: MaterialRef | null, name: string | null, outcome: ExtractionBatchItem["outcome"] | null) => {
      this.ctx.emitter.emit("extraction:progress", { operationId, completed, total, material, name, outcome });
      this.ctx.emitter.emit("operation:progress", { operationId, kind: "extract", progress: completed / total, message: name ?? "Извлечение текста" });
    };
    emit(0, null, null, null);
    const items: ExtractionBatchItem[] = [];
    let stopped = false;
    for (const [index, ref] of unique.entries()) {
      const name = this.materials().find((m) => m.ref.kind === ref.kind && m.ref.id === ref.id)?.name ?? "Материал не найден";
      if (stopped || this.cancels.get(operationId)) {
        stopped = true;
        items.push({ material: ref, name, outcome: "cancelled", status: null, textChars: 0, error: null });
        continue;
      }
      try {
        const result = await this.extractOne(ref, force, () => !!this.cancels.get(operationId));
        items.push({ material: ref, name, outcome: result.cached ? "cached" : "extracted", status: result.status, textChars: result.textChars, error: null });
      } catch (e) {
        if (e instanceof BridgeError && e.code === "cancelled") {
          stopped = true;
          items.push({ material: ref, name, outcome: "cancelled", status: null, textChars: 0, error: null });
        } else {
          const error = e instanceof BridgeError ? { code: e.code, message: e.message } : { code: "unknown" as const, message: "Неизвестная ошибка" };
          items.push({ material: ref, name, outcome: "failed", status: null, textChars: 0, error });
        }
      }
      emit(index + 1, ref, name, items[items.length - 1]!.outcome);
    }
    this.cancels.delete(operationId);
    const count = (o: ExtractionBatchItem["outcome"]) => items.filter((i) => i.outcome === o).length;
    const report: ExtractionBatchReport = { operationId, items, extracted: count("extracted"), cached: count("cached"), failed: count("failed"), cancelled: count("cancelled"), stoppedByUser: stopped, replayed: false };
    const processed = report.extracted + report.cached;
    op.status = stopped || processed === 0 ? "failed" : "completed";
    op.finishedAt = new Date().toISOString();
    op.detail = `Извлечено: ${report.extracted}, из cache: ${report.cached}, ошибок: ${report.failed}${stopped ? `, остановлено: ${report.cancelled}` : ""}`;
    const result = { operation: clone(op), value: report };
    this.results.set(operationId, clone(result));
    this.ctx.emitter.emit("extraction:updated", { reason: "analyzed", materials: items.filter((i) => i.outcome === "extracted" || i.outcome === "cached").map((i) => clone(i.material)) });
    if (op.status === "failed") {
      this.ctx.emitter.emit("operation:failed", { operation: clone(op), error: stopped ? { code: "cancelled", message: `Остановлено пользователем: обработано ${processed} из ${total}` } : { code: "io_error", message: "Ни один материал не обработан" } });
    } else {
      this.ctx.emitter.emit("operation:completed", { operation: clone(op) });
    }
    return clone(result);
  }

  async cancel(operationId: string): Promise<boolean> {
    if (!this.cancels.has(operationId)) return false;
    this.cancels.set(operationId, true);
    return true;
  }

  async clearCache(): Promise<{ removed: number }> {
    await this.ctx.wait(80);
    const removed = this.cache.size;
    this.cache.clear();
    this.links.clear();
    this.ctx.emitter.emit("extraction:updated", { reason: "cleared", materials: [] });
    return { removed };
  }

  async settings(): Promise<ExtractionSettings> {
    await this.ctx.wait(60);
    return {
      ocrEnabled: this.ocrEnabled,
      ocr: { available: false, engine: null, languages: null, message: "Демонстрационный режим: локальный OCR не подключён. В приложении используется установленный Tesseract, если он найден" },
      limits: { ...LIMITS },
    };
  }

  async setSettings(patch: { ocrEnabled: boolean }): Promise<ExtractionSettings> {
    this.ocrEnabled = patch.ocrEnabled;
    return this.settings();
  }

  private checkHomework(homeworkId: string | null): Homework | null {
    if (homeworkId === null) return null;
    const h = this.ctx.homework().find((x) => x.id === homeworkId);
    if (!h) throw new BridgeError("not_found", "ДЗ не найдено");
    return h;
  }

  private current(ref: MaterialRef, pinned: string): { stored: Stored | null; problem: string | null; material: Material | null } {
    const material = this.materials().find((m) => m.ref.kind === ref.kind && m.ref.id === ref.id) ?? null;
    if (!material) return { stored: null, problem: "Материал больше не существует", material };
    const linked = this.links.get(key(ref));
    if (!linked) return { stored: null, problem: "Текст ещё не извлечён", material };
    if (linked !== pinned) return { stored: null, problem: "Текст извлечён из другой версии файла — выберите заново", material };
    const stored = this.byPrefix(linked);
    if (!stored) return { stored: null, problem: "Результат удалён из cache — проанализируйте заново", material };
    if (stored.status !== "complete" && stored.status !== "partial") return { stored: null, problem: stored.statusMessage ?? "Из файла не извлечён текст", material };
    return { stored, problem: null, material };
  }

  async getSelection(homeworkId: string | null): Promise<CorpusSelection> {
    await this.ctx.wait(80);
    this.checkHomework(homeworkId);
    const selection = this.selections.get(homeworkId ?? "__general");
    const items: CorpusItem[] = (selection?.items ?? []).map(({ ref, sha256 }) => {
      const { stored, problem, material } = this.current(ref, sha256);
      return {
        material: clone(ref),
        name: material?.name ?? "Материал удалён",
        context: material?.context ?? "",
        sha256,
        status: stored?.status ?? null,
        textChars: stored?.textChars ?? 0,
        warningCount: stored?.warnings.length ?? 0,
        ready: !problem,
        problem,
      };
    });
    return { homeworkId, updatedAt: selection?.updatedAt ?? null, items, totalChars: items.filter((i) => i.ready).reduce((s, i) => s + i.textChars, 0), limitChars: MAX_CORPUS_CHARS };
  }

  async setSelection(homeworkId: string | null, refs: MaterialRef[]): Promise<CorpusSelection> {
    await this.ctx.wait(100);
    const homework = this.checkHomework(homeworkId);
    if (homework?.stage === "submitted") throw new BridgeError("immutable", "Работа отправлена — подборку материалов изменить нельзя");
    const unique = refs.filter((r, i) => refs.findIndex((x) => x.kind === r.kind && x.id === r.id) === i);
    const items = unique.map((ref) => {
      const material = this.find(ref);
      const sha = this.links.get(key(ref));
      if (!sha) throw new BridgeError("blocked", `«${material.name}»: сначала извлеките текст`);
      const { problem } = this.current(ref, sha);
      if (problem) throw new BridgeError("blocked", `«${material.name}»: ${problem}`);
      return { ref: clone(ref), sha256: sha };
    });
    this.selections.set(homeworkId ?? "__general", { updatedAt: new Date().toISOString(), items });
    return this.getSelection(homeworkId);
  }

  async preview(homeworkId: string | null): Promise<CorpusPreview> {
    await this.ctx.wait(120);
    this.checkHomework(homeworkId);
    const selection = this.selections.get(homeworkId ?? "__general");
    const preview: CorpusPreview = { homeworkId, documents: [], excluded: [], totalChars: 0, limitChars: MAX_CORPUS_CHARS, truncated: false };
    for (const { ref, sha256 } of selection?.items ?? []) {
      const { stored, problem, material } = this.current(ref, sha256);
      if (!stored || problem) {
        preview.excluded.push({ material: clone(ref), name: material?.name ?? "Материал удалён", reason: problem ?? "Недоступен" });
        continue;
      }
      const remaining = MAX_CORPUS_CHARS - preview.totalChars;
      const text = [...stored.text].slice(0, remaining).join("");
      const cut = stored.textChars > remaining;
      if (cut) preview.truncated = true;
      preview.totalChars += [...text].length;
      preview.documents.push({ material: clone(ref), name: material!.name, sha256, format: stored.format, status: stored.status, text, textChars: [...text].length, truncated: cut || stored.truncated, warnings: clone(stored.warnings) });
    }
    return preview;
  }
}
