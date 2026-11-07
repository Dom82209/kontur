import { Check, ListChecks, TriangleAlert, X } from "lucide-react";
import { useMemo, useState } from "react";
import { useOps } from "../../app/operations";
import {
  bridge,
  type ContentChangeCategory,
  type ContentChangeKind,
  type ContentComparisonStatus,
  type CorrectionStatus,
  type Homework,
  type HomeworkVersion,
  type VersionDecision,
  type VersionOrigin,
  type WorkflowCommandOptions,
} from "../../lib/bridge";
import { formatDateTime } from "../../lib/format";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { ErrorState, SkeletonRows } from "../ui/States";

export const ORIGIN_LABEL: Record<VersionOrigin, string> = {
  user_import: "добавлена пользователем",
  claude_result: "результат Claude",
  codex_result: "результат Codex",
  manual_edit: "ручная правка",
  restored: "восстановлена",
  other: "другое",
};

export const DECISION_LABEL: Record<VersionDecision, string> = {
  pending: "без решения",
  accepted: "принята",
  rejected: "отклонена",
  superseded: "заменена",
  final: "итоговая",
};

const DECISION_TONE: Record<VersionDecision, string> = {
  pending: "",
  accepted: "badge--ok",
  rejected: "badge--danger",
  superseded: "badge--warn",
  final: "badge--ok",
};

const CATEGORY_LABEL: Record<ContentChangeCategory, string> = {
  file: "Файлы",
  text: "Текст",
  heading: "Заголовки",
  list: "Списки",
  structure: "Структура",
  page: "Страницы PDF",
  sheet: "Листы",
  cell: "Ячейки",
  formula: "Формулы",
  binary: "Двоичные",
};

const KIND_LABEL: Record<ContentChangeKind, string> = { added: "добавлено", removed: "удалено", modified: "изменено", renamed: "переименовано" };
const STATUS_LABEL: Record<ContentComparisonStatus, string> = { identical: "совпадают", complete: "сравнено", partial: "сравнено частично", manual_required: "требуется ручная проверка" };
const ITEM_STATUS_LABEL: Record<CorrectionStatus, string> = { proposed: "предложено", open: "к исправлению", resolved: "исправлено", dismissed: "не требуется" };
const PRIORITY_LABEL = { high: "высокий", medium: "средний", low: "низкий" } as const;

/**
 * Этап 10: родословная версий — родитель, происхождение, причина и решение
 * пользователя. Версии не меняются: принятие и отклонение — записи журнала.
 */
export function VersionLineagePanel({ homework, locked, options, onChanged }: { homework: Homework; locked: boolean; options: () => WorkflowCommandOptions; onChanged: () => void }) {
  const { reportError } = useOps();
  const [rejecting, setRejecting] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const label = (id: string | null) => homework.versions.find((v) => v.id === id)?.label ?? "—";
  const act = async (fn: () => Promise<unknown>, failure: string) => {
    setBusy(true);
    try {
      await fn();
      setRejecting(null);
      setReason("");
      onChanged();
    } catch (error) {
      reportError(error, failure);
    } finally {
      setBusy(false);
    }
  };
  const versions = [...homework.versions].sort((a, b) => b.number - a.number);
  if (versions.length === 0) return null;
  return (
    <ol className="lineage" data-testid="version-lineage">
      {versions.map((v) => {
        const l = v.lineage;
        const current = homework.finalVersionId === v.id;
        return (
          <li key={v.id} className="lineage__item" data-decision={l.decision}>
            <div className="lineage__head">
              <strong className="mono">{v.label}</strong>
              <span className={`badge ${DECISION_TONE[l.decision]}`}>{DECISION_LABEL[l.decision]}</span>
              {current && <span className="badge">текущая</span>}
              <span className="faint">
                {ORIGIN_LABEL[l.origin]} · от {l.parentVersionId ? label(l.parentVersionId) : "— (родитель неизвестен или первая версия)"} · {formatDateTime(v.createdAt)}
              </span>
            </div>
            {l.creationReason && <p className="lineage__reason">{l.creationReason}</p>}
            {l.decision === "rejected" && l.decisionReason && <p className="lineage__rejected">Причина отклонения: {l.decisionReason}</p>}
            {l.decision === "superseded" && l.supersededByVersionId && <p className="faint">Заменена версией {label(l.supersededByVersionId)}</p>}
            {!locked && (
              <div className="button-row">
                {l.decision !== "accepted" && l.decision !== "final" && (
                  <button className="btn btn--sm" disabled={busy} onClick={() => void act(() => bridge.acceptVersion(v.id, "", options()), "Версия не принята")}>
                    <Check size={13} /> Принять
                  </button>
                )}
                {l.decision !== "rejected" && !current && (
                  <button className="btn btn--sm btn--ghost" disabled={busy} onClick={() => setRejecting(rejecting === v.id ? null : v.id)}>
                    <X size={13} /> Отклонить
                  </button>
                )}
              </div>
            )}
            {rejecting === v.id && (
              <div className="button-row">
                <input className="input" placeholder="Почему версия не подходит" value={reason} onChange={(e) => setReason(e.target.value)} aria-label={`Причина отклонения ${v.label}`} />
                <button className="btn btn--sm btn--danger" disabled={busy || !reason.trim()} onClick={() => void act(() => bridge.rejectVersion(v.id, reason.trim(), options()), "Версия не отклонена")}>
                  Отклонить {v.label}
                </button>
              </div>
            )}
          </li>
        );
      })}
    </ol>
  );
}

/** Этап 10: структурное сравнение содержимого двух версий (текст, PDF, Excel, файлы). */
export function ContentComparePanel({ homework, ids }: { homework: Homework; ids: [string, string] }) {
  const [base, target] = useMemo(() => {
    const [a, b] = ids.map((id) => homework.versions.find((v) => v.id === id)) as [HomeworkVersion | undefined, HomeworkVersion | undefined];
    return (a?.number ?? 0) <= (b?.number ?? 0) ? [ids[0], ids[1]] : [ids[1], ids[0]];
  }, [homework.versions, ids]);
  const q = useBridgeQuery(() => bridge.compareVersionContents(homework.id, base, target), [homework.id, base, target]);
  const [filter, setFilter] = useState<ContentChangeCategory | "all">("all");
  if (q.error && !q.data) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  const c = q.data;
  if (!c) return <SkeletonRows count={3} />;
  const categories = [...new Set(c.changes.map((x) => x.category))];
  const shown = c.changes.filter((x) => filter === "all" || x.category === filter);
  return (
    <div className="ccmp" data-testid="content-compare">
      <div className="ccmp__head">
        <span className={`badge ${c.status === "manual_required" ? "badge--warn" : c.status === "partial" ? "badge--warn" : "badge--ok"}`}>{STATUS_LABEL[c.status]}</span>
        <span>{c.summary}</span>
        {categories.length > 1 && (
          <select className="select select--sm" value={filter} onChange={(e) => setFilter(e.target.value as ContentChangeCategory | "all")} aria-label="Тип изменений">
            <option value="all">Все изменения</option>
            {categories.map((cat) => (
              <option key={cat} value={cat}>
                {CATEGORY_LABEL[cat]}
              </option>
            ))}
          </select>
        )}
      </div>
      {c.stale && (
        <div className="notice notice--warn">
          <TriangleAlert /> Файлы версий изменились после сравнения — сравните снова.
        </div>
      )}
      {c.warnings.map((w) => (
        <div key={w} className="notice notice--warn">
          <TriangleAlert /> {w}
        </div>
      ))}
      {c.manualChecks.length > 0 && (
        <div data-testid="content-compare-manual">
          <div className="eyebrow">Требуется ручная проверка</div>
          <ul className="pkg__files">
            {c.manualChecks.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
        </div>
      )}
      {shown.length > 0 && (
        <ul className="ccmp__list">
          {shown.map((x, i) => (
            <li key={i} className="ccmp__row" data-kind={x.kind}>
              <span className="ccmp__kind">{KIND_LABEL[x.kind]}</span>
              <span className="faint">
                {CATEGORY_LABEL[x.category]} · {x.file}
                {x.location ? ` · ${x.location}` : ""}
              </span>
              {(x.before || x.after) && (
                <span className="ccmp__vals">
                  {x.before && <del>{x.before}</del>}
                  {x.before && x.after && " → "}
                  {x.after && <ins>{x.after}</ins>}
                </span>
              )}
              {x.detail && <span className="faint">{x.detail}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * Этап 10: план исправлений из текущей проверки и действующих требований.
 * Пункты из проверки AI — предложения, пока пользователь их не подтвердит.
 */
export function CorrectionPlanPanel({ homework, locked }: { homework: Homework; locked: boolean }) {
  const { reportError } = useOps();
  const q = useBridgeQuery(() => bridge.getCorrectionPlan(homework.id), [homework.id, homework.revision]);
  const [busy, setBusy] = useState(false);
  const [resolveIn, setResolveIn] = useState<Record<string, string>>({});
  const run = async (fn: () => Promise<unknown>, failure: string) => {
    setBusy(true);
    try {
      await fn();
      await q.reload();
    } catch (error) {
      reportError(error, failure);
    } finally {
      setBusy(false);
    }
  };
  if (q.error && q.data === null && !q.loading) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  if (q.loading && q.data === null) return <SkeletonRows count={2} />;
  const plan = q.data;
  const planVersion = homework.versions.find((v) => v.id === plan?.versionId);
  const later = homework.versions.filter((v) => planVersion && v.number > planVersion.number);
  return (
    <div className="cplan" data-testid="correction-plan">
      <p className="field__hint">Строится только из текущей проверки итоговой версии и действующих требований преподавателя. Замечания проверки — предложения: подтвердите их, прежде чем исправлять.</p>
      {!locked && (
        <div className="button-row">
          <button className="btn btn--sm" disabled={busy} onClick={() => void run(() => bridge.buildCorrectionPlan(homework.id), "План не построен")}>
            <ListChecks /> {plan ? "Перестроить план" : "Построить план"}
          </button>
        </div>
      )}
      {!plan ? (
        <p className="faint">Плана исправлений пока нет.</p>
      ) : (
        <>
          <p className="faint">
            Для версии {planVersion?.label ?? "—"} · {formatDateTime(plan.createdAt)}
          </p>
          {plan.stale && (
            <div className="notice notice--warn">
              <TriangleAlert /> После построения изменились проверка, требования или итоговая версия — перестройте план.
            </div>
          )}
          <ol className="cplan__items">
            {plan.items.map((item) => (
              <li key={item.id} className="cplan__item" data-status={item.status}>
                <div className="cplan__head">
                  <span className={`badge ${item.priority === "high" ? "badge--danger" : item.priority === "medium" ? "badge--warn" : ""}`}>{PRIORITY_LABEL[item.priority]}</span>
                  <span className="badge">{ITEM_STATUS_LABEL[item.status]}</span>
                  <span className="faint">{item.category}</span>
                </div>
                <p>{item.issue}</p>
                <dl className="cplan__meta faint">
                  <dt>Источник</dt>
                  <dd>{item.sourceRequirement || "—"}</dd>
                  {item.location && (
                    <>
                      <dt>Где</dt>
                      <dd>
                        {item.file ? `${item.file} · ` : ""}
                        {item.location}
                      </dd>
                    </>
                  )}
                  {item.evidence && (
                    <>
                      <dt>Основание</dt>
                      <dd>{item.evidence}</dd>
                    </>
                  )}
                  <dt>Что изменить</dt>
                  <dd>{item.expectedChange}</dd>
                  <dt>Критерий приёмки</dt>
                  <dd>{item.acceptanceCriterion}</dd>
                  {item.resolvedInVersionId && (
                    <>
                      <dt>Исправлено в</dt>
                      <dd>{homework.versions.find((v) => v.id === item.resolvedInVersionId)?.label ?? "—"}</dd>
                    </>
                  )}
                </dl>
                {!locked && item.status !== "resolved" && (
                  <div className="button-row">
                    {item.status === "proposed" && (
                      <button className="btn btn--sm" disabled={busy} onClick={() => void run(() => bridge.setCorrectionItemStatus(item.id, "open"), "Пункт не подтверждён")}>
                        <Check size={13} /> Подтвердить
                      </button>
                    )}
                    {item.status !== "dismissed" && (
                      <button className="btn btn--sm btn--ghost" disabled={busy} onClick={() => void run(() => bridge.setCorrectionItemStatus(item.id, "dismissed"), "Пункт не изменён")}>
                        <X size={13} /> Не требуется
                      </button>
                    )}
                    {item.status === "open" && later.length > 0 && (
                      <>
                        <select className="select select--sm" value={resolveIn[item.id] ?? ""} onChange={(e) => setResolveIn((m) => ({ ...m, [item.id]: e.target.value }))} aria-label="Версия с исправлением">
                          <option value="">Версия с исправлением…</option>
                          {later.map((v) => (
                            <option key={v.id} value={v.id}>
                              {v.label}
                            </option>
                          ))}
                        </select>
                        <button className="btn btn--sm" disabled={busy || !resolveIn[item.id]} onClick={() => void run(() => bridge.setCorrectionItemStatus(item.id, "resolved", resolveIn[item.id]), "Исправление не связано с версией")}>
                          Исправлено
                        </button>
                      </>
                    )}
                  </div>
                )}
              </li>
            ))}
          </ol>
        </>
      )}
    </div>
  );
}
