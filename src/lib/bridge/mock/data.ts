import { applyNamePattern } from "../../naming";
import { kindFromName, pad2 } from "../../format";
import { CATEGORY_FOLDER, INBOX_FOLDER, homeworkFolderForWeek, weekFolderForSubject } from "../../paths";

export { CATEGORY_FOLDER, kindFromName, pad2 };
import type {
  AutomationRules,
  FileRef,
  Homework,
  HomeworkVersion,
  InboxFile,
  ProviderStatus,
  Subject,
  VersionSection,
  Week,
} from "../types";

/* Мок-данные строятся относительно текущей даты, чтобы дедлайны всегда были «живыми». */

export const CURRENT_WEEK = 8;

const DAY = 86_400_000;

export function at(daysFromNow: number, hour = 12, minute = 0): string {
  const d = new Date(Date.now() + daysFromNow * DAY);
  d.setHours(hour, minute, 0, 0);
  return d.toISOString();
}

function mondayOf(date: Date): Date {
  const d = new Date(date);
  const day = (d.getDay() + 6) % 7;
  d.setDate(d.getDate() - day);
  d.setHours(0, 0, 0, 0);
  return d;
}

export const SEMESTER_START = (() => {
  const m = mondayOf(new Date());
  m.setDate(m.getDate() - (CURRENT_WEEK - 1) * 7);
  return m;
})();

export function weekRange(n: number): { startsAt: string; endsAt: string } {
  const s = new Date(SEMESTER_START.getTime() + (n - 1) * 7 * DAY);
  const e = new Date(s.getTime() + 7 * DAY - 1000);
  return { startsAt: s.toISOString(), endsAt: e.toISOString() };
}

type SubjectSeed = Pick<Subject, "id" | "name" | "shortName" | "currentWeek" | "weekCount" | "courseUrl">;

/** Поля этапа 3: папка, ключевые слова (заполняются из правил), архив, ревизия. */
function seedSubject(seed: SubjectSeed): Subject {
  return { ...seed, folderPath: `Предметы/${seed.name}`, keywords: [], archived: false, mergedIntoId: null, revision: 0 };
}

export const SUBJECTS: Subject[] = ([
  { id: "eng", name: "Английский", shortName: "Англ", currentWeek: 8, weekCount: 8, courseUrl: null },
  { id: "bstudio", name: "Бизнес-студия", shortName: "Б-студия", currentWeek: 8, weekCount: 8, courseUrl: null },
  { id: "linal", name: "Линейная алгебра", shortName: "Линал", currentWeek: 8, weekCount: 8, courseUrl: null },
  { id: "matan", name: "Математический анализ", shortName: "Матан", currentWeek: 8, weekCount: 8, courseUrl: null },
  { id: "matan-adv", name: "Математический анализ, продвинутый", shortName: "Матан+", currentWeek: 8, weekCount: 7, courseUrl: null },
  { id: "meow", name: "Мяу", shortName: "Мяу", currentWeek: 8, weekCount: 6, courseUrl: null },
  { id: "org", name: "ОРГ", shortName: "ОРГ", currentWeek: 8, weekCount: 8, courseUrl: null },
  { id: "oba", name: "Основы бизнес-аналитики", shortName: "ОБА", currentWeek: 8, weekCount: 8, courseUrl: "https://my.centraluniversity.ru/learn/courses/view/actual/oba" },
  { id: "oba-adv", name: "Основы бизнес-аналитики, продвинутый", shortName: "ОБА+", currentWeek: 8, weekCount: 7, courseUrl: null },
  { id: "stat", name: "Статистика", shortName: "Стат", currentWeek: 8, weekCount: 8, courseUrl: null },
  { id: "stress", name: "Стресс-менеджмент", shortName: "Стресс", currentWeek: 8, weekCount: 5, courseUrl: null },
] satisfies SubjectSeed[]).map(seedSubject);

export function subjectName(id: string | null): string {
  return SUBJECTS.find((s) => s.id === id)?.name ?? "Без предмета";
}

const TOPICS: Record<string, string[]> = {
  eng: ["Introductions", "Work & study", "Describing data", "Presentations", "Argument", "Formal emails", "Debate", "Essay structure"],
  bstudio: ["Команды", "Проблема клиента", "CustDev", "Гипотезы", "MVP", "Метрики", "Юнит-экономика", "Питч"],
  linal: ["Векторы", "Матрицы", "Определители", "СЛАУ", "Ранг", "Линейные пространства", "Линейные операторы", "Собственные значения"],
  matan: ["Множества", "Последовательности", "Пределы", "Непрерывность", "Производная", "Исследование функций", "Интеграл", "Ряды"],
  "matan-adv": ["Аксиоматика ℝ", "Теорема Больцано", "Равномерная непрерывность", "Дифференцируемость", "Формула Тейлора", "Интеграл Римана", "Несобственные интегралы"],
  meow: ["Знакомство", "Наблюдение", "Интервью", "Эссе", "Обсуждение", "Проект"],
  org: ["Организация как система", "Структуры", "Мотивация", "Лидерство", "Коммуникации", "Изменения", "Культура", "Конфликты"],
  oba: ["Данные в Excel", "Формулы", "Фильтры и сортировка", "ВПР и ИНДЕКС", "Диаграммы", "Описательная статистика", "Сводные таблицы", "Дашборды"],
  "oba-adv": ["Power Query", "Модель данных", "DAX", "Python для анализа", "pandas", "Визуализация", "Когортный анализ"],
  stat: ["Выборка", "Распределения", "Описательные меры", "Вероятность", "Нормальное распределение", "ЦПТ", "Доверительные интервалы", "Проверка гипотез"],
  stress: ["Природа стресса", "Сон", "Дыхание", "Планирование", "Границы"],
};

export function weekFolder(subjectId: string, n: number): string {
  return weekFolderForSubject(subjectName(subjectId), n);
}

let fileSeq = 1000;
export function nextFileId(): string {
  fileSeq += 1;
  return `f${fileSeq}`;
}

export function makeFile(name: string, folder: string, modifiedAt: string, sizeBytes: number): FileRef {
  return { id: nextFileId(), name, kind: kindFromName(name), sizeBytes, modifiedAt, relativePath: `${folder}/${name}` };
}

function buildWeeks(subject: Subject): Week[] {
  const topics = TOPICS[subject.id] ?? [];
  const weeks: Week[] = [];
  for (let n = 1; n <= subject.weekCount; n += 1) {
    const { startsAt, endsAt } = weekRange(n);
    const folder = weekFolder(subject.id, n);
    const daysAgo = (CURRENT_WEEK - n) * 7;
    const topic = topics[n - 1] ?? null;
    const lectures = subject.id === "eng" || subject.id === "meow" ? [] : [makeFile(`Лекция ${n}. ${topic ?? "Материалы"}.pdf`, `${folder}/01 Лекции`, at(-daysAgo - 1, 10), 1_200_000 + n * 90_000)];
    const seminars = subject.id === "stress" ? [] : [makeFile(`Семинар ${n}.${subject.id === "oba" || subject.id === "stat" ? "xlsx" : "pdf"}`, `${folder}/02 Семинары`, at(-daysAgo, 15), 240_000 + n * 11_000)];
    const materials = n % 3 === 0 ? [makeFile(`Доп. материалы — ${topic ?? "неделя"}.pdf`, `${folder}/04 Дополнительно`, at(-daysAgo + 1, 19), 880_000)] : [];
    weeks.push({ id: `${subject.id}-w${n}`, subjectId: subject.id, number: n, startsAt, endsAt, datesSource: "semester", topic, lectures, seminars, materials, homeworkIds: [] });
  }
  return weeks;
}

export function buildAllWeeks(): Map<string, Week[]> {
  const map = new Map<string, Week[]>();
  for (const s of SUBJECTS) map.set(s.id, buildWeeks(s));
  return map;
}

const SECTION_TITLES: Record<string, string[]> = {
  oba: ["Лист «Данные»", "Сводная по регионам", "Сводная по месяцам", "Диаграмма", "Выводы"],
  stat: ["Задача 1", "Задача 2", "Задача 3", "Задача 4", "Ответы"],
  eng: ["Introduction", "Argument 1", "Argument 2", "Counterargument", "Conclusion"],
  "oba-adv": ["Загрузка данных", "Когорты", "Retention-таблица", "Heatmap", "Выводы"],
};

let hashSeq = 1;
function hash(): string {
  hashSeq += 1;
  return `h${hashSeq.toString(16)}`;
}

export function buildSections(subjectId: string, prev: VersionSection[] | null): VersionSection[] {
  const titles = SECTION_TITLES[subjectId] ?? ["Задача 1", "Задача 2", "Задача 3", "Выводы"];
  if (!prev) {
    // Первая версия — не все разделы готовы.
    return titles.slice(0, Math.max(2, titles.length - 2)).map((t, i) => ({ id: `s${i}`, title: t, hash: hash() }));
  }
  const next: VersionSection[] = prev.map((s, i) => (i % 2 === 1 ? { ...s, hash: hash() } : { ...s }));
  const missing = titles.find((t) => !next.some((s) => s.title === t));
  if (missing) next.push({ id: `s${titles.indexOf(missing)}`, title: missing, hash: hash() });
  return next;
}

function versions(hwId: string, subjectId: string, week: number, base: string, ext: string, count: number, startDaysAgo: number): HomeworkVersion[] {
  const out: HomeworkVersion[] = [];
  let sections: VersionSection[] | null = null;
  const notes = ["Первый проход по заданиям", "Исправлены формулы, добавлены пояснения", "Чистовик: оформление и выводы"];
  for (let i = 1; i <= count; i += 1) {
    sections = buildSections(subjectId, sections);
    const fileName = `${base}_v${pad2(i)}.${ext}`;
    out.push({
      id: `${hwId}-v${i}`,
      homeworkId: hwId,
      number: i,
      label: `v${pad2(i)}`,
      fileName,
      sizeBytes: 90_000 + i * 21_000,
      createdAt: at(-startDaysAgo + i - 1, 20 + i, 10),
      sourceFileId: nextFileId(),
      note: notes[i - 1] ?? null,
      sections,
      author: "user",
      promptId: null,
      sourceBundleId: null,
      files: [makeFile(fileName, `${weekFolder(subjectId, week)}/03 ДЗ/02 Работа`, at(-startDaysAgo + i - 1, 20 + i, 10), 90_000 + i * 21_000)],
      frozen: true,
    });
  }
  return out;
}

export const STUDENT = { lastName: "Гончарова", firstName: "Доминика" };

export function finalName(pattern: string, week: number, subjectId: string, ext: string, version = ""): string {
  return applyNamePattern(pattern, { ...STUDENT, week, subject: subjectName(subjectId), version }, ext);
}

export const DEFAULT_FINAL_PATTERN = "{Фамилия}_{Имя}_Неделя{Неделя} ДЗ";

function hw(
  id: string,
  subjectId: string,
  week: number,
  title: string,
  deadline: string,
  stage: Homework["stage"],
  ext: string,
  versionCount: number,
): Homework {
  const folder = homeworkFolderForWeek(weekFolder(subjectId, week));
  const vs = versions(id, subjectId, week, `Работа_Неделя${week}`, ext, versionCount, 4);
  const final = stage === "ready" || stage === "submitted" ? vs[vs.length - 1] ?? null : null;
  return {
    id,
    subjectId,
    weekNumber: week,
    ordinal: 1,
    title,
    deadline,
    deadlineSource: "manual",
    stage,
    taskFiles: [
      makeFile(`Условие ДЗ ${week}.pdf`, `${folder}/01 Условие и данные`, at(-5, 11), 310_000),
      ...(ext === "xlsx" || ext === "ipynb" ? [makeFile(`данные_неделя${week}.csv`, `${folder}/01 Условие и данные`, at(-5, 11, 5), 1_840_000)] : []),
    ],
    versions: vs,
    finalVersionId: final?.id ?? null,
    finalFileName: final ? finalName(DEFAULT_FINAL_PATTERN, week, subjectId, ext) : null,
    submittedAt: stage === "submitted" ? at(-3, 21, 40) : null,
    folderPath: folder,
    workflowStage: stage === "submitted" ? "submitted" : stage === "task" ? "materials_collected" : stage === "ready" ? "teacher_review" : "codex_review",
    assignmentUrl: null,
    revision: 0,
  };
}

export function buildHomework(): Homework[] {
  return [
    hw("hw-stat-7", "stat", 7, "Доверительные интервалы", at(1, 18, 0), "ready", "pdf", 3),
    hw("hw-oba-7", "oba", 7, "Сводные таблицы: анализ продаж", at(2, 23, 59), "in_progress", "xlsx", 2),
    hw("hw-obaadv-7", "oba-adv", 7, "Когортный анализ в pandas", at(3, 23, 59), "in_progress", "ipynb", 1),
    hw("hw-eng-8", "eng", 8, "Essay: Remote work", at(4, 9, 0), "in_progress", "docx", 1),
    hw("hw-matanadv-8", "matan-adv", 7, "Формула Тейлора, листок 7", at(5, 23, 59), "task", "pdf", 0),
    hw("hw-linal-8", "linal", 8, "Собственные значения и векторы", at(6, 23, 59), "task", "pdf", 0),
    hw("hw-matan-6", "matan", 6, "Исследование функций", at(-2, 23, 59), "submitted", "pdf", 2),
  ];
}

export function buildInbox(): InboxFile[] {
  const base = (name: string, sizeBytes: number, minutesAgo: number, origin: InboxFile["origin"]): InboxFile => ({
    id: nextFileId(),
    name,
    kind: kindFromName(name),
    sizeBytes,
    addedAt: new Date(Date.now() - minutesAgo * 60_000).toISOString(),
    origin,
    status: "new",
    classification: null,
    duplicate: null,
    provenance: origin === "lms" ? { source: "cu_export", via: "watcher", archiveName: "Основы бизнес-аналитики — экспорт.zip", entryPath: `Основы бизнес-аналитики/Неделя 8/${name}`, courseUrl: "https://my.centraluniversity.ru/learn/courses/view/actual/oba", assignmentUrl: null } : origin === "downloads" ? { source: "watcher", via: "watcher", archiveName: null, entryPath: null, courseUrl: null, assignmentUrl: null } : { source: "manual", via: "drop", archiveName: null, entryPath: null, courseUrl: null, assignmentUrl: null },
  });
  return [
    base("Лекция_8_Ряды_Тейлора.pdf", 2_310_000, 12, "downloads"),
    base("linal_hw8_условие.pdf", 420_000, 25, "lms"),
    base("seminar7_stat_data.xlsx", 186_000, 40, "downloads"),
    base("scan_0042.pdf", 3_900_000, 64, "downloads"),
    base("Лекция 7. Доверительные интервалы.pdf", 1_830_000, 90, "downloads"),
    base("Кейс_команды_Бизнес-студия.pptx", 5_400_000, 130, "downloads"),
  ];
}

/** Набор для кнопки «Симулировать загрузку» и для drop в браузере без реальных путей. */
export const DEMO_DROP: { name: string; sizeBytes: number }[] = [
  { name: "Лекция 8 — Сводные таблицы ОБА.pdf", sizeBytes: 1_640_000 },
  { name: "English_seminar_week8.docx", sizeBytes: 96_000 },
  { name: "ДЗ8_стат_данные.csv", sizeBytes: 712_000 },
  { name: "IMG_2041.HEIC", sizeBytes: 2_850_000 },
];

/** Сводное состояние демонстрационного провайдера (как проекция backend этапа 9). */
export function mockAvailability(state: ProviderStatus["state"]): ProviderStatus["availability"] {
  if (state === "connected") return "connected";
  if (state === "error" || state === "reauth_required") return "error";
  return "manual";
}

export function buildProviders(): ProviderStatus[] {
  const checked = at(0, 9, 12);
  const report = {
    platform: "macos", runnerAvailable: true, profileVersion: 1, passed: true,
    checks: [{ id: "mock", passed: true, message: "Демонстрационная проверка" }],
    checkedAt: checked,
    explanation: "Демонстрационный режим не запускает внешний CLI.",
  };
  return [
    {
      provider: "codex",
      state: "disconnected",
      accountLabel: null,
      lastCheckedAt: checked,
      errorMessage: null,
      adapter: "mock",
      roles: ["analyst", "reviewer"],
      availability: "manual",
      tasks: ["material_analysis", "initial_prompt", "solution_review", "revision_prompt", "teacher_review"],
      cli: { enabled: false, detected: true, executable: "codex", version: "codex-cli (демо)", supported: true, missingFlags: [], loggedIn: null, checkedAt: null },
      isolation: { capability: { available: true, profileVersion: 1, reason: null }, report, consentProfileVersion: 1, consentValid: true, executionPolicy: "manual_only", blockingReason: null },
      handoff: { appInstalled: false, webUrl: "https://chatgpt.com/" },
    },
    {
      provider: "claude",
      state: "reauth_required",
      accountLabel: "Claude Code CLI (демо)",
      lastCheckedAt: checked,
      errorMessage: "Войдите в официальном CLI: выполните `claude auth login` в Терминале, затем «Проверить».",
      adapter: "mock",
      roles: ["author"],
      availability: "error",
      tasks: [],
      cli: { enabled: true, detected: true, executable: "~/.local/bin/claude", version: "2.1.0 (Claude Code, демо)", supported: true, missingFlags: [], loggedIn: false, checkedAt: checked },
      isolation: { capability: { available: true, profileVersion: 1, reason: null }, report, consentProfileVersion: 1, consentValid: true, executionPolicy: "isolated_automatic", blockingReason: null },
      handoff: { appInstalled: true, webUrl: "https://claude.ai/new" },
    },
  ];
}

export function buildRules(): AutomationRules {
  return {
    inboxPath: INBOX_FOLDER,
    studentName: { ...STUDENT },
    finalNamePattern: DEFAULT_FINAL_PATTERN,
    subjectRules: [
      { subjectId: "eng", keywords: ["англ", "english", "essay", "eng_"] },
      { subjectId: "bstudio", keywords: ["бизнес-студ", "бизнес студ", "business studio", "кейс"] },
      { subjectId: "linal", keywords: ["линал", "линейн", "linal", "матриц"] },
      { subjectId: "matan", keywords: ["матан", "анализ", "тейлор", "ряды", "calculus"] },
      { subjectId: "matan-adv", keywords: ["матан продв", "матан+", "calculus adv"] },
      { subjectId: "meow", keywords: ["мяу", "meow"] },
      { subjectId: "org", keywords: ["орг_", "орг ", "организац"] },
      { subjectId: "oba", keywords: ["оба", "бизнес-аналит", "сводн", "excel"] },
      { subjectId: "oba-adv", keywords: ["оба+", "оба продв", "pandas", "power query"] },
      { subjectId: "stat", keywords: ["стат", "stat", "регресс", "доверит"] },
      { subjectId: "stress", keywords: ["стресс", "stress"] },
    ],
    lowConfidenceThreshold: 0.7,
    confirm: { lowConfidenceRoute: true, markFinal: true, markSubmitted: true, allowOverwrite: false },
    workflow: {
      roles: { analyst: "codex", author: "claude", reviewer: "codex" },
      tasks: { material_analysis: "codex", initial_prompt: "codex", solution_review: "codex", revision_prompt: "codex", teacher_review: "codex" },
      styleThreshold: 4,
      styleProfilePath: "~/Documents/Obsidian/Учёба/ПРОФИЛЬ_СТИЛЯ_ДЗ.md",
      requireTeacherReview: true,
    },
  };
}

export const STYLE_PROFILE_FILE = "ПРОФИЛЬ_СТИЛЯ_ДЗ.md";

/** Прошлые работы пользователя по предмету — эталон стиля. */
export function previousWorks(subjectId: string, week: number): FileRef[] {
  const ext = subjectId === "oba" || subjectId === "oba-adv" ? "xlsx" : "pdf";
  return [week - 2, week - 1]
    .filter((n) => n >= 1)
    .map((n) => makeFile(`${STUDENT.lastName}_${STUDENT.firstName}_Неделя${n} ДЗ.${ext}`, `${weekFolder(subjectId, n)}/03 ДЗ/04 Отправлено`, at(-(week - n) * 7 + 1, 22), 140_000 + n * 3_000));
}
