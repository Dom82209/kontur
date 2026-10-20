import { AnimatePresence, motion } from "motion/react";
import { ArrowRight, CircleCheck, Copy, Inbox, ListChecks, Pencil, TriangleAlert, WandSparkles } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { weekPath } from "../app/nav";
import { useOps } from "../app/operations";
import { useToast } from "../app/toasts";
import { DropZone } from "../components/domain/DropZone";
import { FileChip } from "../components/domain/FileChip";
import { ClassifyWizard } from "../components/domain/ClassifyWizard";
import { ClassificationFields } from "../components/domain/ClassificationFields";
import { Confidence } from "../components/ui/Confidence";
import { EmptyState, ErrorState, SkeletonRows } from "../components/ui/States";
import { bridge, type DroppedItem, type InboxFile, type RoutePreview } from "../lib/bridge";
import { CATEGORY_LABEL, formatBytes, formatRelative, pad2, plural } from "../lib/format";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
import { useSubjectNames } from "../lib/hooks/useSubjectNames";

interface Landed {
  file: InboxFile;
  preview: RoutePreview;
  subjectId: string;
  weekNumber: number;
}

const isReady = (f: InboxFile) => f.status === "classified" && !!f.classification?.subjectId && !!f.classification.weekNumber && f.classification.category !== "unknown";

export function InboxPage() {
  const toast = useToast();
  const { reportError, undo } = useOps();
  const names = useSubjectNames();
  const q = useBridgeQuery(() => bridge.getInboxFiles(), [], ["inbox:file-added", "inbox:classification-updated", "operation:completed"]);
  const files = q.data ?? [];
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [focusId, setFocusId] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [wizard, setWizard] = useState(false);
  const [routing, setRouting] = useState<Set<string>>(new Set());
  const [landed, setLanded] = useState<Landed[]>([]);
  const [preview, setPreview] = useState<RoutePreview | null>(null);

  const ambiguous = files.filter((f) => f.status === "ambiguous");
  const ready = files.filter(isReady);
  const allChecked = files.length > 0 && selected.size === files.length;
  const someChecked = selected.size > 0 && !allChecked;
  const focus = files.find((f) => f.id === focusId) ?? files[0] ?? null;

  // Предпросмотр итогового пути для файла в фокусе.
  useEffect(() => {
    setPreview(null);
    const c = focus?.classification;
    if (!focus || !c?.subjectId || !c.weekNumber || c.category === "unknown") return;
    let alive = true;
    void bridge.previewRoute(focus.id, { subjectId: c.subjectId, weekNumber: c.weekNumber, category: c.category }).then((p) => alive && setPreview(p));
    return () => {
      alive = false;
    };
  }, [focus?.id, focus?.classification?.subjectId, focus?.classification?.weekNumber, focus?.classification?.category, focus]);

  const toggle = (id: string) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const onDrop = async (items: DroppedItem[]) => {
    try {
      const added = await bridge.importDroppedFiles(items);
      toast.push({ tone: "info", title: `Добавлено ${added.length} ${plural(added.length, "файл", "файла", "файлов")}`, text: "Классификация запущена" });
    } catch (e) {
      reportError(e, "Не удалось добавить файлы");
    }
  };

  const route = async (ids: string[]) => {
    if (ids.length === 0) return;
    setRouting(new Set(ids));
    try {
      const res = await bridge.routeFiles(ids, null);
      const moved = files.filter((f) => ids.includes(f.id));
      // Один рендер: файл исчезает из списка и появляется в лотке недели → shared layout transition.
      q.setData((list) => list?.filter((f) => !ids.includes(f.id)) ?? null);
      setLanded((prev) => [
        ...moved.map((f) => ({ file: f, preview: res.value.find((p) => p.fileId === f.id)!, subjectId: f.classification!.subjectId!, weekNumber: f.classification!.weekNumber! })),
        ...prev,
      ].slice(0, 12));
      setSelected((s) => new Set([...s].filter((id) => !ids.includes(id))));
      const dups = res.value.filter((p) => p.duplicate).length;
      toast.push({
        tone: "ok",
        title: res.operation.title,
        text: dups ? `${dups} ${plural(dups, "дубль сохранён", "дубля сохранены", "дублей сохранены")} с новым именем` : res.operation.detail ?? undefined,
        action: { label: "Отменить", run: () => void undo(res.operation.id).then(() => setLanded((l) => l.filter((x) => !ids.includes(x.file.id)))) },
      });
    } catch (e) {
      reportError(e, "Раскладка не выполнена");
    } finally {
      setRouting(new Set());
    }
  };

  const groups = useMemo(() => {
    const map = new Map<string, Landed[]>();
    for (const l of landed) {
      const k = `${l.subjectId}:${l.weekNumber}`;
      map.set(k, [...(map.get(k) ?? []), l]);
    }
    return [...map.entries()];
  }, [landed]);

  return (
    <div className="inbox">
      <header className="pagehead">
        <div>
          <span className="eyebrow">Входящие</span>
          <h1 className="pagehead__title">
            {files.length > 0 ? `${files.length} ${plural(files.length, "файл", "файла", "файлов")} ждут раскладки` : "Всё разложено"}
          </h1>
          <p className="muted">Загрузки из папки наблюдения и перетащенные файлы. Перед раскладкой можно поправить предмет, неделю и тип.</p>
        </div>
      </header>

      <DropZone onFiles={(items) => void onDrop(items)} onDemo={() => void onDrop([])} onError={(error) => reportError(error, "Не удалось выбрать файлы")} />

      <div className="inbox__layout">
        <section className="panel inbox__list" aria-label="Новые файлы">
          <div className="toolbar">
            <label className="toolbar__check">
              <input
                type="checkbox"
                className="checkbox"
                checked={allChecked}
                ref={(el) => {
                  if (el) el.indeterminate = someChecked;
                }}
                onChange={() => setSelected(allChecked ? new Set() : new Set(files.map((f) => f.id)))}
                aria-label="Выбрать все"
                disabled={files.length === 0}
              />
              <span className="faint">{selected.size > 0 ? `Выбрано: ${selected.size}` : "Выбрать все"}</span>
            </label>
            <div className="toolbar__actions">
              {ambiguous.length > 0 && (
                <button className="btn btn--sm" onClick={() => setWizard(true)}>
                  <ListChecks /> Уточнить неясные <span className="badge badge--warn">{ambiguous.length}</span>
                </button>
              )}
              {selected.size > 0 && (
                <button className="btn btn--sm" onClick={() => void route([...selected])} disabled={routing.size > 0}>
                  Разложить выбранные
                </button>
              )}
              <button className="btn btn--sm btn--primary" onClick={() => void route(ready.map((f) => f.id))} disabled={ready.length === 0 || routing.size > 0} title="Раскладываются только уверенно распознанные файлы">
                <WandSparkles /> Разложить автоматически{ready.length > 0 && <span className="num"> · {ready.length}</span>}
              </button>
            </div>
          </div>

          {q.error && !q.data ? (
            <ErrorState message={q.error} onRetry={() => void q.reload()} />
          ) : q.loading && !q.data ? (
            <SkeletonRows count={5} />
          ) : files.length === 0 ? (
            <EmptyState icon={<Inbox />} title="Входящие пусты" text="Скачайте материалы или перетащите файлы в зону выше — «Контур» распознает предмет и неделю." />
          ) : (
            <ul className="inbox__rows" role="list">
              <AnimatePresence initial={false}>
                {files.map((f) => {
                  const c = f.classification;
                  const isEditing = editing === f.id;
                  return (
                    <motion.li
                      key={f.id}
                      layout="position"
                      className="irow"
                      data-focus={focus?.id === f.id}
                      data-status={f.status}
                      data-routing={routing.has(f.id)}
                      initial={{ opacity: 0, y: -6 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, transition: { duration: 0.12 } }}
                      onClick={() => setFocusId(f.id)}
                    >
                      <input type="checkbox" className="checkbox" checked={selected.has(f.id)} onChange={() => toggle(f.id)} onClick={(e) => e.stopPropagation()} aria-label={`Выбрать ${f.name}`} />
                      <button className="irow__file" onClick={() => setFocusId(f.id)} aria-label={`Показать детали: ${f.name}`}>
                        <FileChip id={f.id} name={f.name} kind={f.kind} sub={`${formatBytes(f.sizeBytes)} · ${formatRelative(f.addedAt)}`} />
                      </button>
                      <div className="irow__class">
                        {f.status === "classifying" || f.status === "new" ? (
                          <span className="irow__scanning">
                            <span className="scanline" aria-hidden /> Распознаётся…
                          </span>
                        ) : c ? (
                          <div className="irow__tags">
                            <span className={c.subjectId ? "" : "irow__missing"}>{c.subjectId ? names(c.subjectId) : "Предмет?"}</span>
                            <span className="faint">·</span>
                            <span className={c.weekNumber ? "num" : "irow__missing"}>{c.weekNumber ? `Неделя ${pad2(c.weekNumber)}` : "Неделя?"}</span>
                            <span className="faint">·</span>
                            <span className={c.category === "unknown" ? "irow__missing" : ""}>{CATEGORY_LABEL[c.category]}</span>
                            {c.source === "manual" && <span className="badge">вручную</span>}
                          </div>
                        ) : null}
                      </div>
                      <div className="irow__meta">
                        {f.duplicate && (
                          <span className="badge badge--warn" title={`Уже есть: ${f.duplicate.existingPath}`}>
                            <Copy /> Дубль
                          </span>
                        )}
                        {c && f.status !== "classifying" && <Confidence value={c.confidence} />}
                        {!isEditing && c && (
                          <button
                            className="btn btn--sm btn--ghost btn--icon"
                            aria-label={`Исправить классификацию: ${f.name}`}
                            onClick={(e) => {
                              e.stopPropagation();
                              setEditing(f.id);
                              setFocusId(f.id);
                            }}
                          >
                            <Pencil />
                          </button>
                        )}
                      </div>
                      {isEditing && c && (
                        <motion.div className="irow__edit" initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }}>
                          <ClassificationFields file={f} onDone={() => setEditing(null)} />
                        </motion.div>
                      )}
                    </motion.li>
                  );
                })}
              </AnimatePresence>
            </ul>
          )}
        </section>

        <aside className="inbox__side">
          <section className="panel" aria-label="Предпросмотр">
            <div className="panel__head">
              <h2 className="panel__title">Куда попадёт файл</h2>
            </div>
            <div className="panel__body">
              {!focus ? (
                <p className="faint">Выберите файл в списке.</p>
              ) : (
                <div className="preview">
                  <div className="preview__name">{focus.name}</div>
                  {focus.classification && (
                    <ul className="preview__reasons">
                      {focus.classification.reasons.map((r) => (
                        <li key={r}>{r}</li>
                      ))}
                    </ul>
                  )}
                  {preview ? (
                    <>
                      <div className="path mono selectable">
                        {preview.targetPath.split("/").map((seg, i, arr) => (
                          <span key={`${seg}-${i}`} className={i === arr.length - 1 ? "path__leaf" : "path__seg"}>
                            {seg}
                            {i < arr.length - 1 && <span className="path__sep">/</span>}
                          </span>
                        ))}
                      </div>
                      {preview.createsWeek && <div className="notice"><CircleCheck /> Папка недели будет создана со структурой 01–04.</div>}
                      {preview.duplicate && (
                        <div className="notice notice--warn" role="status">
                          <TriangleAlert />
                          <div>
                            {preview.duplicate.sameContent ? "Такой же файл уже лежит в неделе." : "Файл с таким именем уже есть."} Существующий не будет перезаписан — новый сохранится как <span className="mono">{preview.duplicate.suggestedName}</span>.
                          </div>
                        </div>
                      )}
                    </>
                  ) : focus.status === "ambiguous" ? (
                    <div className="notice notice--warn">
                      <TriangleAlert />
                      <div>Недостаточно данных для пути. Уточните предмет, неделю или тип.</div>
                      <button className="btn btn--sm notice__action" onClick={() => setWizard(true)}>
                        Уточнить
                      </button>
                    </div>
                  ) : (
                    <p className="faint">Определяется…</p>
                  )}
                </div>
              )}
            </div>
          </section>

          <section className="panel tray" aria-label="Только что разложено">
            <div className="panel__head">
              <h2 className="panel__title">Разложено сейчас</h2>
            </div>
            <div className="panel__body">
              {groups.length === 0 ? (
                <p className="faint" style={{ fontSize: 12 }}>
                  Файлы «переедут» сюда после раскладки — с группировкой по неделям.
                </p>
              ) : (
                <ul className="tray__groups">
                  {groups.map(([k, items]) => {
                    const first = items[0]!;
                    return (
                      <motion.li key={k} layout className="tray__group" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
                        <motion.span className="tray__pulse" aria-hidden initial={{ opacity: 0.9, scale: 0.96 }} animate={{ opacity: 0, scale: 1.04 }} transition={{ duration: 0.7, ease: "easeOut" }} />
                        <Link to={weekPath(first.subjectId, first.weekNumber)} className="tray__head">
                          <span>{names(first.subjectId)}</span>
                          <span className="faint num">Неделя {pad2(first.weekNumber)}</span>
                          <ArrowRight size={13} className="faint" />
                        </Link>
                        <div className="tray__chips">
                          {items.map((l) => (
                            <FileChip key={l.file.id} id={l.file.id} name={l.preview.targetName} kind={l.file.kind} sub={l.preview.targetPath.split("/").slice(2, -1).join(" / ")} />
                          ))}
                        </div>
                      </motion.li>
                    );
                  })}
                </ul>
              )}
            </div>
          </section>
        </aside>
      </div>

      <ClassifyWizard open={wizard} files={files} onClose={() => setWizard(false)} onRoute={(ids) => void route(ids)} />
    </div>
  );
}
