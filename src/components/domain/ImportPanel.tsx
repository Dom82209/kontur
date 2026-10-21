import { motion } from "motion/react";
import { CircleAlert, CircleCheck, Copy, FolderSearch, X } from "lucide-react";
import { Link } from "react-router-dom";
import { bridge, type BridgeErrorCode, type ImportItemReport, type InboxImportReport } from "../../lib/bridge";
import { plural } from "../../lib/format";

export interface ImportRun {
  operationId: string;
  total: number;
  progress: number;
  message: string;
  report: InboxImportReport | null;
  error: { code: BridgeErrorCode; message: string } | null;
}

interface Props {
  run: ImportRun;
  onDismiss: () => void;
  onReveal: (fileId: string) => void;
}

const ERROR_HINT: Partial<Record<BridgeErrorCode, string>> = {
  not_found: "Файл исчез до копирования",
  blocked: "Источник небезопасен",
  conflict: "Конфликт с файлом на диске",
  database: "Ошибка базы данных",
  io_error: "Ошибка чтения или записи",
};

/** Прогресс и итог импорта во «Входящие»: что добавлено, что оказалось дублем, что не удалось. */
export function ImportPanel({ run, onDismiss, onReveal }: Props) {
  const done = run.report !== null || run.error !== null;
  const settingsHint = run.error?.code === "blocked" && run.error.message.includes("Хранилище");

  return (
    <motion.section
      className="panel importrun"
      aria-label="Импорт во «Входящие»"
      aria-live="polite"
      data-state={run.error ? "error" : done ? "done" : "running"}
      initial={{ opacity: 0, y: -4 }}
      animate={{ opacity: 1, y: 0 }}
    >
      <div className="importrun__head">
        <div className="importrun__title">
          {!done && `Импорт · ${Math.round(run.progress * run.total)} из ${run.total}`}
          {run.report && summary(run.report)}
          {run.error && "Импорт не выполнен"}
        </div>
        {done && (
          <button className="btn btn--sm btn--ghost btn--icon" onClick={onDismiss} aria-label="Скрыть итог импорта">
            <X />
          </button>
        )}
      </div>

      {!done && (
        <>
          <div className="meter" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(run.progress * 100)}>
            <motion.div className="meter__fill" initial={false} animate={{ scaleX: Math.max(0.02, run.progress) }} transition={{ duration: 0.2 }} />
          </div>
          <div className="importrun__message faint mono">{run.message}</div>
        </>
      )}

      {run.error && (
        <div className="notice notice--warn" role="alert">
          <CircleAlert />
          <div>
            {run.error.message}
            {settingsHint && (
              <>
                {" "}
                <Link to="/settings">Открыть настройки</Link>
              </>
            )}
          </div>
        </div>
      )}

      {run.report && (
        <ul className="importrun__items" role="list">
          {run.report.items.map((item, index) => (
            <li key={`${item.name}-${index}`} className="importrun__item" data-status={item.status}>
              <StatusIcon item={item} />
              <span className="importrun__name">{item.status === "failed" ? item.name : item.file.name}</span>
              <span className="importrun__note">{note(item)}</span>
              {item.status !== "failed" && bridge.kind === "tauri" && (
                <button className="btn btn--sm btn--ghost" onClick={() => onReveal(item.file.id)}>
                  <FolderSearch /> В Finder
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </motion.section>
  );
}

function StatusIcon({ item }: { item: ImportItemReport }) {
  if (item.status === "imported") return <CircleCheck className="importrun__icon importrun__icon--ok" aria-label="Добавлен" />;
  if (item.status === "duplicate") return <Copy className="importrun__icon importrun__icon--dup" aria-label="Дубль" />;
  return <CircleAlert className="importrun__icon importrun__icon--fail" aria-label="Ошибка" />;
}

function note(item: ImportItemReport): string {
  switch (item.status) {
    case "imported":
      if (item.renamedFrom) return `Имя «${item.renamedFrom}» занято — сохранён под новым, ничего не перезаписано`;
      if (item.recovered) return "Файл уже был во «Входящих» без записи — запись восстановлена";
      return "Добавлен";
    case "duplicate":
      return item.restored ? "Такой файл уже есть во «Входящих»; копия на диске восстановлена" : "Такой файл уже есть во «Входящих» — второй копии нет";
    case "failed":
      return `${ERROR_HINT[item.error.code] ?? "Ошибка"}: ${item.error.message}`;
  }
}

function summary(report: InboxImportReport): string {
  const parts = [
    `добавлено ${report.imported}`,
    report.duplicates > 0 ? `${report.duplicates} ${plural(report.duplicates, "дубль", "дубля", "дублей")}` : null,
    report.failed > 0 ? `${report.failed} ${plural(report.failed, "ошибка", "ошибки", "ошибок")}` : null,
    report.skipped > 0 ? `пропущено ${report.skipped}` : null,
  ].filter(Boolean);
  return `Импорт завершён: ${parts.join(" · ")}${report.replayed ? " (повтор)" : ""}`;
}
