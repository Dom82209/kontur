import { Archive, CircleAlert, Info, Link2, TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useOps } from "../../app/operations";
import {
  bridge,
  BridgeError,
  newOperationId,
  type CuImportPhase,
  type DownloadCandidate,
  type ExportEntryPreview,
  type ExportPreview,
  type InboxImportReport,
  type MaterialCategory,
} from "../../lib/bridge";
import { CATEGORY_LABEL, formatBytes, plural } from "../../lib/format";
import { lmsUrlProblem } from "../../lib/lmsLinks";
import { useBridgeEvent } from "../../lib/hooks/useBridgeEvent";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { Confidence } from "../ui/Confidence";
import { Dialog } from "../ui/Dialog";
import { SkeletonRows } from "../ui/States";

const CATEGORIES: MaterialCategory[] = ["lecture", "seminar", "homework_task", "homework_data", "material", "homework_draft"];
const WEEKS = Array.from({ length: 20 }, (_, i) => i + 1);

const PHASE: Record<CuImportPhase, string> = {
  inspect: "Проверка архива",
  extract: "Распаковка во временную папку",
  import: "Импорт во «Входящие»",
  cleanup: "Удаление временных файлов",
};

interface Row {
  selected: boolean;
  subjectId: string | null;
  weekNumber: number | null;
  category: MaterialCategory;
  edited: boolean;
}

interface Props {
  candidate: DownloadCandidate | null;
  onClose: () => void;
  onImported: (report: InboxImportReport) => void;
}

/**
 * Предпросмотр экспорта CU LMS Enhancer: индекс архива без распаковки,
 * предварительная классификация по путям, предупреждения и правка каждой
 * записи до импорта. Импорт кладёт файлы только во «Входящие».
 */
export function CuExportDialog({ candidate, onClose, onImported }: Props) {
  const { reportError } = useOps();
  const subjects = useBridgeQuery(() => bridge.getSubjects(), []);
  const [preview, setPreview] = useState<ExportPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rows, setRows] = useState<Record<string, Row>>({});
  const [courseUrl, setCourseUrl] = useState("");
  const [assignmentUrl, setAssignmentUrl] = useState("");
  const [run, setRun] = useState<{ operationId: string; phase: CuImportPhase; completed: number; total: number; entryName: string | null } | null>(null);

  useEffect(() => {
    setPreview(null);
    setError(null);
    setRows({});
    setRun(null);
    setCourseUrl("");
    setAssignmentUrl("");
    if (!candidate) return;
    let alive = true;
    bridge
      .previewCuExport(candidate.id)
      .then((p) => {
        if (!alive) return;
        setPreview(p);
        setRows(Object.fromEntries(p.entries.map((e) => [e.entryId, { selected: e.selected, subjectId: e.classification.subjectId, weekNumber: e.classification.weekNumber, category: e.classification.category, edited: false }])));
        const subjectIds = new Set(p.entries.map((e) => e.classification.subjectId).filter(Boolean));
        if (subjectIds.size === 1) {
          const known = subjects.data?.find((s) => s.id === [...subjectIds][0])?.courseUrl;
          if (known) setCourseUrl(known);
        }
      })
      .catch((e: unknown) => alive && setError(e instanceof Error ? e.message : "Предпросмотр не выполнен"));
    return () => {
      alive = false;
    };
    // subjects загружаются параллельно; ссылку курса подставляем, если они уже есть.
  }, [candidate?.id]);

  useBridgeEvent("cu-import:progress", (p) => {
    setRun((current) => (current && current.operationId === p.operationId ? { ...current, ...p } : current));
  });

  const selectedEntries = useMemo(() => (preview ? preview.entries.filter((e) => rows[e.entryId]?.selected) : []), [preview, rows]);
  const needsReview = selectedEntries.filter((e) => {
    const r = rows[e.entryId]!;
    return !r.subjectId || !r.weekNumber || r.category === "unknown" || (!r.edited && e.status === "ambiguous");
  }).length;
  const courseProblem = lmsUrlProblem(courseUrl);
  const assignmentProblem = lmsUrlProblem(assignmentUrl);
  const importing = run !== null;

  const patch = (entry: ExportEntryPreview, next: Partial<Row>) =>
    setRows((current) => ({ ...current, [entry.entryId]: { ...current[entry.entryId]!, ...next, edited: next.selected === undefined ? true : current[entry.entryId]!.edited } }));

  const setAll = (selected: boolean) =>
    setRows((current) => Object.fromEntries(Object.entries(current).map(([id, r]) => [id, { ...r, selected: selected && !!preview?.entries.find((e) => e.entryId === id)?.selectable }])));

  const submit = async () => {
    if (!preview || selectedEntries.length === 0 || courseProblem || assignmentProblem) return;
    const operationId = newOperationId();
    setRun({ operationId, phase: "inspect", completed: 0, total: selectedEntries.length, entryName: null });
    try {
      const result = await bridge.importCuExport(
        {
          candidateId: preview.candidateId,
          archiveSha256: preview.archiveSha256,
          entries: selectedEntries.map((e) => {
            const r = rows[e.entryId]!;
            return { entryId: e.entryId, subjectId: r.subjectId, weekNumber: r.weekNumber, category: r.category, edited: r.edited };
          }),
          courseUrl: courseUrl.trim() || null,
          assignmentUrl: assignmentUrl.trim() || null,
        },
        operationId,
      );
      onImported(result.value);
    } catch (e) {
      setRun(null);
      if (e instanceof BridgeError && e.code === "conflict") setError(e.message);
      else reportError(e, "Импорт экспорта не выполнен");
    }
  };

  const footer = preview && (
    <>
      <span className="cuexport__summary faint">
        Выбрано {selectedEntries.length} из {preview.totalFiles}
        {needsReview > 0 && ` · без раскладки останутся ${needsReview}`}
      </span>
      <button className="btn btn--sm btn--ghost" onClick={onClose} disabled={importing}>
        Отмена
      </button>
      <button className="btn btn--sm btn--primary" onClick={() => void submit()} disabled={importing || selectedEntries.length === 0 || !!courseProblem || !!assignmentProblem}>
        {importing ? "Импорт…" : `Импортировать ${selectedEntries.length} ${plural(selectedEntries.length, "файл", "файла", "файлов")}`}
      </button>
    </>
  );

  return (
    <Dialog open={candidate !== null} onClose={importing ? () => undefined : onClose} title="Импорт экспорта ЦУ" description={candidate ? `${candidate.name} · ${formatBytes(candidate.sizeBytes)}` : undefined} extraWide footer={footer}>
      {error ? (
        <div className="notice notice--warn" role="alert">
          <CircleAlert />
          <div>{error}</div>
        </div>
      ) : !preview ? (
        <SkeletonRows count={6} />
      ) : (
        <div className="cuexport">
          <div className="notice">
            <Info />
            <div>
              Файлы попадут во «Входящие» с отметкой происхождения. По предметам ничего не раскладывается автоматически; LMS, вход и отправка работ не затрагиваются. Архив распаковывается только во временную папку, которая удаляется после импорта.
            </div>
          </div>

          {preview.warnings.length > 0 && (
            <ul className="cuexport__warnings" role="list">
              {preview.warnings.map((w) => (
                <li key={w}>
                  <TriangleAlert size={13} /> {w}
                </li>
              ))}
            </ul>
          )}

          <div className="cuexport__meta">
            <span>
              <Archive size={13} /> {preview.format === "zip" ? "ZIP-экспорт" : "Файл экспорта"}
            </span>
            {preview.courseTitle && <span>Курс: «{preview.courseTitle}»</span>}
            <span className="faint">{formatBytes(preview.totalBytes)} после распаковки</span>
            {preview.format === "zip" && (
              <span className="cuexport__bulk">
                <button className="btn btn--sm btn--ghost" onClick={() => setAll(true)} disabled={importing}>
                  Выбрать все
                </button>
                <button className="btn btn--sm btn--ghost" onClick={() => setAll(false)} disabled={importing}>
                  Снять все
                </button>
              </span>
            )}
          </div>

          <div className="cutable" role="table" aria-label="Записи экспорта">
            <div className="cutable__row cutable__row--head" role="row">
              <span role="columnheader">
                <span className="sr-only">Импорт</span>
              </span>
              <span role="columnheader">Файл в архиве</span>
              <span role="columnheader">Предмет</span>
              <span role="columnheader">Неделя</span>
              <span role="columnheader">Тип</span>
              <span role="columnheader">Распознано</span>
            </div>
            {preview.entries.map((entry) => {
              const r = rows[entry.entryId];
              if (!r) return null;
              const complete = !!r.subjectId && !!r.weekNumber && r.category !== "unknown";
              const ambiguous = !complete || (!r.edited && entry.status === "ambiguous");
              return (
                <div key={entry.entryId} className="cutable__row" role="row" data-selected={r.selected} data-disabled={!entry.selectable}>
                  <span role="cell">
                    <input type="checkbox" className="checkbox" checked={r.selected} disabled={!entry.selectable || importing} onChange={(e) => patch(entry, { selected: e.target.checked })} aria-label={`Импортировать ${entry.name}`} />
                  </span>
                  <span role="cell" className="cutable__file">
                    <span className="cutable__name">{entry.name}</span>
                    <span className="cutable__path mono faint" title={entry.path}>
                      {entry.path}
                    </span>
                    {entry.warnings.map((w) => (
                      <span key={w} className="cutable__warn">
                        {w}
                      </span>
                    ))}
                  </span>
                  <span role="cell">
                    <select className="select select--sm" value={r.subjectId ?? ""} disabled={!entry.selectable || importing} onChange={(e) => patch(entry, { subjectId: e.target.value || null })} aria-label={`Предмет: ${entry.name}`}>
                      <option value="">— предмет —</option>
                      {subjects.data
                        ?.filter((s) => !s.archived)
                        .map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.shortName}
                          </option>
                      ))}
                    </select>
                  </span>
                  <span role="cell">
                    <select className="select select--sm" value={r.weekNumber ?? ""} disabled={!entry.selectable || importing} onChange={(e) => patch(entry, { weekNumber: e.target.value ? Number(e.target.value) : null })} aria-label={`Неделя: ${entry.name}`}>
                      <option value="">—</option>
                      {WEEKS.map((w) => (
                        <option key={w} value={w}>
                          {w}
                        </option>
                      ))}
                    </select>
                  </span>
                  <span role="cell">
                    <select className="select select--sm" value={r.category === "unknown" ? "" : r.category} disabled={!entry.selectable || importing} onChange={(e) => patch(entry, { category: (e.target.value || "unknown") as MaterialCategory })} aria-label={`Тип: ${entry.name}`}>
                      <option value="">—</option>
                      {CATEGORIES.map((k) => (
                        <option key={k} value={k}>
                          {CATEGORY_LABEL[k]}
                        </option>
                      ))}
                    </select>
                  </span>
                  <span role="cell" className="cutable__status" title={entry.classification.reasons.join("\n")}>
                    {r.edited ? <span className="badge">вручную</span> : <Confidence value={entry.classification.confidence} />}
                    {ambiguous && entry.selectable && <span className="badge badge--warn">уточнить</span>}
                  </span>
                </div>
              );
            })}
          </div>

          {preview.ignored.length > 0 && (
            <details className="cuexport__ignored">
              <summary className="faint">Пропущено служебных файлов: {preview.ignored.length}</summary>
              <ul className="mono faint">
                {preview.ignored.map((path) => (
                  <li key={path}>{path}</li>
                ))}
              </ul>
            </details>
          )}

          <div className="cuexport__links">
            <label className="field">
              <span className="field__label">
                <Link2 size={12} /> Ссылка на курс (необязательно)
              </span>
              <input className="input mono" placeholder="https://my.centraluniversity.ru/…" value={courseUrl} onChange={(e) => setCourseUrl(e.target.value)} disabled={importing} spellCheck={false} />
              {courseProblem && <span className="field__error">{courseProblem}</span>}
            </label>
            <label className="field">
              <span className="field__label">
                <Link2 size={12} /> Ссылка на задание (необязательно)
              </span>
              <input className="input mono" placeholder="https://my.centraluniversity.ru/…" value={assignmentUrl} onChange={(e) => setAssignmentUrl(e.target.value)} disabled={importing} spellCheck={false} />
              {assignmentProblem && <span className="field__error">{assignmentProblem}</span>}
            </label>
            <p className="field__hint">Ссылки сохраняются как метаданные и открываются во внешнем браузере. Логин, пароль и cookies LMS «Контур» не запрашивает и не хранит.</p>
          </div>

          {run && (
            <div className="cuexport__progress" aria-live="polite">
              <div className="meter" role="progressbar" aria-valuemin={0} aria-valuemax={run.total} aria-valuenow={run.completed}>
                <div className="meter__fill" style={{ transform: `scaleX(${Math.max(0.03, run.phase === "cleanup" ? 1 : run.completed / Math.max(1, run.total))})` }} />
              </div>
              <span className="faint mono">
                {PHASE[run.phase]}
                {run.entryName ? ` · ${run.entryName}` : ""} · {Math.min(run.completed, run.total)} из {run.total}
              </span>
            </div>
          )}
        </div>
      )}
    </Dialog>
  );
}
