import { CalendarClock } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type Homework } from "../../lib/bridge";

/** ISO (UTC) → значение `datetime-local` в часовом поясе этого компьютера. */
export function toLocalInput(iso: string | null): string {
  if (!iso) return "";
  const date = new Date(iso);
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

/** `datetime-local` (местное время) → ISO с часовым поясом; пусто — срок неизвестен. */
export function fromLocalInput(value: string): string | null {
  return value ? new Date(value).toISOString() : null;
}

/**
 * Этап 5: точный срок ДЗ задаётся вручную или остаётся «не указан».
 * Сданное ДЗ неизменяемо — редактор не показывается.
 */
export function DeadlineEditor({ homework, onSaved }: { homework: Homework; onSaved: () => void }) {
  const { reportError } = useOps();
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState(toLocalInput(homework.deadline));
  const [saving, setSaving] = useState(false);
  if (homework.stage === "submitted") return null;

  const save = async (deadline: string | null) => {
    setSaving(true);
    try {
      await bridge.setHomeworkDeadline(homework.id, homework.revision, deadline);
      setOpen(false);
      onSaved();
    } catch (error) {
      reportError(error, "Срок не сохранён");
    } finally {
      setSaving(false);
    }
  };

  if (!open)
    return (
      <button className="btn btn--sm btn--ghost" onClick={() => { setValue(toLocalInput(homework.deadline)); setOpen(true); }} data-testid="deadline-edit">
        <CalendarClock /> {homework.deadline ? "Изменить срок" : "Указать срок"}
      </button>
    );
  return (
    <form
      className="deadline-editor"
      onSubmit={(event) => {
        event.preventDefault();
        void save(fromLocalInput(value));
      }}
    >
      <input className="input" type="datetime-local" value={value} onChange={(event) => setValue(event.target.value)} aria-label="Срок сдачи" />
      <button type="submit" className="btn btn--sm btn--primary" disabled={saving || !value} aria-busy={saving}>
        Сохранить
      </button>
      {homework.deadline && (
        <button type="button" className="btn btn--sm btn--ghost" disabled={saving} onClick={() => void save(null)}>
          Срок неизвестен
        </button>
      )}
      <button type="button" className="btn btn--sm btn--ghost" disabled={saving} onClick={() => setOpen(false)}>
        Отмена
      </button>
    </form>
  );
}
