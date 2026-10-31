import { Archive, ArchiveRestore, GitMerge, Pencil, Plus, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type Subject, type SubjectInput, type SubjectMergePreview } from "../../lib/bridge";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { Dialog } from "../ui/Dialog";
import { ErrorState, SkeletonRows } from "../ui/States";

/** «стат, Вероятност» → ["стат", "Вероятност"]; нормализацию выполняет backend. */
export function parseKeywords(raw: string): string[] {
  return raw
    .split(/[,;\n]/)
    .map((keyword) => keyword.trim())
    .filter(Boolean);
}

type Editing = { kind: "new" } | { kind: "edit"; subject: Subject } | null;

/**
 * Этап 3: предметы как управляемые сущности. Ни одно действие не пишет в vault:
 * папка предмета задаётся при создании и не меняется при переименовании.
 */
export function SubjectsPanel() {
  const toast = useToast();
  const { reportError } = useOps();
  const q = useBridgeQuery(() => bridge.getSubjects(), []);
  const [editing, setEditing] = useState<Editing>(null);
  const [busy, setBusy] = useState(false);
  const [merge, setMerge] = useState<{ source: Subject; preview: SubjectMergePreview | null } | null>(null);

  if (q.error && !q.data) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  if (!q.data) return <SkeletonRows count={4} />;
  const active = q.data.filter((subject) => !subject.archived);
  const archived = q.data.filter((subject) => subject.archived);
  const nameOf = (id: string | null) => q.data?.find((subject) => subject.id === id)?.name ?? "—";

  const run = async (action: () => Promise<unknown>, failure: string) => {
    setBusy(true);
    try {
      await action();
      await q.reload();
      return true;
    } catch (error) {
      reportError(error, failure);
      return false;
    } finally {
      setBusy(false);
    }
  };

  const save = async (input: SubjectInput) => {
    const target = editing;
    const ok = await run(
      () => (target?.kind === "edit" ? bridge.updateSubject(target.subject.id, target.subject.revision, input) : bridge.createSubject(input)),
      target?.kind === "edit" ? "Предмет не сохранён" : "Предмет не создан",
    );
    if (ok) {
      setEditing(null);
      toast.push({ tone: "ok", title: target?.kind === "edit" ? "Предмет сохранён" : "Предмет создан", text: input.name });
    }
  };

  const startMerge = async (source: Subject, targetId: string) => {
    if (!targetId) return;
    setBusy(true);
    try {
      setMerge({ source, preview: await bridge.previewSubjectMerge(source.id, targetId) });
    } catch (error) {
      reportError(error, "Предпросмотр слияния не выполнен");
    } finally {
      setBusy(false);
    }
  };

  const confirmMerge = async () => {
    const preview = merge?.preview;
    if (!preview) return;
    const ok = await run(() => bridge.mergeSubjects(preview.source.id, preview.target.id, preview.fingerprint), "Предметы не слиты");
    setMerge(null);
    if (ok) toast.push({ tone: "ok", title: "Предметы слиты", text: `${preview.source.name} → ${preview.target.name}` });
  };

  return (
    <div className="subjects-panel" data-testid="subjects-panel">
      <p className="field__hint">
        Изменения здесь не трогают vault: папка предмета задаётся при создании и сохраняется при переименовании. Архивный предмет не предлагается при раскладке, его недели и ДЗ остаются.
      </p>
      {editing ? (
        <SubjectForm key={editing.kind === "edit" ? editing.subject.id : "new"} initial={editing.kind === "edit" ? editing.subject : null} busy={busy} onCancel={() => setEditing(null)} onSave={(input) => void save(input)} />
      ) : (
        <div className="button-row">
          <button className="btn btn--sm" disabled={busy} onClick={() => setEditing({ kind: "new" })}>
            <Plus /> Новый предмет
          </button>
        </div>
      )}

      <ul className="subjects-panel__list">
        {active.map((subject) => (
          <li key={subject.id} className="subjects-panel__row">
            <div className="subjects-panel__main">
              <div className="subjects-panel__title">
                {subject.name} <span className="badge">{subject.shortName}</span>
                <span className="faint num"> · недель {subject.weekCount}</span>
              </div>
              <div className="field__hint mono selectable">{subject.folderPath}</div>
              <div className="subjects-panel__keywords">
                {subject.keywords.length ? subject.keywords.map((keyword) => <span key={keyword} className="token">{keyword}</span>) : <span className="faint">ключевых слов нет — предмет узнаётся только по названию</span>}
              </div>
            </div>
            <div className="subjects-panel__actions">
              <button className="btn btn--sm btn--ghost" disabled={busy} onClick={() => setEditing({ kind: "edit", subject })}>
                <Pencil /> Изменить
              </button>
              <button className="btn btn--sm btn--ghost" disabled={busy} onClick={() => void run(() => bridge.setSubjectArchived(subject.id, subject.revision, true), "Предмет не перенесён в архив")}>
                <Archive /> В архив
              </button>
              <label className="subjects-panel__merge">
                <GitMerge aria-hidden />
                <span className="sr-only">Слить «{subject.name}» с предметом</span>
                <select className="select select--sm" value="" disabled={busy} onChange={(event) => void startMerge(subject, event.target.value)} aria-label={`Слить «${subject.name}» с…`}>
                  <option value="">Слить с…</option>
                  {active
                    .filter((other) => other.id !== subject.id)
                    .map((other) => (
                      <option key={other.id} value={other.id}>
                        {other.name}
                      </option>
                    ))}
                </select>
              </label>
            </div>
          </li>
        ))}
      </ul>

      {archived.length > 0 && (
        <details className="subjects-panel__archive">
          <summary>Архив ({archived.length})</summary>
          <ul className="subjects-panel__list">
            {archived.map((subject) => (
              <li key={subject.id} className="subjects-panel__row" data-archived>
                <div className="subjects-panel__main">
                  <div className="subjects-panel__title">{subject.name}</div>
                  <div className="field__hint">{subject.mergedIntoId ? `Слит в «${nameOf(subject.mergedIntoId)}»` : `В архиве · недель ${subject.weekCount}`}</div>
                </div>
                {!subject.mergedIntoId && (
                  <button className="btn btn--sm btn--ghost" disabled={busy} onClick={() => void run(() => bridge.setSubjectArchived(subject.id, subject.revision, false), "Предмет не возвращён")}>
                    <ArchiveRestore /> Вернуть
                  </button>
                )}
              </li>
            ))}
          </ul>
        </details>
      )}

      {merge?.preview && (
        <Dialog
          open
          onClose={() => setMerge(null)}
          title="Слить предметы"
          description="Ничего не изменено. Подтвердите, чтобы применить ровно этот план."
          wide
          footer={
            <>
              <button className="btn btn--ghost" disabled={busy} onClick={() => setMerge(null)} data-close>
                Отмена
              </button>
              <button className="btn btn--primary" disabled={busy || !merge.preview.canMerge} aria-busy={busy} onClick={() => void confirmMerge()}>
                <GitMerge /> Слить
              </button>
            </>
          }
        >
          <MergePreviewBody preview={merge.preview} />
        </Dialog>
      )}
    </div>
  );
}

export function MergePreviewBody({ preview }: { preview: SubjectMergePreview }) {
  return (
    <div className="workspace-preview" data-testid="subject-merge-preview">
      <div className="workspace-preview__route">
        <span>{preview.source.name}</span>
        <GitMerge aria-label="сливается в" />
        <strong>{preview.target.name}</strong>
      </div>
      <strong>После подтверждения:</strong>
      <ul>
        {preview.effects.map((effect) => (
          <li key={effect}>{effect}</li>
        ))}
      </ul>
      {preview.movedWeeks.length > 0 && (
        <p className="field__hint">
          Недели: {preview.movedWeeks.map((week) => `${week.number}${week.hasHomework ? " (с ДЗ)" : ""}`).join(", ")}
        </p>
      )}
      {preview.warnings.map((warning) => (
        <div key={warning} className="notice notice--warn">
          <TriangleAlert /> {warning}
        </div>
      ))}
    </div>
  );
}

function SubjectForm({ initial, busy, onSave, onCancel }: { initial: Subject | null; busy: boolean; onSave: (input: SubjectInput) => void; onCancel: () => void }) {
  const [name, setName] = useState(initial?.name ?? "");
  const [shortName, setShortName] = useState(initial?.shortName ?? "");
  const [keywords, setKeywords] = useState(initial?.keywords.join(", ") ?? "");
  return (
    <form
      className="subjects-panel__form"
      onSubmit={(event) => {
        event.preventDefault();
        onSave({ name, shortName, keywords: parseKeywords(keywords) });
      }}
    >
      <label className="field">
        <span className="field__label">Название</span>
        <input className="input" value={name} onChange={(event) => setName(event.target.value)} required maxLength={80} data-autofocus />
        {initial && <span className="field__hint">Папка в vault останется прежней: {initial.folderPath}</span>}
      </label>
      <label className="field">
        <span className="field__label">Короткое название</span>
        <input className="input" value={shortName} onChange={(event) => setShortName(event.target.value)} required maxLength={12} />
      </label>
      <label className="field">
        <span className="field__label">Ключевые слова через запятую</span>
        <input className="input" value={keywords} onChange={(event) => setKeywords(event.target.value)} placeholder="стат, вероятност" />
        <span className="field__hint">По ним файлы узнаются при раскладке. Одно слово не может относиться к двум предметам.</span>
      </label>
      <div className="button-row">
        <button type="button" className="btn btn--ghost" disabled={busy} onClick={onCancel}>
          Отмена
        </button>
        <button type="submit" className="btn btn--primary" disabled={busy} aria-busy={busy}>
          {initial ? "Сохранить" : "Создать"}
        </button>
      </div>
    </form>
  );
}
