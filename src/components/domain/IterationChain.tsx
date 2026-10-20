import { AnimatePresence, motion } from "motion/react";
import { ArrowDown, Flag, Lock, MessagesSquare, ScanSearch, Wand2 } from "lucide-react";
import { useState } from "react";
import type { HomeworkIteration, RevisionPrompt } from "../../lib/bridge";
import { authorName, formatDateTime, providerName } from "../../lib/format";
import { PromptBox } from "./PromptBox";

interface Props {
  iterations: HomeworkIteration[];
  finalVersionId: string | null;
  compare: string[];
  onToggleCompare: (versionId: string) => void;
  onMarkFinal: (it: HomeworkIteration) => void;
  locked: boolean;
}

const STATUS: Record<HomeworkIteration["status"], { label: string; tone: string }> = {
  draft: { label: "не проверена", tone: "" },
  reviewing: { label: "проверяется", tone: "badge--info" },
  failed: { label: "есть ошибки", tone: "badge--danger" },
  passed: { label: "проверка пройдена", tone: "badge--ok" },
  final: { label: "итоговая", tone: "badge--accent" },
  submitted: { label: "отправлена", tone: "badge--ok" },
};

function PromptLink({ prompt, label }: { prompt: RevisionPrompt; label: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="itc__prompt">
      <button className="itc__link" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        {label}
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div className="itc__promptbody" initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }} exit={{ opacity: 0, height: 0 }}>
            <PromptBox prompt={prompt} compact />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/**
 * Рабочий цикл: v01 → проверка → промт исправлений → v02 → … Каждая версия — неизменяемый
 * снимок; рядом — промт, по которому она создана, результат проверки, что исправлено и что осталось.
 */
export function IterationChain({ iterations, finalVersionId, compare, onToggleCompare, onMarkFinal, locked }: Props) {
  return (
    <ol className="itc">
      <AnimatePresence initial={false}>
        {iterations.map((it) => {
          const rv = it.review?.result;
          const tr = it.teacherReview?.result;
          const st = STATUS[it.status];
          return (
            <motion.li key={it.versionId} layout className="itc__iter" initial={{ opacity: 0, y: 14 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", stiffness: 260, damping: 30 }}>
              <div className="itc__node itc__node--version" data-final={it.versionId === finalVersionId}>
                <span className="itc__dot" aria-hidden />
                <div className="itc__card">
                  <div className="itc__top">
                    <span className="itc__label mono">{it.label}</span>
                    <span className={`badge ${st.tone}`}>{st.label}</span>
                    <span className="itc__lock faint" title="Версия неизменяема после создания">
                      <Lock size={11} /> снимок
                    </span>
                    <span className="faint itc__meta">
                      {authorName(it.author)} · {formatDateTime(it.createdAt)}
                    </span>
                  </div>
                  <div className="itc__files mono">{it.files.map((f) => f.name).join(" · ")}</div>
                  {it.prompt ? <PromptLink prompt={it.prompt} label={`Создана по: ${it.prompt.kind === "initial" ? "первичному промту" : "промту исправлений"}`} /> : <div className="faint itc__small">Добавлена вручную, без промта</div>}
                  <div className="itc__actions">
                    <label className="vtl__cmp">
                      <input type="checkbox" className="checkbox" checked={compare.includes(it.versionId)} onChange={() => onToggleCompare(it.versionId)} aria-label={`Сравнить ${it.label}`} />
                      Сравнить
                    </label>
                    {it.versionId !== finalVersionId && (
                      <button className="btn btn--sm btn--ghost" onClick={() => onMarkFinal(it)} disabled={locked}>
                        <Flag /> Сделать итоговой
                      </button>
                    )}
                  </div>
                </div>
              </div>

              {it.review && rv && (
                <div className="itc__node itc__node--review" data-verdict={rv.verdict}>
                  <span className="itc__dot" aria-hidden>
                    <ScanSearch size={11} />
                  </span>
                  <div className="itc__body">
                    <div className="itc__title">
                      Проверка {providerName(it.review.provider)} · {formatDateTime(it.review.startedAt)}
                      {!it.review.savedAsIteration && <span className="badge badge--warn">не сохранена в истории</span>}
                    </div>
                    <div className="itc__summary">
                      Критических: <strong>{rv.critical.length}</strong> · замечаний: {rv.remarks.length} · расхождение со стилем: <span className="num">{rv.style?.score ?? "—"}/10</span>
                    </div>
                    {it.fixedFindings.length > 0 && (
                      <ul className="itc__list itc__list--fixed">
                        {it.fixedFindings.map((f) => (
                          <li key={f.id}>Исправлено: {f.text}</li>
                        ))}
                      </ul>
                    )}
                    {it.openFindings.length > 0 && (
                      <ul className="itc__list itc__list--open">
                        {it.openFindings.map((f) => (
                          <li key={f.id} data-severity={f.severity}>
                            Осталось: {f.text}
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                </div>
              )}

              {it.teacherReview && tr && (
                <div className="itc__node itc__node--teacher" data-verdict={tr.verdict}>
                  <span className="itc__dot" aria-hidden>
                    <MessagesSquare size={11} />
                  </span>
                  <div className="itc__body">
                    <div className="itc__title">Сверка с комментариями преподавателя</div>
                    <div className="itc__summary">{tr.summary}</div>
                  </div>
                </div>
              )}

              {it.revisionPrompt && (
                <div className="itc__node itc__node--prompt">
                  <span className="itc__dot" aria-hidden>
                    <Wand2 size={11} />
                  </span>
                  <div className="itc__body">
                    <PromptLink prompt={it.revisionPrompt} label={`${it.revisionPrompt.kind === "teacher" ? "Промт по комментариям" : "Промт исправлений"} ${it.revisionPrompt.sentAt ? "— передан" : "— ещё не передан"}`} />
                  </div>
                </div>
              )}
              {it.nextVersionId && (
                <div className="itc__arrow faint" aria-hidden>
                  <ArrowDown size={13} />
                </div>
              )}
            </motion.li>
          );
        })}
      </AnimatePresence>
    </ol>
  );
}
