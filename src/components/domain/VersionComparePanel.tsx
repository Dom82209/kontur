import { motion } from "motion/react";
import { bridge } from "../../lib/bridge";
import { CRITERION_STATUS_LABEL } from "../../lib/format";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { SkeletonRows } from "../ui/States";

const SEC = { same: "без изменений", changed: "изменён", added: "добавлен", removed: "удалён" } as const;

function Delta({ label, from, to, better }: { label: string; from: string; to: string; better: boolean | null }) {
  return (
    <div className="delta" data-better={better === null ? "na" : String(better)}>
      <span className="delta__label">{label}</span>
      <span className="delta__vals num">
        {from} → <strong>{to}</strong>
      </span>
    </div>
  );
}

/** Сравнение двух версий: исправленные и новые проблемы, разделы, стиль, опора на материалы, требования. */
export function VersionComparePanel({ fromId, toId }: { fromId: string; toId: string }) {
  const q = useBridgeQuery(() => bridge.compareVersions(fromId, toId), [fromId, toId]);
  const c = q.data;
  if (!c) return <SkeletonRows count={3} />;
  const t = (x: { done: number; total: number } | null) => (x ? `${x.done}/${x.total}` : "—");
  return (
    <motion.div className="vcmp" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}>
      <div className="vcmp__deltas">
        <Delta label="Расхождение со стилем" from={c.style.from === null ? "—" : `${c.style.from}/10`} to={c.style.to === null ? "—" : `${c.style.to}/10`} better={c.style.from === null || c.style.to === null ? null : c.style.to <= c.style.from} />
        <Delta label="Опора на материалы" from={c.grounding.from ? CRITERION_STATUS_LABEL[c.grounding.from] : "—"} to={c.grounding.to ? CRITERION_STATUS_LABEL[c.grounding.to] : "—"} better={c.grounding.unsupportedTo <= c.grounding.unsupportedFrom} />
        <Delta label="Требования преподавателя" from={t(c.teacher.from)} to={t(c.teacher.to)} better={c.teacher.to && c.teacher.from ? c.teacher.to.done >= c.teacher.from.done : null} />
      </div>
      <div className="vcmp__cols">
        <div>
          <h4 className="eyebrow">Исправлено · {c.fixed.length}</h4>
          <ul className="itc__list itc__list--fixed">{c.fixed.length ? c.fixed.map((f) => <li key={f.id}>{f.text}</li>) : <li className="faint">—</li>}</ul>
        </div>
        <div>
          <h4 className="eyebrow">Появилось · {c.introduced.length}</h4>
          <ul className="itc__list itc__list--open">{c.introduced.length ? c.introduced.map((f) => <li key={f.id}>{f.text}</li>) : <li className="faint">—</li>}</ul>
        </div>
      </div>
      <ul className="cmp__rows">
        {c.sections.map((s) => (
          <li key={s.title} className="cmp__row" data-diff={s.diff}>
            <span className="cmp__mark" aria-hidden />
            <span className="cmp__title">{s.title}</span>
            <span className="cmp__state">{SEC[s.diff]}</span>
          </li>
        ))}
      </ul>
    </motion.div>
  );
}
