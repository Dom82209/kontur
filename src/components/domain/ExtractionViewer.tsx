import { CircleAlert, Info, RefreshCw, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type ExtractedBlock, type ExtractionMetadata, type MaterialExtraction, type MaterialRef } from "../../lib/bridge";
import { EXTRACTION_FORMAT, EXTRACTION_STATUS, formatChars } from "../../lib/extraction";
import { formatBytes, formatDateTime } from "../../lib/format";
import { Dialog } from "../ui/Dialog";
import { SkeletonRows } from "../ui/States";
import { Tabs } from "../ui/Tabs";

type View = "structure" | "text" | "meta";

const META_LABEL: Partial<Record<keyof ExtractionMetadata, string>> = {
  title: "Заголовок",
  author: "Автор",
  subject: "Тема",
  application: "Программа",
  producer: "PDF создан",
  created: "Создан",
  modified: "Изменён",
  pageCount: "Страниц",
  sheetCount: "Листов",
  slideCount: "Слайдов",
  wordCount: "Слов (по данным файла)",
  pdfVersion: "Версия PDF",
  imageFormat: "Формат изображения",
  imageWidth: "Ширина, px",
  imageHeight: "Высота, px",
  encoding: "Кодировка",
  delimiter: "Разделитель",
  rowCount: "Строк",
  columnCount: "Столбцов",
  language: "Язык",
};

/** Текст длиннее этого показывается частями: окно не должно зависать на огромных файлах. */
const TEXT_PAGE = 60_000;

interface Props {
  material: MaterialRef | null;
  name: string;
  onClose: () => void;
}

/**
 * Просмотр извлечённого содержимого: структура, нормализованный текст, metadata
 * и предупреждения о пропущенных частях. Перестроение — повторный разбор
 * оригинала без изменения самого файла.
 */
export function ExtractionViewer({ material, name, onClose }: Props) {
  const { reportError } = useOps();
  const [data, setData] = useState<MaterialExtraction | null>(null);
  const [missing, setMissing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [view, setView] = useState<View>("structure");
  const [shown, setShown] = useState(TEXT_PAGE);

  useEffect(() => {
    setData(null);
    setMissing(false);
    setView("structure");
    setShown(TEXT_PAGE);
    if (!material) return;
    let alive = true;
    bridge
      .getMaterialExtraction(material)
      .then((result) => {
        if (!alive) return;
        if (result) setData(result);
        else setMissing(true);
      })
      .catch((e: unknown) => alive && reportError(e, "Результат извлечения не загружен"));
    return () => {
      alive = false;
    };
  }, [material]);

  const analyze = async (force: boolean) => {
    if (!material) return;
    setBusy(true);
    try {
      const result = await bridge.analyzeMaterial(material, { force });
      setData(result);
      setMissing(false);
    } catch (e) {
      reportError(e, "Текст не извлечён");
    } finally {
      setBusy(false);
    }
  };

  const status = data ? EXTRACTION_STATUS[data.status] : null;

  return (
    <Dialog
      open={!!material}
      onClose={onClose}
      title={name}
      description="Текст и структура извлечены на этом компьютере. Оригинал не изменяется; в анализ попадёт только явно выбранная подборка."
      extraWide
      footer={
        <>
          {data && (
            <span className="faint mono extract-view__foot">
              SHA-256 {data.sha256.slice(0, 12)}… · правила v{data.extractorVersion}
            </span>
          )}
          <button className="btn btn--sm" onClick={() => void analyze(!!data)} disabled={busy || !material}>
            <RefreshCw /> {data ? "Перестроить" : "Извлечь текст"}
          </button>
          <button className="btn btn--sm btn--primary" onClick={onClose}>
            Готово
          </button>
        </>
      }
    >
      {!data && !missing ? (
        <SkeletonRows count={4} />
      ) : missing && !data ? (
        <div className="notice">
          <Info />
          <div>Текст из этого материала ещё не извлекался. Нажмите «Извлечь текст» — файл будет только прочитан.</div>
        </div>
      ) : data ? (
        <div className="extract-view">
          <div className="extract-view__summary">
            <span className={`badge ${status!.tone}`} title={status!.hint}>
              {status!.label}
            </span>
            <span>{EXTRACTION_FORMAT[data.format]}</span>
            <span className="faint">{formatBytes(data.sourceSizeBytes)}</span>
            <span className="faint">{formatChars(data.textChars)}</span>
            {data.cached && <span className="badge" title="Результат взят из cache по SHA-256 без повторного разбора">из cache</span>}
            {data.ocrUsed && <span className="badge badge--info">OCR</span>}
            {data.stale && <span className="badge badge--warn">файл изменился</span>}
            <span className="faint extract-view__when">
              {formatDateTime(data.extractedAt)} · {data.durationMs} мс
            </span>
          </div>

          {data.statusMessage && (
            <div className={`notice${data.status === "complete" || data.status === "partial" ? "" : " notice--warn"}`}>
              {data.status === "corrupt" || data.status === "rejected" || data.status === "failed" ? <CircleAlert /> : <Info />}
              <div>{data.statusMessage}</div>
            </div>
          )}

          {data.warnings.length > 0 && (
            <ul className="extract-view__warnings" role="list" aria-label="Предупреждения">
              {data.warnings.map((w, index) => (
                <li key={`${w.code}-${index}`}>
                  <TriangleAlert size={13} />
                  <span>
                    {w.message}
                    {w.location && <span className="faint"> · {w.location}</span>}
                  </span>
                </li>
              ))}
            </ul>
          )}

          <Tabs
            label="Представление"
            value={view}
            onChange={setView}
            items={[
              { value: "structure", label: "Структура" },
              { value: "text", label: "Текст" },
              { value: "meta", label: "Метаданные" },
            ]}
          />

          {view === "structure" && (
            <div className="extract-view__blocks" role="tabpanel">
              {data.blocks.length === 0 ? <p className="faint">Структуры нет: текст не извлечён.</p> : data.blocks.map((block, index) => <BlockView key={index} block={block} />)}
              {data.blocksTruncated && <p className="faint">Показана первая часть структуры; полный текст — во вкладке «Текст».</p>}
            </div>
          )}

          {view === "text" && (
            <div role="tabpanel">
              {data.text ? (
                <>
                  <pre className="extract-view__text selectable">{data.text.slice(0, shown)}</pre>
                  {data.text.length > shown && (
                    <button className="btn btn--sm btn--ghost" onClick={() => setShown((n) => n + TEXT_PAGE)}>
                      Показать дальше
                    </button>
                  )}
                </>
              ) : (
                <p className="faint">Нормализованного текста нет.</p>
              )}
              {data.truncated && <p className="faint">Текст обрезан лимитом извлечения.</p>}
            </div>
          )}

          {view === "meta" && (
            <dl className="extract-view__meta" role="tabpanel">
              {(Object.keys(META_LABEL) as (keyof ExtractionMetadata)[])
                .filter((k) => data.metadata[k] !== null && data.metadata[k] !== "")
                .map((k) => (
                  <div key={k}>
                    <dt className="faint">{META_LABEL[k]}</dt>
                    <dd>{String(data.metadata[k])}</dd>
                  </div>
                ))}
              <div>
                <dt className="faint">Контекст</dt>
                <dd>{data.context}</dd>
              </div>
            </dl>
          )}
        </div>
      ) : null}
    </Dialog>
  );
}

function BlockView({ block }: { block: ExtractedBlock }) {
  switch (block.type) {
    case "heading":
      return <div className={`xb xb--h xb--h${Math.min(block.level, 4)}`}>{block.text}</div>;
    case "paragraph":
      return <p className="xb xb--p">{block.text}</p>;
    case "list_item":
      return (
        <div className="xb xb--li" style={{ paddingLeft: 12 + block.level * 16 }}>
          <span className="faint">{block.ordered ? "1." : "•"}</span> {block.text}
        </div>
      );
    case "code":
      return <pre className="xb xb--code mono">{block.text}</pre>;
    case "page":
      return (
        <div className="xb xb--page">
          <span className="eyebrow">Страница {block.number}</span>
          <p>{block.text}</p>
        </div>
      );
    case "slide":
      return (
        <div className="xb xb--page">
          <span className="eyebrow">
            Слайд {block.number}
            {block.hidden ? " · скрытый" : ""}
          </span>
          {block.title && <strong>{block.title}</strong>}
          {block.body.map((line, i) => (
            <p key={i}>{line}</p>
          ))}
          {block.notes && <p className="faint">Заметки: {block.notes}</p>}
        </div>
      );
    case "table":
      return (
        <div className="xb xb--table scroll">
          <table>
            <tbody>
              {block.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((cell, c) => (
                    <td key={c}>{cell}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          {block.truncated && <p className="faint">Таблица обрезана для просмотра.</p>}
        </div>
      );
    case "sheet":
      return (
        <div className="xb xb--page">
          <span className="eyebrow">
            Лист «{block.name}»{block.hidden ? " · скрытый" : ""}
          </span>
          <span className="faint">
            Диапазон {block.usedRange ?? "—"}
            {block.declaredRange && block.declaredRange !== block.usedRange ? ` (в файле ${block.declaredRange})` : ""} · ячеек {block.cellCount} · формул {block.formulaCount}
          </span>
          {block.cells.length > 0 && (
            <div className="xb--table scroll">
              <table>
                <tbody>
                  {block.cells.slice(0, 400).map((cell) => (
                    <tr key={cell.ref}>
                      <td className="mono faint">{cell.ref}</td>
                      <td>{cell.value}</td>
                      <td className="mono faint">{cell.formula ?? ""}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {(block.truncated || block.cells.length > 400) && <p className="faint">Показана часть ячеек; все значения — во вкладке «Текст».</p>}
        </div>
      );
  }
}
