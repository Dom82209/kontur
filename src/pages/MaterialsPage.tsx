import { FileSearch, RefreshCw, ScanText, Square, TriangleAlert } from "lucide-react";
import { useMemo, useState } from "react";
import { useOps } from "../app/operations";
import { useToast } from "../app/toasts";
import { CorpusPanel } from "../components/domain/CorpusPanel";
import { ExtractionViewer } from "../components/domain/ExtractionViewer";
import { FileChip } from "../components/domain/FileChip";
import { EmptyState, ErrorState, SkeletonRows } from "../components/ui/States";
import { bridge, newOperationId, type ExtractionBatchReport, type MaterialExtractionItem, type MaterialRef } from "../lib/bridge";
import { BATCH_OUTCOME, EXTRACTION_FORMAT, EXTRACTION_STATUS, formatChars, materialKey } from "../lib/extraction";
import { formatBytes, plural } from "../lib/format";
import { useBridgeEvent } from "../lib/hooks/useBridgeEvent";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";

interface Run {
  operationId: string;
  total: number;
  completed: number;
  name: string | null;
  cancelling: boolean;
  report: ExtractionBatchReport | null;
}

type Filter = "all" | "pending" | "problems";

/**
 * Материалы и извлечение текста (этап 7): локальный разбор PDF, DOCX, XLSX,
 * PPTX, текстовых файлов и изображений в нормализованный корпус. Оригиналы не
 * изменяются; результаты привязаны к SHA-256 и переиспользуются из cache.
 */
export function MaterialsPage() {
  const toast = useToast();
  const { reportError } = useOps();
  const q = useBridgeQuery(() => bridge.listMaterialExtractions(), [], ["extraction:updated", "inbox:updated", "inbox:file-added"]);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [run, setRun] = useState<Run | null>(null);
  const [viewer, setViewer] = useState<MaterialExtractionItem | null>(null);
  const [filter, setFilter] = useState<Filter>("all");

  const items = q.data ?? [];
  const visible = useMemo(
    () =>
      items.filter((item) => {
        const status = item.extraction?.status;
        if (filter === "pending") return !item.extraction || item.extraction.stale;
        if (filter === "problems") return !!status && status !== "complete";
        return true;
      }),
    [items, filter],
  );
  const checkedRefs: MaterialRef[] = items.filter((i) => checked.has(materialKey(i.material))).map((i) => i.material);
  const running = !!run && !run.report;
  const allChecked = visible.length > 0 && visible.every((i) => checked.has(materialKey(i.material)));

  useBridgeEvent("extraction:progress", (p) => {
    setRun((r) => (r && r.operationId === p.operationId && !r.report ? { ...r, completed: p.completed, total: p.total, name: p.name } : r));
  });

  const toggle = (ref: MaterialRef) =>
    setChecked((s) => {
      const next = new Set(s);
      const k = materialKey(ref);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });

  const analyze = async (materials: MaterialRef[], force: boolean) => {
    if (materials.length === 0 || running) return;
    const operationId = newOperationId();
    setRun({ operationId, total: materials.length, completed: 0, name: null, cancelling: false, report: null });
    try {
      const result = await bridge.analyzeMaterials(materials, { force, operationId });
      setRun((r) => (r?.operationId === operationId ? { ...r, completed: r.total, report: result.value } : r));
      const { extracted, cached, failed, stoppedByUser } = result.value;
      if (!stoppedByUser && failed === 0) toast.push({ tone: "ok", title: `Обработано ${extracted + cached} ${plural(extracted + cached, "материал", "материала", "материалов")}`, text: cached > 0 ? `Из cache без повторного разбора: ${cached}` : undefined });
    } catch (e) {
      setRun(null);
      reportError(e, "Извлечение не запущено");
    }
  };

  const cancel = async () => {
    if (!run || run.report) return;
    setRun({ ...run, cancelling: true });
    try {
      await bridge.cancelExtraction(run.operationId);
    } catch (e) {
      reportError(e, "Не удалось остановить");
    }
  };

  const analyzeOne = async (item: MaterialExtractionItem) => {
    try {
      await bridge.analyzeMaterial(item.material);
    } catch (e) {
      reportError(e, `Текст из «${item.name}» не извлечён`);
    }
  };

  const counts = useMemo(() => {
    const done = items.filter((i) => i.extraction && !i.extraction.stale && (i.extraction.status === "complete" || i.extraction.status === "partial")).length;
    return { done, total: items.length };
  }, [items]);

  return (
    <div className="materials">
      <header className="pagehead">
        <div>
          <span className="eyebrow">Материалы</span>
          <h1 className="pagehead__title">{items.length > 0 ? `Текст извлечён: ${counts.done} из ${counts.total}` : "Извлечение текста"}</h1>
          <p className="muted">PDF, DOCX, XLSX, PPTX, TXT, MD, CSV, TSV, ноутбуки и изображения разбираются на этом компьютере. Оригиналы только читаются, результат привязан к содержимому файла.</p>
        </div>
      </header>

      <div className="materials__layout">
        <section className="panel" aria-label="Материалы">
          <div className="toolbar">
            <label className="toolbar__check">
              <input
                type="checkbox"
                className="checkbox"
                checked={allChecked}
                onChange={() =>
                  setChecked((s) => {
                    const next = new Set(s);
                    for (const i of visible) {
                      if (allChecked) next.delete(materialKey(i.material));
                      else next.add(materialKey(i.material));
                    }
                    return next;
                  })
                }
                aria-label="Выбрать все"
                disabled={visible.length === 0}
              />
              <span className="faint">{checked.size > 0 ? `Отмечено: ${checked.size}` : "Выбрать все"}</span>
            </label>
            <select className="select select--sm" value={filter} onChange={(e) => setFilter(e.target.value as Filter)} aria-label="Фильтр">
              <option value="all">Все</option>
              <option value="pending">Не извлечено</option>
              <option value="problems">С предупреждениями</option>
            </select>
            <div className="toolbar__actions">
              {running ? (
                <button className="btn btn--sm" onClick={() => void cancel()} disabled={run.cancelling}>
                  <Square /> {run.cancelling ? "Останавливается…" : "Остановить"}
                </button>
              ) : (
                <>
                  <button className="btn btn--sm btn--ghost" disabled={checkedRefs.length === 0} onClick={() => void analyze(checkedRefs, true)} title="Разобрать заново, не используя cache">
                    <RefreshCw /> Перестроить
                  </button>
                  <button className="btn btn--sm btn--primary" disabled={checkedRefs.length === 0} onClick={() => void analyze(checkedRefs, false)}>
                    <ScanText /> Извлечь текст{checkedRefs.length > 0 && <span className="num"> · {checkedRefs.length}</span>}
                  </button>
                </>
              )}
            </div>
          </div>

          {run && (
            <div className="materials__run" aria-live="polite">
              <div className="meter" role="progressbar" aria-valuemin={0} aria-valuemax={run.total} aria-valuenow={run.completed}>
                <div className="meter__fill" style={{ transform: `scaleX(${Math.max(0.03, run.completed / Math.max(1, run.total))})` }} />
              </div>
              {run.report ? (
                <div className="materials__report">
                  <span>
                    Извлечено: {run.report.extracted} · из cache: {run.report.cached} · ошибок: {run.report.failed}
                    {run.report.stoppedByUser && ` · остановлено: ${run.report.cancelled}`}
                  </span>
                  {run.report.items
                    .filter((i) => i.outcome === "failed")
                    .map((i) => (
                      <span key={materialKey(i.material)} className="materials__fail">
                        <TriangleAlert size={12} /> {i.name}: {i.error?.message ?? BATCH_OUTCOME[i.outcome]}
                      </span>
                    ))}
                  <button className="btn btn--sm btn--ghost" onClick={() => setRun(null)}>
                    Скрыть
                  </button>
                </div>
              ) : (
                <span className="faint mono">
                  {Math.min(run.completed, run.total)} из {run.total}
                  {run.name ? ` · ${run.name}` : ""}
                </span>
              )}
            </div>
          )}

          {q.error && !q.data ? (
            <ErrorState message={q.error} onRetry={() => void q.reload()} />
          ) : q.loading && !q.data ? (
            <SkeletonRows count={5} />
          ) : visible.length === 0 ? (
            <EmptyState icon={<FileSearch />} title={items.length === 0 ? "Материалов пока нет" : "Ничего не найдено"} text={items.length === 0 ? "Импортируйте файлы во «Входящие» — здесь можно будет извлечь из них текст." : "Измените фильтр."} />
          ) : (
            <ul className="mrows" role="list">
              {visible.map((item) => {
                const x = item.extraction;
                const status = x?.status ? EXTRACTION_STATUS[x.status] : null;
                const k = materialKey(item.material);
                return (
                  <li key={k} className="mrow">
                    <input type="checkbox" className="checkbox" checked={checked.has(k)} onChange={() => toggle(item.material)} aria-label={`Отметить ${item.name}`} />
                    <button className="mrow__file" onClick={() => setViewer(item)} aria-label={`Открыть извлечённый текст: ${item.name}`}>
                      <FileChip id={`m-${k}`} name={item.name} kind={item.fileKind} sub={`${item.context} · ${formatBytes(item.sizeBytes)}`} />
                    </button>
                    <div className="mrow__status">
                      {x && status ? (
                        <>
                          <span className={`badge ${status.tone}`} title={x.statusMessage ?? status.hint}>
                            {status.label}
                          </span>
                          {x.format && <span className="faint">{EXTRACTION_FORMAT[x.format]}</span>}
                          {x.textChars > 0 && <span className="faint">{formatChars(x.textChars)}</span>}
                          {x.warningCount > 0 && (
                            <span className="badge" title="Есть пропущенные или обрезанные части">
                              <TriangleAlert /> {x.warningCount}
                            </span>
                          )}
                          {x.stale && <span className="badge badge--warn">файл изменился</span>}
                        </>
                      ) : (
                        <span className="faint">Не извлекалось</span>
                      )}
                    </div>
                    <div className="mrow__actions">
                      <button className="btn btn--sm btn--ghost" onClick={() => void analyzeOne(item)} disabled={running} aria-label={`Извлечь текст: ${item.name}`}>
                        <ScanText />
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </section>

        <aside className="materials__side">
          <CorpusPanel checked={checkedRefs} />
        </aside>
      </div>

      <ExtractionViewer material={viewer?.material ?? null} name={viewer?.name ?? ""} onClose={() => setViewer(null)} />
    </div>
  );
}
