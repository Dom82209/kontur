import { ClipboardCheck } from "lucide-react";
import { useEffect, useState } from "react";
import type { CriterionId, CriterionStatus, ManualAnalysisInput, ManualReviewInput, ReviewRun, ReviewVerdict } from "../../lib/bridge";
import { CRITERION_STATUS_LABEL, RUN_KIND_LABEL } from "../../lib/format";
import { Dialog } from "../ui/Dialog";

const VERDICTS: { value: ReviewVerdict; label: string }[] = [
  { value: "needs_fixes", label: "Нужны исправления" },
  { value: "pass", label: "Пройдено" },
  { value: "needs_teacher_review", label: "Пройдено, нужна сверка с преподавателем" },
  { value: "blocked", label: "Проверка невозможна" },
];

const STATUSES = Object.keys(CRITERION_STATUS_LABEL) as CriterionStatus[];

const lines = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

/** «Текст — место» → утверждение и его место в работе. */
const claim = (line: string) => {
  const [text, ...rest] = line.split(" — ");
  return { text: text!.trim(), location: rest.join(" — ").trim() || "место не указано" };
};

interface Props {
  run: ReviewRun | null;
  busy: boolean;
  onClose: () => void;
  onAnalysis: (input: ManualAnalysisInput) => void;
  onReview: (input: ManualReviewInput) => void;
}

/**
 * Ручной результат шага, для которого AI-адаптера нет (этап 8). Результат
 * сохраняется с отметкой «вручную» — приложение не выдаёт его за ответ AI.
 */
export function ManualResultDialog({ run, busy, onClose, onAnalysis, onReview }: Props) {
  const [summary, setSummary] = useState("");
  const [promptText, setPromptText] = useState("");
  const [verdict, setVerdict] = useState<ReviewVerdict>("needs_fixes");
  const [criteria, setCriteria] = useState<Record<"correctness" | "taskFit" | "courseGrounding" | "teacher", CriterionStatus>>({
    correctness: "pass",
    taskFit: "pass",
    courseGrounding: "pass",
    teacher: "na",
  });
  const [style, setStyle] = useState("");
  const [critical, setCritical] = useState("");
  const [remarks, setRemarks] = useState("");
  const [claims, setClaims] = useState("");
  const [missing, setMissing] = useState("");

  useEffect(() => {
    setSummary("");
    setPromptText("");
    setCritical("");
    setRemarks("");
    setClaims("");
    setMissing("");
    setStyle("");
  }, [run?.id]);

  if (!run) return null;
  const analysis = run.kind === "analyze_materials";
  const styleScore = style.trim() === "" ? null : Number(style);
  const styleInvalid = styleScore !== null && (!Number.isInteger(styleScore) || styleScore < 0 || styleScore > 10);
  const finding = (criterion: CriterionId) => (text: string) => ({ criterion, text, location: null });
  const invalid = !summary.trim() || (analysis ? !promptText.trim() : styleInvalid);

  const submit = () => {
    if (invalid) return;
    if (analysis) {
      onAnalysis({ summary, promptText });
      return;
    }
    onReview({
      verdict,
      summary,
      correctness: criteria.correctness,
      taskFit: criteria.taskFit,
      courseGrounding: criteria.courseGrounding,
      teacher: criteria.teacher,
      styleScore,
      critical: lines(critical).map(finding("correctness")),
      remarks: lines(remarks).map(finding("task_fit")),
      unsupportedClaims: lines(claims).map(claim),
      missing: lines(missing).map((label) => ({ kind: /^https?:|ссылк/i.test(label) ? "link" : "file", label, required: true })),
    });
  };

  return (
    <Dialog
      open={!!run}
      onClose={onClose}
      wide
      title={`Ручной результат: ${RUN_KIND_LABEL[run.kind].toLowerCase()}`}
      description="AI-адаптер ещё не подключён. Выполните шаг сами (или в официальном приложении) и перенесите итог — он сохранится с отметкой «вручную»."
      footer={
        <>
          <button className="btn btn--ghost" onClick={onClose}>
            Отмена
          </button>
          <button className="btn btn--primary" onClick={submit} disabled={busy || invalid}>
            <ClipboardCheck /> Сохранить результат
          </button>
        </>
      }
    >
      <div className="manual-form">
        <label className="field">
          <span className="field__label">{analysis ? "Итог анализа материалов" : "Итог проверки"}</span>
          <textarea className="textarea" value={summary} onChange={(e) => setSummary(e.target.value)} />
        </label>
        {analysis ? (
          <label className="field">
            <span className="field__label">Промт для автора</span>
            <textarea className="textarea manual-form__prompt" value={promptText} onChange={(e) => setPromptText(e.target.value)} />
            <span className="field__hint">Сохранится черновиком: его можно поправить и затем зафиксировать.</span>
          </label>
        ) : (
          <>
            <div className="manual-form__grid">
              <label className="field">
                <span className="field__label">Итоговое решение</span>
                <select className="select" value={verdict} onChange={(e) => setVerdict(e.target.value as ReviewVerdict)}>
                  {VERDICTS.map((item) => (
                    <option key={item.value} value={item.value}>
                      {item.label}
                    </option>
                  ))}
                </select>
              </label>
              {(
                [
                  ["correctness", "Корректность решения"],
                  ["taskFit", "Соответствие условию"],
                  ["courseGrounding", "Опора на материалы курса"],
                  ["teacher", "Требования преподавателя"],
                ] as const
              ).map(([key, label]) => (
                <label className="field" key={key}>
                  <span className="field__label">{label}</span>
                  <select className="select" value={criteria[key]} onChange={(e) => setCriteria((c) => ({ ...c, [key]: e.target.value as CriterionStatus }))}>
                    {STATUSES.map((status) => (
                      <option key={status} value={status}>
                        {CRITERION_STATUS_LABEL[status]}
                      </option>
                    ))}
                  </select>
                </label>
              ))}
              <label className="field">
                <span className="field__label">Расхождение с личным стилем, 0–10</span>
                <input className="input num" inputMode="numeric" value={style} placeholder="не оценивалось" aria-invalid={styleInvalid} onChange={(e) => setStyle(e.target.value)} />
                <span className="field__hint">Эвристика сравнения с профилем стиля, не определение авторства. Порог — {run.styleThreshold}.</span>
              </label>
            </div>
            <label className="field">
              <span className="field__label">Критические ошибки — по одной в строке</span>
              <textarea className="textarea" value={critical} onChange={(e) => setCritical(e.target.value)} />
            </label>
            <label className="field">
              <span className="field__label">Замечания — по одному в строке</span>
              <textarea className="textarea" value={remarks} onChange={(e) => setRemarks(e.target.value)} />
            </label>
            <label className="field">
              <span className="field__label">Неподтверждённые утверждения — «текст — место»</span>
              <textarea className="textarea" value={claims} onChange={(e) => setClaims(e.target.value)} />
            </label>
            <label className="field">
              <span className="field__label">Не хватает обязательных файлов или ссылок — по одному в строке</span>
              <textarea className="textarea" value={missing} onChange={(e) => setMissing(e.target.value)} />
            </label>
          </>
        )}
      </div>
    </Dialog>
  );
}
