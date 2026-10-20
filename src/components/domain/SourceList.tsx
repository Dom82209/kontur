import { motion } from "motion/react";
import { CircleAlert, Info } from "lucide-react";
import type { SourceBundle, SourceKind } from "../../lib/bridge";
import { formatBytes, SOURCE_KIND_LABEL } from "../../lib/format";
import { FileIcon } from "../ui/FileIcon";

const ORDER: SourceKind[] = ["task", "data", "lecture", "seminar", "extra", "style", "previous_work", "current_version", "previous_version", "teacher_comments"];

interface Props {
  bundle: SourceBundle;
  onToggle?: (itemId: string, included: boolean) => void;
  busy?: boolean;
}

/** Набор источников, передаваемых в запуск. Любой файл можно исключить; недостающие обязательные — отдельно. */
export function SourceList({ bundle, onToggle, busy }: Props) {
  const groups = ORDER.map((k) => ({ kind: k, items: bundle.items.filter((i) => i.kind === k) })).filter((g) => g.items.length > 0);
  const required = bundle.missing.filter((m) => m.required);
  const optional = bundle.missing.filter((m) => !m.required);
  return (
    <div className="sources">
      {required.length > 0 && (
        <div className="notice notice--danger" role="alert">
          <CircleAlert />
          <div>
            <strong>Не хватает обязательных материалов:</strong>
            <ul className="sources__missing">
              {required.map((m) => (
                <li key={m.label}>
                  {m.label} — <span className="faint">{m.hint}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}
      {optional.length > 0 && (
        <div className="notice notice--warn">
          <Info />
          <div>
            {optional.map((m) => (
              <div key={m.label}>
                {m.label}: <span className="faint">{m.hint}</span>
              </div>
            ))}
          </div>
        </div>
      )}
      {groups.map((g) => (
        <div key={g.kind} className="sources__group">
          <div className="sources__head eyebrow">
            {SOURCE_KIND_LABEL[g.kind]}
            {g.items.some((i) => i.required) && <span className="sources__req">обязательно</span>}
          </div>
          <ul>
            {g.items.map((i) => (
              <motion.li key={i.id} layout="position" className="sources__item" data-included={i.included}>
                <label className="row sources__row">
                  {onToggle && (
                    <input type="checkbox" className="checkbox" checked={i.included} disabled={busy} onChange={(e) => onToggle(i.id, e.target.checked)} aria-label={`${i.included ? "Исключить" : "Включить"}: ${i.file.name}`} />
                  )}
                  <FileIcon kind={i.file.kind} />
                  <span className="row__main">
                    <span className="row__title" style={{ display: "block" }}>
                      {i.file.name}
                    </span>
                    <span className="row__sub" style={{ display: "block" }}>
                      {formatBytes(i.file.sizeBytes)}
                      {!i.included && " · исключён"}
                    </span>
                  </span>
                </label>
              </motion.li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}
