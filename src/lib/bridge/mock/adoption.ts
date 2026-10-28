import {
  BridgeError,
  type AdoptionCategoryOverride,
  type AdoptionPlan,
  type AdoptionPlanStatus,
  type AdoptionStatus,
  type PlannedFile,
  type VaultAdoptionPreview,
  type VaultAdoptionReport,
} from "../types";

/**
 * Имитация этапа 2 для demo/browser: синтетический vault, та же семантика,
 * что у backend — план неизменяем, dry-run ничего не сохраняет, ручная категория
 * ДЗ доступна только файлам из «03 ДЗ», повторный preview помечает импорт `existing`.
 */
const SUBJECT = "Предметы/Демо-курс";

interface MockFile {
  relativePath: string;
  category: PlannedFile["category"];
  homework: boolean;
  confidence: number;
  reason: string;
}

const FILES: MockFile[] = [
  { relativePath: `${SUBJECT}/Неделя 01/01 Лекции/Лекция 1.pdf`, category: "lecture", homework: false, confidence: 0.95, reason: "Папка «01 Лекции»" },
  { relativePath: `${SUBJECT}/Неделя 01/03 ДЗ/ДЗ 1.pdf`, category: "homework_task", homework: true, confidence: 0.8, reason: "Документ с условием задания" },
  { relativePath: `${SUBJECT}/Неделя 01/03 ДЗ/week1.xlsx`, category: "homework_draft", homework: true, confidence: 0.5, reason: "Тип файла похож на работу, проверьте категорию" },
];

const SKIPPED = [
  { relativePath: `${SUBJECT}/00 Общие материалы/Программа.pdf`, reason: "subject_level" as const, detail: "Общие материалы предмета пока не поддерживаются: файл должен относиться к неделе" },
  { relativePath: `${SUBJECT}/Неделя 01/01 Лекции/README.md`, reason: "obsidian_note" as const, detail: "Служебная заметка Obsidian" },
];

export class MockAdoption {
  private seq = 0;
  private readonly plans = new Map<string, { preview: VaultAdoptionPreview; status: AdoptionPlanStatus }>();
  private readonly imported = new Set<string>();

  constructor(private readonly wait: (ms: number) => Promise<void>) {}

  async preview(): Promise<VaultAdoptionPreview> {
    await this.wait(120);
    const id = `adoption-${++this.seq}`;
    const done = (path: string): AdoptionStatus => (this.imported.has(path) ? "existing" : "new");
    const plan: AdoptionPlan = {
      subjects: [{ id: "mock-subject", name: "Демо-курс", shortName: "Демо", folderPath: SUBJECT, status: done(SUBJECT), reason: null }],
      weeks: [{ id: "mock-week-1", subjectId: "mock-subject", number: 1, folderPath: `${SUBJECT}/Неделя 01`, status: done(`${SUBJECT}/Неделя 01`), reason: null }],
      homeworks: [
        { id: "mock-homework-1", weekId: "mock-week-1", title: "Домашнее задание — неделя 1", folderPath: `${SUBJECT}/Неделя 01/03 ДЗ`, status: done(`${SUBJECT}/Неделя 01/03 ДЗ`), reason: null },
      ],
      files: FILES.map((file, index) => ({
        id: `mock-file-${index}`,
        relativePath: file.relativePath,
        name: file.relativePath.split("/").pop() ?? file.relativePath,
        kind: file.relativePath.split(".").pop() ?? "file",
        sizeBytes: 1024 * (index + 1),
        modifiedNs: 0,
        sha256: String(index).repeat(64).slice(0, 64),
        weekId: "mock-week-1",
        homeworkId: file.homework ? "mock-homework-1" : null,
        category: file.category,
        confidence: file.confidence,
        reasons: [file.reason],
        needsReview: file.confidence < 0.7,
        status: done(file.relativePath),
        existingSha256: null,
      })),
      skipped: SKIPPED.map((item) => ({ ...item })),
      truncated: false,
    };
    const count = (items: { status: AdoptionStatus }[], status: AdoptionStatus) => items.filter((item) => item.status === status).length;
    const preview: VaultAdoptionPreview = {
      id,
      workspaceId: "mock-workspace",
      createdAt: new Date().toISOString(),
      expiresAt: new Date(Date.now() + 30 * 60_000).toISOString(),
      summary: {
        subjectsNew: count(plan.subjects, "new"),
        subjectsExisting: count(plan.subjects, "existing"),
        weeksNew: count(plan.weeks, "new"),
        weeksExisting: count(plan.weeks, "existing"),
        homeworksNew: count(plan.homeworks, "new"),
        homeworksExisting: count(plan.homeworks, "existing"),
        filesNew: count(plan.files, "new"),
        filesExisting: count(plan.files, "existing"),
        filesChanged: 0,
        conflicts: 0,
        skipped: plan.skipped.length,
        needsReview: plan.files.filter((file) => file.status === "new" && file.needsReview).length,
        truncated: false,
      },
      plan,
    };
    this.plans.set(id, { preview, status: "previewed" });
    return structuredClone(preview);
  }

  async apply(planId: string, overrides: AdoptionCategoryOverride[], dryRun: boolean): Promise<VaultAdoptionReport> {
    await this.wait(160);
    const entry = this.plans.get(planId);
    if (!entry) throw new BridgeError("not_found", "План подключения vault не найден");
    if (entry.status !== "previewed") throw new BridgeError("conflict", "План уже применён, отменён или завершился ошибкой. Выполните предпросмотр заново.");
    const { plan } = entry.preview;
    const seen = new Set<string>();
    for (const item of overrides) {
      const file = plan.files.find((candidate) => candidate.relativePath === item.relativePath);
      if (!file) throw new BridgeError("not_found", "Файл для ручной категории не найден в плане");
      if (file.status !== "new") throw new BridgeError("blocked", "Категорию можно задать только новому файлу плана");
      if (item.category.startsWith("homework_") && !file.homeworkId) throw new BridgeError("blocked", `Файл ${file.name} не находится в папке ДЗ: категория ДЗ недоступна`);
      if (seen.has(item.relativePath)) throw new BridgeError("blocked", "Повторная ручная категория для файла");
      seen.add(item.relativePath);
    }
    const fresh = <T extends { status: AdoptionStatus }>(items: T[]) => items.filter((item) => item.status === "new");
    const report: VaultAdoptionReport = {
      planId,
      dryRun,
      status: dryRun ? "previewed" : "applied",
      created: {
        subjects: fresh(plan.subjects).length,
        weeks: fresh(plan.weeks).length,
        homeworks: fresh(plan.homeworks).length,
        files: fresh(plan.files).length,
      },
      alreadyPresent: [...plan.subjects, ...plan.weeks, ...plan.homeworks, ...plan.files].filter((item) => item.status === "existing").length,
      changedNotApplied: 0,
      conflictsNotApplied: 0,
      skipped: plan.skipped.length,
      overridesApplied: overrides.length,
    };
    if (!dryRun) {
      entry.status = "applied";
      for (const item of [...plan.subjects, ...plan.weeks, ...plan.homeworks]) this.imported.add(item.folderPath);
      for (const file of plan.files) this.imported.add(file.relativePath);
    }
    return report;
  }

  async cancel(planId: string): Promise<void> {
    const entry = this.plans.get(planId);
    if (!entry || entry.status !== "previewed") throw new BridgeError("conflict", "План уже применён, отменён или не найден");
    entry.status = "cancelled";
  }
}
