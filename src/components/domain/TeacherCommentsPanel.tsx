import { AnimatePresence, motion } from "motion/react";
import { FileUp, MessagesSquare, Paperclip, Plus, ScanSearch, Trash2 } from "lucide-react";
import { useRef, useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type TeacherComment, type TeacherCommentSource, type TeacherRequirement } from "../../lib/bridge";
import { COMMENT_SOURCE_LABEL, formatDate, plural } from "../../lib/format";
import { Segmented } from "../ui/Segmented";
import { EmptyState } from "../ui/States";
import { RequirementList } from "./RequirementList";

type Mode = "paste" | "file" | "manual";

interface Props {
  homeworkId: string;
  comments: TeacherComment[];
  requirements: TeacherRequirement[];
  locked: boolean;
  running: boolean;
  canReview: boolean;
  onReview: () => void;
  /** Перезагрузить данные после правок без события backend. */
  onChanged: () => void;
}

const today = () => new Date().toISOString().slice(0, 10);

export function TeacherCommentsPanel({ homeworkId, comments, requirements, locked, running, canReview, onReview, onChanged }: Props) {
  const toast = useToast();
  const { reportError } = useOps();
  const [mode, setMode] = useState<Mode>("paste");
  const [source, setSource] = useState<TeacherCommentSource>("chat");
  const [date, setDate] = useState(today);
  const [text, setText] = useState("");
  const [q, setQ] = useState("");
  const [a, setA] = useState("");
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const iso = new Date(`${date}T12:00:00`).toISOString();

  const done = (n: number) => toast.push({ tone: "ok", title: `Добавлено ${n} ${plural(n, "комментарий", "комментария", "комментариев")}`, text: "Запустите повторную проверку по всем комментариям" });

  const submit = async (fn: () => Promise<{ value: TeacherComment[] }>) => {
    setBusy(true);
    try {
      const res = await fn();
      done(res.value.length);
      setText("");
      setQ("");
      setA("");
    } catch (e) {
      reportError(e, "Комментарии не сохранены");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="teacher">
      {!locked && (
        <section className="panel teacher__input">
          <div className="panel__head">
            <h2 className="panel__title">Добавить комментарии</h2>
          </div>
          <div className="panel__body teacher__form">
            <Segmented
              label="Способ добавления"
              value={mode}
              onChange={setMode}
              options={[
                { value: "paste", label: "Вставить переписку" },
                { value: "file", label: "Загрузить файл" },
                { value: "manual", label: "Вопрос и ответ" },
              ]}
            />
            <div className="teacher__meta">
              <label className="field">
                <span className="field__label">Источник</span>
                <select className="select" value={source} onChange={(e) => setSource(e.target.value as TeacherCommentSource)}>
                  {(Object.keys(COMMENT_SOURCE_LABEL) as TeacherCommentSource[]).map((k) => (
                    <option key={k} value={k}>
                      {COMMENT_SOURCE_LABEL[k]}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span className="field__label">Дата</span>
                <input type="date" className="input" value={date} onChange={(e) => setDate(e.target.value)} />
              </label>
            </div>
            <AnimatePresence mode="wait" initial={false}>
              <motion.div key={mode} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }} transition={{ duration: 0.15 }} className="teacher__mode">
                {mode === "paste" && (
                  <>
                    <label className="field">
                      <span className="field__label">Текст переписки</span>
                      <textarea className="textarea" style={{ minHeight: 120, fontFamily: "var(--font-sans)", fontSize: 13 }} placeholder={"В: Можно ли…?\nО: Да, но…"} value={text} onChange={(e) => setText(e.target.value)} />
                      <span className="field__hint">Строки «В:» / «О:» (или «Вопрос:» / «Ответ:») разбиваются на пары автоматически.</span>
                    </label>
                    <button className="btn btn--primary" disabled={busy || !text.trim()} onClick={() => void submit(() => bridge.importTeacherTranscript(homeworkId, { text, source, date: iso }))}>
                      <MessagesSquare /> Разобрать и сохранить
                    </button>
                  </>
                )}
                {mode === "file" && (
                  <>
                    <p className="faint">Markdown, TXT, PDF или изображение. Текст извлечёт backend; файл сохранится рядом с условием.</p>
                    <button className="btn" disabled={busy} onClick={() => fileRef.current?.click()}>
                      <FileUp /> Выбрать файл
                    </button>
                    <input
                      ref={fileRef}
                      type="file"
                      hidden
                      accept=".md,.txt,.pdf,image/*"
                      onChange={(e) => {
                        const f = e.target.files?.[0];
                        e.target.value = "";
                        if (f) void submit(() => bridge.importTeacherTranscript(homeworkId, { file: { name: f.name, sizeBytes: f.size }, source, date: iso }));
                      }}
                    />
                  </>
                )}
                {mode === "manual" && (
                  <>
                    <label className="field">
                      <span className="field__label">Вопрос</span>
                      <input className="input" value={q} onChange={(e) => setQ(e.target.value)} />
                    </label>
                    <label className="field">
                      <span className="field__label">Ответ преподавателя</span>
                      <textarea className="textarea" style={{ minHeight: 70, fontFamily: "var(--font-sans)", fontSize: 13 }} value={a} onChange={(e) => setA(e.target.value)} />
                    </label>
                    <button className="btn btn--primary" disabled={busy || !a.trim()} onClick={() => void submit(() => bridge.saveTeacherComments(homeworkId, [{ source, date: iso, question: q, answer: a }]))}>
                      <Plus /> Добавить
                    </button>
                  </>
                )}
              </motion.div>
            </AnimatePresence>
          </div>
        </section>
      )}

      <section className="panel teacher__list">
        <div className="panel__head">
          <h2 className="panel__title">Комментарии · {comments.length}</h2>
          <div className="panel__meta">
            <button className="btn btn--sm btn--primary" onClick={onReview} disabled={!canReview || running || comments.length === 0 || locked}>
              <ScanSearch /> Проверить по всем комментариям
            </button>
          </div>
        </div>
        <div className="panel__body">
          {comments.length === 0 ? (
            <EmptyState icon={<MessagesSquare />} title="Комментариев пока нет" text="Добавьте вопросы и ответы из чата с преподавателем — проверка сверит с ними версию." />
          ) : (
            <ul className="tcomments">
              <AnimatePresence initial={false}>
                {comments.map((c) => (
                  <motion.li key={c.id} layout className="tcomment" initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
                    <div className="tcomment__meta faint">
                      {COMMENT_SOURCE_LABEL[c.source]} · {formatDate(c.date)}
                      {c.attachment && (
                        <span className="tcomment__att">
                          <Paperclip size={11} /> {c.attachment.name}
                        </span>
                      )}
                    </div>
                    {c.question && <div className="tcomment__q">{c.question}</div>}
                    <div className="tcomment__a">{c.answer}</div>
                    {!locked && (
                      <button className="btn btn--sm btn--ghost btn--icon tcomment__del" aria-label="Удалить комментарий" onClick={() => void bridge.removeTeacherComment(c.id).then(onChanged, (e: unknown) => reportError(e))}>
                        <Trash2 />
                      </button>
                    )}
                  </motion.li>
                ))}
              </AnimatePresence>
            </ul>
          )}
        </div>
      </section>

      {requirements.length > 0 && (
        <section className="panel teacher__reqs">
          <div className="panel__head">
            <h2 className="panel__title">Требования преподавателя</h2>
            <span className="panel__meta num">
              {requirements.filter((r) => r.status === "done" || r.status === "na").length}/{requirements.length}
            </span>
          </div>
          <div className="panel__body">
            <RequirementList
              requirements={requirements}
              comments={comments}
              locked={locked}
              onResolve={(r, status) => void bridge.resolveRequirement(r.id, status, "Проверено вручную").then(onChanged, (e: unknown) => reportError(e))}
            />
          </div>
        </section>
      )}
    </div>
  );
}
