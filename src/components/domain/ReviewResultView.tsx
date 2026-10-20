import { motion } from "motion/react";
import { CircleAlert, CircleCheck, FileSearch, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";
import type { CriterionStatus, ReviewFinding, ReviewRun, StyleDivergence, TeacherComment } from "../../lib/bridge";
import { CRITERION_STATUS_LABEL, STYLE_REASON_LABEL } from "../../lib/format";
import { PromptBox } from "./PromptBox";
import { RequirementList } from "./RequirementList";

const DOT: Record<CriterionStatus, string> = { pass: "dot dot--ok", warn: "dot dot--warn", fail: "dot dot--danger", manual: "dot dot--info", na: "dot" };

const VERDICT = {
  pass: { icon: CircleCheck, tone: "ok", title: "Проверка пройдена" },
  needs_fixes: { icon: CircleAlert, tone: "danger", title: "Требуются исправления" },
  needs_teacher_review: { icon: TriangleAlert, tone: "warn", title: "Осталось сверить с преподавателем" },
  blocked: { icon: CircleAlert, tone: "danger", title: "Запуск заблокирован" },
} as const;

export function StyleScore({ style }: { style: StyleDivergence }) {
  const over = style.score > style.threshold;
  return (
    <div className="stylescore" data-over={over}>
      <div className="stylescore__head">
        <span className="stylescore__title">
          Расхождение с личным стилем: <strong className="num">{style.score}/10</strong>
        </span>
        <span className="faint">порог {style.threshold}/10</span>
      </div>
      <div className="stylescore__bar" role="meter" aria-label="Расхождение с личным стилем" aria-valuemin={0} aria-valuemax={10} aria-valuenow={style.score}>
        <motion.span className="stylescore__fill" initial={{ scaleX: 0 }} animate={{ scaleX: style.score / 10 }} transition={{ type: "spring", stiffness: 140, damping: 24 }} />
        <span className="stylescore__tick" style={{ left: `${style.threshold * 10}%` }} />
      </div>
      {style.reasons.length > 0 && (
        <ul className="stylescore__reasons">
          {style.reasons.map((r) => (
            <li key={r.kind + r.text}>
              <span className="stylescore__kind">{STYLE_REASON_LABEL[r.kind]}</span>
              <span className="faint">
                {r.text}
                {r.example ? ` — ${r.example}` : ""}
              </span>
            </li>
          ))}
        </ul>
      )}
      <p className="stylescore__note faint">Сравнение с файлом «Мой стиль» и вашими прошлыми работами. Это не определение авторства текста.</p>
    </div>
  );
}

function Findings({ title, items, tone }: { title: string; items: ReviewFinding[]; tone: "danger" | "warn" }) {
  if (items.length === 0) return null;
  return (
    <section className="findings">
      <h3 className="findings__title eyebrow">
        {title} · {items.length}
      </h3>
      <ul>
        {items.map((f) => (
          <li key={f.id} className="finding" data-tone={tone}>
            <span className="finding__bar" aria-hidden />
            <div>
              <div className="finding__text">{f.text}</div>
              {f.location && <div className="finding__loc faint">{f.location}</div>}
              {f.evidence.length > 0 && (
                <div className="evidence">
                  {f.evidence.map((e) => (
                    <span key={e.sourceId + e.locator} className="evidence__chip" title={e.fileName}>
                      <FileSearch size={11} /> {e.fileName.replace(/\.[^.]+$/, "")} · {e.locator}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

interface Props {
  run: ReviewRun;
  comments?: TeacherComment[];
  promptActions?: ReactNode;
  /** Если передан — промт можно редактировать перед сохранением. */
  onPromptChange?: (text: string) => void;
}

/** Результат запуска: статус, критерии, стиль, ошибки, неподтверждённые утверждения, требования, промт. */
export function ReviewResultView({ run, comments = [], promptActions, onPromptChange }: Props) {
  const r = run.result;
  if (!r) return null;
  const v = VERDICT[r.verdict];
  const Icon = v.icon;
  return (
    <div className="review">
      <div className={`review__verdict notice notice--${v.tone}`}>
        <Icon />
        <div>
          <strong>{run.kind === "analyze_materials" ? "Анализ завершён" : v.title}.</strong> {r.summary}
          <div className="review__action">Рекомендуемое действие: {r.recommendedAction.label}</div>
        </div>
      </div>

      {r.criteria.length > 1 && (
        <div className="criteria">
          {r.criteria.map((c) => (
            <div key={c.id} className="criterion" data-status={c.status}>
              <div className="criterion__head">
                <span className={DOT[c.status]} aria-hidden />
                <span className="criterion__label">{c.label}</span>
              </div>
              <div className="criterion__status">{CRITERION_STATUS_LABEL[c.status]}</div>
              <div className="criterion__note faint">{c.note}</div>
            </div>
          ))}
        </div>
      )}

      {r.style && <StyleScore style={r.style} />}
      <Findings title="Критические ошибки" items={r.critical} tone="danger" />
      <Findings title="Замечания" items={r.remarks} tone="warn" />
      {r.unsupportedClaims.length > 0 && (
        <section className="findings">
          <h3 className="findings__title eyebrow">Неподтверждённые утверждения · {r.unsupportedClaims.length}</h3>
          <ul>
            {r.unsupportedClaims.map((u) => (
              <li key={u.id} className="finding" data-tone="warn">
                <span className="finding__bar" aria-hidden />
                <div>
                  <div className="finding__text">{u.text}</div>
                  <div className="finding__loc faint">{u.location} · опоры в материалах не найдено</div>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {r.teacherRequirements.length > 0 && (
        <section className="findings">
          <h3 className="findings__title eyebrow">Требования преподавателя</h3>
          <RequirementList requirements={r.teacherRequirements} comments={comments} />
        </section>
      )}
      <details className="review__sources">
        <summary>Использованные материалы · {run.sourcesUsed.length}</summary>
        <ul>
          {run.sourcesUsed.map((s) => (
            <li key={s.sourceId}>
              <span className="mono">{s.fileName}</span> <span className="faint">— {s.locator}</span>
            </li>
          ))}
        </ul>
      </details>
      {r.prompt && <PromptBox prompt={r.prompt} actions={promptActions} editable={!!onPromptChange} onChange={onPromptChange} />}
    </div>
  );
}
