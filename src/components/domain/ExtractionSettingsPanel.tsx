import { Trash2 } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge } from "../../lib/bridge";
import { formatChars } from "../../lib/extraction";
import { formatBytes } from "../../lib/format";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { ErrorState, SkeletonRows } from "../ui/States";
import { Switch } from "../ui/Switch";

/**
 * Настройки извлечения текста: необязательный локальный OCR (выключен по
 * умолчанию), действующие лимиты и очистка cache результатов.
 */
export function ExtractionSettingsPanel() {
  const { reportError } = useOps();
  const toast = useToast();
  const q = useBridgeQuery(() => bridge.getExtractionSettings(), []);
  const [busy, setBusy] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  const toggleOcr = async (ocrEnabled: boolean) => {
    setBusy(true);
    try {
      const next = await bridge.setExtractionSettings({ ocrEnabled });
      q.setData(() => next);
    } catch (e) {
      reportError(e, "Настройка OCR не сохранена");
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      const { removed } = await bridge.clearExtractionCache();
      toast.push({ tone: "ok", title: "Cache извлечения очищен", text: `Удалено результатов: ${removed}. Оригиналы не затронуты — текст можно извлечь заново.` });
    } catch (e) {
      reportError(e, "Cache не очищен");
    } finally {
      setBusy(false);
      setConfirmClear(false);
    }
  };

  if (q.error && !q.data) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  const s = q.data;
  if (!s) return <SkeletonRows count={3} />;
  return (
    <div className="settings__fields">
      <div className="settings__line">
        <div>
          <div className="toggles__title">
            Распознавать текст на изображениях (OCR){" "}
            <span className={`badge ${s.ocr.available ? "badge--ok" : ""}`}>{s.ocr.available ? `${s.ocr.engine} · ${s.ocr.languages}` : "не найден"}</span>
          </div>
          <div className="toggles__hint">Только локально, по умолчанию выключено. Без OCR изображения и сканы сохраняются с метаданными и статусом «Нужен OCR» — импорт это не блокирует.</div>
          <div className="field__hint">{s.ocr.message}</div>
        </div>
        <Switch label="Распознавать текст на изображениях" checked={s.ocrEnabled} onChange={(v) => void toggleOcr(v)} disabled={busy} />
      </div>
      <ul className="extract-limits faint" aria-label="Лимиты извлечения">
        <li>Файл до {formatBytes(s.limits.maxInputBytes)}, до {s.limits.timeLimitSeconds} с на файл</li>
        <li>
          Текст до {formatChars(s.limits.maxTextChars)} на файл, корпус подборки до {formatChars(s.limits.maxCorpusChars)}
        </li>
        <li>
          PDF до {s.limits.maxPdfPages} стр., книги до {s.limits.maxSheets} листов и {s.limits.maxCellsPerSheet.toLocaleString("ru-RU")} ячеек на лист, презентации до {s.limits.maxSlides} слайдов
        </li>
        <li>Макросы, формулы и встроенные объекты не выполняются; DTD и внешние ссылки в Office-файлах отклоняются</li>
      </ul>
      {confirmClear ? (
        <div className="notice notice--warn">
          <Trash2 />
          <div>Удалить все сохранённые результаты извлечения? Подборки останутся, но их материалы нужно будет проанализировать заново.</div>
          <button className="btn btn--sm btn--danger notice__action" onClick={() => void clear()} disabled={busy}>
            Очистить
          </button>
          <button className="btn btn--sm btn--ghost" onClick={() => setConfirmClear(false)} disabled={busy}>
            Отмена
          </button>
        </div>
      ) : (
        <button className="btn btn--sm btn--ghost" style={{ justifySelf: "start" }} onClick={() => setConfirmClear(true)} disabled={busy}>
          <Trash2 /> Очистить cache извлечения
        </button>
      )}
    </div>
  );
}
