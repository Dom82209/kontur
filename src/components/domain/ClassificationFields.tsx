import { Check } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type InboxFile, type MaterialCategory } from "../../lib/bridge";
import { CATEGORY_LABEL } from "../../lib/format";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";

const CATEGORIES: MaterialCategory[] = ["lecture", "seminar", "material", "homework_task", "homework_data", "homework_draft"];
const WEEKS = Array.from({ length: 16 }, (_, i) => i + 1);

interface Props {
  file: InboxFile;
  layout?: "inline" | "stacked";
  onDone?: () => void;
}

/** Поля ручной правки классификации. Каждое изменение сразу уходит в bridge.updateClassification. */
export function ClassificationFields({ file, layout = "inline", onDone }: Props) {
  const { reportError } = useOps();
  const subjects = useBridgeQuery(() => bridge.getSubjects(), []);
  const [saving, setSaving] = useState(false);
  const c = file.classification;

  const patch = async (p: Parameters<typeof bridge.updateClassification>[1]) => {
    setSaving(true);
    try {
      await bridge.updateClassification(file.id, p);
    } catch (e) {
      reportError(e, "Не удалось сохранить");
    } finally {
      setSaving(false);
    }
  };

  // Этап 4: если в неделе несколько ДЗ, файл ДЗ нужно отнести к конкретному.
  const isHomework = c?.category === "homework_task" || c?.category === "homework_data" || c?.category === "homework_draft";
  const weekHomeworks = useBridgeQuery(
    async () => {
      if (!isHomework || !c?.subjectId || !c.weekNumber) return [];
      const week = await bridge.getWeek(c.subjectId, c.weekNumber);
      const list = await Promise.all((week?.homeworkIds ?? []).map((id) => bridge.getHomework(id)));
      return list.filter((h): h is NonNullable<typeof h> => !!h);
    },
    [isHomework, c?.subjectId, c?.weekNumber],
  );

  const stacked = layout === "stacked";
  return (
    <div className={stacked ? "cfields cfields--stacked" : "cfields"} onClick={(e) => e.stopPropagation()}>
      <label className={stacked ? "field" : undefined}>
        {stacked ? <span className="field__label">Предмет</span> : <span className="sr-only">Предмет</span>}
        <select className={`select${stacked ? "" : " select--sm"}`} value={c?.subjectId ?? ""} onChange={(e) => void patch({ subjectId: e.target.value || null })} data-autofocus>
          <option value="">— выберите предмет —</option>
          {subjects.data
            ?.filter((s) => !s.archived || s.id === c?.subjectId)
            .map((s) => (
              <option key={s.id} value={s.id} disabled={s.archived}>
                {s.archived ? `${s.name} (в архиве)` : s.name}
              </option>
          ))}
        </select>
      </label>
      <label className={stacked ? "field" : undefined}>
        {stacked ? <span className="field__label">Неделя</span> : <span className="sr-only">Неделя</span>}
        <select className={`select${stacked ? "" : " select--sm"}`} value={c?.weekNumber ?? ""} onChange={(e) => void patch({ weekNumber: e.target.value ? Number(e.target.value) : null })}>
          <option value="">—</option>
          {WEEKS.map((w) => (
            <option key={w} value={w}>
              Неделя {w}
            </option>
          ))}
        </select>
      </label>
      <label className={stacked ? "field" : undefined}>
        {stacked ? <span className="field__label">Тип материала</span> : <span className="sr-only">Тип</span>}
        <select className={`select${stacked ? "" : " select--sm"}`} value={c?.category === "unknown" ? "" : c?.category ?? ""} onChange={(e) => void patch({ category: (e.target.value || "unknown") as MaterialCategory })}>
          <option value="">—</option>
          {CATEGORIES.map((k) => (
            <option key={k} value={k}>
              {CATEGORY_LABEL[k]}
            </option>
          ))}
        </select>
      </label>
      {(weekHomeworks.data?.length ?? 0) > 1 && (
        <label className={stacked ? "field" : undefined}>
          {stacked ? <span className="field__label">Какое ДЗ недели</span> : <span className="sr-only">ДЗ недели</span>}
          <select className={`select${stacked ? "" : " select--sm"}`} value={c?.homeworkId ?? ""} onChange={(e) => void patch({ homeworkId: e.target.value || null })} aria-label="ДЗ недели">
            <option value="">— выберите ДЗ —</option>
            {weekHomeworks.data!.map((h) => (
              <option key={h.id} value={h.id}>
                {h.ordinal}. {h.title}
              </option>
            ))}
          </select>
        </label>
      )}
      {onDone && (
        <button className="btn btn--sm btn--icon" onClick={onDone} aria-label="Готово" disabled={saving}>
          <Check />
        </button>
      )}
    </div>
  );
}
