import { Eye, Layers, Plus, ShieldCheck, TriangleAlert, X } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type CorpusPreview, type MaterialRef } from "../../lib/bridge";
import { EXTRACTION_FORMAT, formatChars, materialKey, sameMaterial } from "../../lib/extraction";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { Dialog } from "../ui/Dialog";
import { ErrorState, SkeletonRows } from "../ui/States";

interface Props {
  /** Материалы, отмеченные в списке: их можно добавить в подборку. */
  checked: MaterialRef[];
}

/**
 * Подборка материалов — явный выбор пользователя. Только этот набор позже
 * станет источниками (SourceBundle) для анализа; на этом этапе анализ не
 * запускается и Codex/Claude не вызываются.
 */
export function CorpusPanel({ checked }: Props) {
  const { reportError } = useOps();
  const homework = useBridgeQuery(() => bridge.getHomeworkList(), []);
  const [homeworkId, setHomeworkId] = useState<string | null>(null);
  const selection = useBridgeQuery(() => bridge.getCorpusSelection(homeworkId), [homeworkId], ["extraction:updated"]);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<CorpusPreview | null>(null);

  const current = selection.data;
  const submitted = homework.data?.find((h) => h.id === homeworkId)?.stage === "submitted";
  const selected = current?.items.map((i) => i.material) ?? [];
  const addable = checked.filter((m) => !selected.some((s) => sameMaterial(s, m)));

  const save = async (materials: MaterialRef[]) => {
    setBusy(true);
    try {
      const next = await bridge.setCorpusSelection(homeworkId, materials);
      selection.setData(() => next);
    } catch (e) {
      reportError(e, "Подборка не сохранена");
    } finally {
      setBusy(false);
    }
  };

  const openPreview = async () => {
    try {
      setPreview(await bridge.previewCorpus(homeworkId));
    } catch (e) {
      reportError(e, "Предпросмотр не построен");
    }
  };

  return (
    <section className="panel corpus" aria-label="Подборка для анализа">
      <div className="panel__head">
        <h2 className="panel__title">
          <Layers size={15} /> Подборка для анализа
        </h2>
      </div>
      <div className="panel__body corpus__body">
        <label className="field">
          <span className="field__label">Для чего</span>
          <select className="select" value={homeworkId ?? ""} onChange={(e) => setHomeworkId(e.target.value || null)}>
            <option value="">Общая подборка (без ДЗ)</option>
            {homework.data?.map((h) => (
              <option key={h.id} value={h.id}>
                {h.title}
                {h.stage === "submitted" ? " · отправлено" : ""}
              </option>
            ))}
          </select>
        </label>

        <div className="notice corpus__notice">
          <ShieldCheck />
          <div>В анализ позже передаётся только эта подборка. Сейчас ничего не отправляется: Codex и Claude не вызываются.</div>
        </div>

        {selection.error && !current ? (
          <ErrorState message={selection.error} onRetry={() => void selection.reload()} />
        ) : !current ? (
          <SkeletonRows count={2} />
        ) : (
          <>
            {current.items.length === 0 ? (
              <p className="faint corpus__empty">Отметьте материалы с извлечённым текстом и добавьте их сюда.</p>
            ) : (
              <ul className="corpus__items" role="list">
                {current.items.map((item) => (
                  <li key={materialKey(item.material)} className="corpus__item" data-ready={item.ready}>
                    <div className="corpus__name">
                      <span>{item.name}</span>
                      <span className="faint">
                        {item.context}
                        {item.ready ? ` · ${formatChars(item.textChars)}` : ""}
                      </span>
                      {item.problem && (
                        <span className="corpus__problem">
                          <TriangleAlert size={12} /> {item.problem}
                        </span>
                      )}
                    </div>
                    <button
                      className="btn btn--ghost btn--icon btn--sm"
                      aria-label={`Убрать из подборки: ${item.name}`}
                      disabled={busy || submitted}
                      onClick={() => void save(selected.filter((m) => !sameMaterial(m, item.material)))}
                    >
                      <X />
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <div className="corpus__total faint">
              {formatChars(current.totalChars)} из {formatChars(current.limitChars)}
              {current.totalChars > current.limitChars && " — лишнее будет обрезано"}
            </div>
            <div className="corpus__actions">
              <button className="btn btn--sm" disabled={busy || submitted || addable.length === 0} onClick={() => void save([...selected, ...addable])} title={submitted ? "Подборку отправленной работы изменить нельзя" : undefined}>
                <Plus /> Добавить отмеченные{addable.length > 0 && <span className="num"> · {addable.length}</span>}
              </button>
              <button className="btn btn--sm btn--ghost" disabled={current.items.length === 0} onClick={() => void openPreview()}>
                <Eye /> Что получит анализ
              </button>
            </div>
          </>
        )}
      </div>

      <Dialog open={!!preview} onClose={() => setPreview(null)} title="Корпус подборки" description="Ровно этот текст получит будущий анализ. Порядок — как в подборке." extraWide>
        {preview && (
          <div className="corpus-preview">
            <div className="faint">
              {preview.documents.length} документов · {formatChars(preview.totalChars)} из {formatChars(preview.limitChars)}
              {preview.truncated && " · часть текста не поместилась в лимит"}
            </div>
            {preview.excluded.length > 0 && (
              <div className="notice notice--warn">
                <TriangleAlert />
                <div>
                  Не войдут:
                  <ul>
                    {preview.excluded.map((x) => (
                      <li key={materialKey(x.material)}>
                        «{x.name}» — {x.reason}
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            )}
            {preview.documents.map((doc) => (
              <details key={materialKey(doc.material)} className="corpus-preview__doc">
                <summary>
                  <strong>{doc.name}</strong> <span className="faint">· {EXTRACTION_FORMAT[doc.format]} · {formatChars(doc.textChars)}</span>
                  {doc.truncated && <span className="badge badge--warn">обрезан</span>}
                  {doc.warnings.length > 0 && <span className="badge">предупреждений: {doc.warnings.length}</span>}
                </summary>
                <pre className="extract-view__text selectable">{doc.text.slice(0, 20_000)}</pre>
                {doc.text.length > 20_000 && <p className="faint">Показано начало документа.</p>}
              </details>
            ))}
          </div>
        )}
      </Dialog>
    </section>
  );
}
