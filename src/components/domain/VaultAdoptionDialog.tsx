import { CheckCircle2, FlaskConical, ShieldCheck, TriangleAlert } from "lucide-react";
import { useCallback, useMemo, useState, type ReactNode } from "react";
import { useOps } from "../../app/operations";
import {
  bridge,
  type AdoptionFileCategory,
  type AdoptionSkipReason,
  type PlannedFile,
  type VaultAdoptionPreview,
  type VaultAdoptionReport,
} from "../../lib/bridge";
import { Dialog } from "../ui/Dialog";

export const ADOPTION_CATEGORY_LABEL: Record<AdoptionFileCategory, string> = {
  lecture: "Лекция",
  seminar: "Семинар",
  material: "Доп. материал",
  homework_task: "ДЗ: условие",
  homework_data: "ДЗ: данные",
  homework_draft: "ДЗ: моя работа",
  homework_final: "ДЗ: на сдачу",
};

const WEEK_CATEGORIES: AdoptionFileCategory[] = ["lecture", "seminar", "material"];
const HOMEWORK_CATEGORIES: AdoptionFileCategory[] = ["homework_task", "homework_data", "homework_draft", "homework_final"];

export const SKIP_REASON_LABEL: Record<AdoptionSkipReason, string> = {
  obsidian_note: "Заметки Obsidian",
  service_folder: "Служебные папки",
  subject_level: "Общие материалы предмета (без недели)",
  week_zero: "«Неделя 00»",
  outside_structure: "Вне структуры «Предмет/Неделя NN»",
  symlink: "Символические ссылки (не открываются)",
  special_file: "Не обычные файлы",
  invalid_name: "Недопустимые имена",
  unreadable: "Недоступные папки",
  parent_conflict: "Внутри конфликтующего предмета или недели",
};

/** Категории, доступные файлу: ДЗ — только для файлов из папки «03 ДЗ». */
export function allowedCategories(file: PlannedFile): AdoptionFileCategory[] {
  return file.homeworkId ? [...HOMEWORK_CATEGORIES, ...WEEK_CATEGORIES] : WEEK_CATEGORIES;
}

type Phase =
  | { kind: "idle" }
  | { kind: "preview"; preview: VaultAdoptionPreview; dryRun: VaultAdoptionReport | null }
  | { kind: "done"; report: VaultAdoptionReport };

/**
 * Этап 2: подключение материалов существующего vault.
 * Предпросмотр только читает vault; «Проверить» выполняет dry-run (все проверки,
 * транзакция откатывается); «Подключить» применяет план после повторного хэширования.
 */
export function useVaultAdoption(onApplied?: () => void): { start: () => Promise<void>; busy: boolean; dialog: ReactNode } {
  const { reportError } = useOps();
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [overrides, setOverrides] = useState<Record<string, AdoptionFileCategory>>({});
  const [busy, setBusy] = useState(false);

  const start = useCallback(async () => {
    setBusy(true);
    try {
      const preview = await bridge.previewVaultAdoption();
      setOverrides({});
      setPhase({ kind: "preview", preview, dryRun: null });
    } catch (error) {
      reportError(error, "Подключение материалов: предпросмотр не выполнен");
    } finally {
      setBusy(false);
    }
  }, [reportError]);

  const cancel = async () => {
    if (phase.kind !== "preview") return setPhase({ kind: "idle" });
    const id = phase.preview.id;
    setPhase({ kind: "idle" });
    try {
      await bridge.cancelVaultAdoption(id);
    } catch {
      // План уже применён, завершился ошибкой или устарел — отмена не требуется.
    }
  };

  const run = async (dryRun: boolean) => {
    if (phase.kind !== "preview") return;
    setBusy(true);
    const list = Object.entries(overrides).map(([relativePath, category]) => ({ relativePath, category }));
    try {
      const report = await bridge.applyVaultAdoption(phase.preview.id, list, dryRun);
      if (dryRun) {
        setPhase({ ...phase, dryRun: report });
      } else {
        setPhase({ kind: "done", report });
        onApplied?.();
      }
    } catch (error) {
      reportError(error, dryRun ? "Проверка плана не пройдена" : "Материалы не подключены");
      // Изменение vault или базы завершает план: нужен новый предпросмотр.
      if (!dryRun) setPhase({ kind: "idle" });
    } finally {
      setBusy(false);
    }
  };

  const dialog =
    phase.kind === "preview" ? (
      <Dialog
        open
        onClose={() => void cancel()}
        title="Подключить материалы из vault"
        description="Vault только прочитан, ничего не изменено. Файлы остаются на своих местах: «Контур» лишь запоминает их пути и SHA-256."
        extraWide
        footer={
          <>
            <button className="btn btn--ghost" disabled={busy} onClick={() => void cancel()} data-close>
              Отмена
            </button>
            <button className="btn" disabled={busy} aria-busy={busy} onClick={() => void run(true)}>
              <FlaskConical /> Проверить без записи
            </button>
            <button className="btn btn--primary" disabled={busy || phase.preview.summary.filesNew + phase.preview.summary.subjectsNew + phase.preview.summary.weeksNew === 0} aria-busy={busy} onClick={() => void run(false)}>
              <ShieldCheck /> Подключить
            </button>
          </>
        }
      >
        <AdoptionPreviewBody preview={phase.preview} dryRun={phase.dryRun} overrides={overrides} onOverride={(path, category) => setOverrides((prev) => withOverride(prev, phase.preview, path, category))} />
      </Dialog>
    ) : phase.kind === "done" ? (
      <Dialog open onClose={() => setPhase({ kind: "idle" })} title="Материалы подключены" footer={<button className="btn btn--primary" onClick={() => setPhase({ kind: "idle" })}>Готово</button>}>
        <ReportSummary report={phase.report} />
      </Dialog>
    ) : null;

  return { start, busy, dialog };
}

/** Ручная категория хранится только если отличается от предложенной. */
function withOverride(prev: Record<string, AdoptionFileCategory>, preview: VaultAdoptionPreview, path: string, category: AdoptionFileCategory): Record<string, AdoptionFileCategory> {
  const next = { ...prev };
  const suggested = preview.plan.files.find((file) => file.relativePath === path)?.category;
  if (category === suggested) delete next[path];
  else next[path] = category;
  return next;
}

function shortPath(path: string): string {
  return path.replace(/^Предметы\//, "");
}

export function AdoptionPreviewBody({
  preview,
  dryRun,
  overrides,
  onOverride,
}: {
  preview: VaultAdoptionPreview;
  dryRun: VaultAdoptionReport | null;
  overrides: Record<string, AdoptionFileCategory>;
  onOverride: (relativePath: string, category: AdoptionFileCategory) => void;
}) {
  const { summary, plan } = preview;
  const newFiles = plan.files.filter((file) => file.status === "new");
  const review = newFiles.filter((file) => file.needsReview);
  const changed = plan.files.filter((file) => file.status === "changed");
  const conflicts = [...plan.subjects, ...plan.weeks, ...plan.homeworks].filter((item) => item.status === "conflict");
  const skippedGroups = useMemo(() => {
    const groups = new Map<AdoptionSkipReason, string[]>();
    for (const item of plan.skipped) groups.set(item.reason, [...(groups.get(item.reason) ?? []), item.relativePath]);
    return [...groups.entries()];
  }, [plan.skipped]);

  return (
    <div className="adoption" data-testid="adoption-preview">
      <dl className="adoption__summary">
        <Stat label="Предметы" fresh={summary.subjectsNew} existing={summary.subjectsExisting} />
        <Stat label="Недели" fresh={summary.weeksNew} existing={summary.weeksExisting} />
        <Stat label="ДЗ" fresh={summary.homeworksNew} existing={summary.homeworksExisting} />
        <Stat label="Файлы" fresh={summary.filesNew} existing={summary.filesExisting} />
      </dl>
      <p className="field__hint">
        Сроки ДЗ и даты недель в vault не хранятся, поэтому подключаются как «не указаны» — их можно будет задать позже. Ничего не выдумывается.
      </p>

      {summary.truncated && (
        <div className="notice notice--warn">
          <TriangleAlert /> Vault слишком большой: обход остановлен на лимите, план неполный.
        </div>
      )}

      {review.length > 0 && (
        <section>
          <h3 className="adoption__h">Проверьте категорию ({review.length})</h3>
          <p className="field__hint">Категория определена неуверенно. Ваш выбор имеет приоритет над автоматическим.</p>
          <ul className="adoption__files">
            {review.map((file) => (
              <CategoryRow key={file.relativePath} file={file} value={overrides[file.relativePath] ?? file.category} onChange={(category) => onOverride(file.relativePath, category)} />
            ))}
          </ul>
        </section>
      )}

      {changed.length > 0 && (
        <div className="notice notice--warn">
          <TriangleAlert /> {changed.length} уже подключённых файлов изменились в vault. Они не обновляются этим планом: запись в базе остаётся прежней.
        </div>
      )}

      {conflicts.length > 0 && (
        <section>
          <h3 className="adoption__h">Не будет подключено: конфликт с базой ({conflicts.length})</h3>
          <ul className="adoption__list">
            {conflicts.map((item) => (
              <li key={item.folderPath}>
                <span className="mono">{shortPath(item.folderPath)}</span> — {item.reason}
              </li>
            ))}
          </ul>
        </section>
      )}

      {newFiles.length > 0 && (
        <details>
          <summary>Все новые файлы ({newFiles.length})</summary>
          <ul className="adoption__list">
            {newFiles.map((file) => (
              <li key={file.relativePath}>
                <span className="mono">{shortPath(file.relativePath)}</span> · {ADOPTION_CATEGORY_LABEL[overrides[file.relativePath] ?? file.category]}
              </li>
            ))}
          </ul>
        </details>
      )}

      {skippedGroups.length > 0 && (
        <details>
          <summary>Пропущено ({summary.skipped}) — файлы не изменяются и не подключаются</summary>
          {skippedGroups.map(([reason, paths]) => (
            <div key={reason} className="adoption__skip">
              <strong>
                {SKIP_REASON_LABEL[reason]} · {paths.length}
              </strong>
              <ul className="adoption__list">
                {paths.map((path) => (
                  <li key={path} className="mono">
                    {shortPath(path)}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </details>
      )}

      {dryRun && (
        <div className="notice notice--ok" data-testid="adoption-dry-run">
          <CheckCircle2 /> Проверка без записи пройдена: файлы перечитаны, SHA-256 совпали, транзакция откатана.
          <ReportSummary report={dryRun} />
        </div>
      )}
      <p className="field__hint">План действует до {new Date(preview.expiresAt).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" })}.</p>
    </div>
  );
}

function Stat({ label, fresh, existing }: { label: string; fresh: number; existing: number }) {
  return (
    <div className="adoption__stat">
      <dt>{label}</dt>
      <dd>
        <span className="num">+{fresh}</span>
        {existing > 0 && <span className="faint"> · уже есть {existing}</span>}
      </dd>
    </div>
  );
}

function CategoryRow({ file, value, onChange }: { file: PlannedFile; value: AdoptionFileCategory; onChange: (category: AdoptionFileCategory) => void }) {
  return (
    <li className="adoption__file">
      <div>
        <div className="mono">{shortPath(file.relativePath)}</div>
        <div className="field__hint">{file.reasons.join(" · ")}</div>
      </div>
      <select className="select select--sm" aria-label={`Категория файла ${file.name}`} value={value} onChange={(event) => onChange(event.target.value as AdoptionFileCategory)}>
        {allowedCategories(file).map((category) => (
          <option key={category} value={category}>
            {ADOPTION_CATEGORY_LABEL[category]}
          </option>
        ))}
      </select>
    </li>
  );
}

export function ReportSummary({ report }: { report: VaultAdoptionReport }) {
  const { created } = report;
  return (
    <ul className="adoption__list" data-testid="adoption-report">
      <li>
        {report.dryRun ? "Будет создано" : "Создано"}: предметов {created.subjects}, недель {created.weeks}, ДЗ {created.homeworks}, файлов {created.files}
      </li>
      {report.alreadyPresent > 0 && <li>Уже было подключено: {report.alreadyPresent}</li>}
      {report.overridesApplied > 0 && <li>Ручных категорий: {report.overridesApplied}</li>}
      {report.changedNotApplied > 0 && <li>Изменённые файлы не обновлены: {report.changedNotApplied}</li>}
      {report.conflictsNotApplied > 0 && <li>Конфликты не применены: {report.conflictsNotApplied}</li>}
      {report.skipped > 0 && <li>Пропущено: {report.skipped}</li>}
    </ul>
  );
}
