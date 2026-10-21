import { Archive, Eye, EyeOff, FileText, FolderSearch, Radar, X } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router-dom";
import { useOps } from "../../app/operations";
import { bridge, type DownloadCandidate, type DownloadsWatcher, type DownloadsWatcherState } from "../../lib/bridge";
import { formatBytes, formatRelative } from "../../lib/format";
import { useDownloadsWatcher } from "../../lib/hooks/useDownloadsWatcher";
import { Switch } from "../ui/Switch";
import { ErrorState, SkeletonRows } from "../ui/States";

export const WATCHER_STATE: Record<DownloadsWatcherState, { label: string; tone: string }> = {
  disabled: { label: "Выключено", tone: "" },
  no_folder: { label: "Нет папки", tone: "badge--warn" },
  watching: { label: "Наблюдает", tone: "badge--ok" },
  error: { label: "Ошибка", tone: "badge--danger" },
};

/**
 * Переключатель наблюдения за загрузками (Настройки). Выключен по умолчанию;
 * включение — только явным действием пользователя. Watcher ничего не импортирует
 * сам: он показывает кандидатов, импорт начинается после предпросмотра.
 */
export function DownloadsWatcherToggle() {
  const { reportError } = useOps();
  const q = useDownloadsWatcher();
  const [busy, setBusy] = useState(false);

  const toggle = async (enabled: boolean) => {
    setBusy(true);
    try {
      const next = await bridge.setDownloadsWatcher(enabled);
      q.setData(() => next);
    } catch (e) {
      reportError(e, enabled ? "Наблюдение не включено" : "Наблюдение не выключено");
      await q.reload();
    } finally {
      setBusy(false);
    }
  };

  if (q.error && !q.data) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  const w = q.data;
  return (
    <div className="watcher">
      <div className="settings__line">
        <div>
          <div className="toggles__title">
            Следить за папкой загрузок {w && <span className={`badge ${WATCHER_STATE[w.state].tone}`}>{WATCHER_STATE[w.state].label}</span>}
          </div>
          <div className="toggles__hint">
            Завершённые загрузки (ZIP/PDF экспорта CU LMS Enhancer и учебные файлы) появятся как кандидаты во «Входящих». Импорт — только после предпросмотра и подтверждения. Временные файлы браузера (.crdownload, .part) игнорируются.
          </div>
          {w?.message && <div className="field__hint watcher__message">{w.message}</div>}
        </div>
        <Switch label="Следить за папкой загрузок" checked={!!w?.enabled} onChange={(v) => void toggle(v)} disabled={busy || !w} />
      </div>
    </div>
  );
}

/** Статус watcher и список кандидатов (Автоматизация и «Входящие»). */
export function DownloadCandidates({ compact = false, onPreview }: { compact?: boolean; onPreview?: (candidate: DownloadCandidate) => void }) {
  const { reportError } = useOps();
  const q = useDownloadsWatcher();
  if (q.error && !q.data) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  if (!q.data) return <SkeletonRows count={2} />;
  const w: DownloadsWatcher = q.data;
  const dismiss = (id: string) =>
    void bridge
      .dismissDownloadCandidate(id)
      .then((next) => q.setData(() => next))
      .catch((e: unknown) => reportError(e, "Не удалось скрыть файл"));

  return (
    <div className="candidates" data-compact={compact}>
      {!compact && (
        <div className="candidates__status">
          <Radar size={14} />
          <span className={`badge ${WATCHER_STATE[w.state].tone}`}>{WATCHER_STATE[w.state].label}</span>
          <span className="faint">{w.enabled ? (w.lastScanAt ? `проверено ${formatRelative(w.lastScanAt)}` : "запускается…") : "Включается в «Настройки → Управляемые каталоги»."}</span>
          {w.message && <span className="candidates__error">{w.message}</span>}
        </div>
      )}
      {w.candidates.length === 0 ? (
        !compact && <p className="faint candidates__empty">Новых загрузок нет. Файлы, уже лежавшие в папке при включении, не предлагаются — для них есть «Импортировать экспорт ЦУ».</p>
      ) : (
        <ul className="candidates__list" role="list">
          {w.candidates.map((c) => (
            <li key={c.id} className="candidates__item">
              {c.kind === "zip" ? <Archive size={15} className="candidates__icon" /> : <FileText size={15} className="candidates__icon" />}
              <div className="candidates__main">
                <div className="candidates__name">{c.name}</div>
                <div className="faint candidates__meta">
                  {formatBytes(c.sizeBytes)} · {c.origin === "watcher" ? "найден в загрузках" : "выбран вручную"} · {formatRelative(c.detectedAt)}
                </div>
              </div>
              {onPreview ? (
                <button className="btn btn--sm" onClick={() => onPreview(c)}>
                  <Eye /> Предпросмотр
                </button>
              ) : (
                <Link className="btn btn--sm" to={`/inbox?candidate=${encodeURIComponent(c.id)}`}>
                  <FolderSearch /> Во «Входящие»
                </Link>
              )}
              <button className="btn btn--sm btn--ghost btn--icon" aria-label={`Скрыть: ${c.name}`} title="Не предлагать этот файл" onClick={() => dismiss(c.id)}>
                {compact ? <X /> : <EyeOff />}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
