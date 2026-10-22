import { motion } from "motion/react";
import { Undo2 } from "lucide-react";
import { useOps } from "../../app/operations";
import { bridge, type OperationKind } from "../../lib/bridge";
import { formatDateTime, formatRelative } from "../../lib/format";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { SkeletonRows } from "../ui/States";

const KIND: Record<OperationKind, string> = {
  classify: "Классификация",
  route: "Раскладка",
  create_week: "Неделя",
  create_version: "Версия",
  mark_final: "Итоговая",
  mark_submitted: "Отправка",
  prepare_sources: "Источники",
  ai_run: "AI",
  revision_prompt: "Промт",
  send_prompt: "Решение",
  save_teacher_comments: "Преподаватель",
  build_package: "Комплект",
  mark_ready: "Готовность",
  confirm_submission: "Отправка",
  update_rules: "Правила",
  import: "Импорт",
  extract: "Извлечение",
  workflow_transition: "Этап работы",
  confirm_style: "Подтверждение стиля",
  undo: "Отмена",
};

export function ActivityLog({ limit = 14 }: { limit?: number }) {
  const { undo } = useOps();
  const q = useBridgeQuery(() => bridge.getActivityLog(), [], ["operation:completed", "operation:failed"]);
  if (!q.data) return <SkeletonRows count={4} />;
  return (
    <ol className="journal">
      {q.data.slice(0, limit).map((o) => (
        <motion.li key={o.id} layout="position" className="journal__item" data-status={o.status} initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
          <span className="journal__kind">{KIND[o.kind]}</span>
          <div className="journal__main">
            <div className="journal__title">
              {o.title}
              {o.status === "undone" && <span className="badge">отменено</span>}
              {o.status === "failed" && <span className="badge badge--danger">ошибка</span>}
              {o.status === "running" && <span className="badge badge--info">выполняется</span>}
            </div>
            {o.detail && <div className="journal__detail">{o.detail}</div>}
            {o.backupPath && <div className="journal__backup mono">резервная копия: {o.backupPath}</div>}
          </div>
          <span className="journal__time num" title={formatDateTime(o.startedAt)}>
            {formatRelative(o.startedAt)}
          </span>
          {o.undoable && o.status === "completed" ? (
            <button className="btn btn--sm btn--ghost" onClick={() => void undo(o.id)} aria-label={`Отменить: ${o.title}`}>
              <Undo2 /> Отменить
            </button>
          ) : (
            <span className="journal__noundo" />
          )}
        </motion.li>
      ))}
    </ol>
  );
}
