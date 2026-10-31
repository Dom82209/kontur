import { CalendarRange, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type Semester } from "../../lib/bridge";
import { formatDate } from "../../lib/format";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { ErrorState, SkeletonRows } from "../ui/States";

/** Смещения от UTC−12:00 до UTC+14:00 с шагом 30 минут (плюс редкие :45). */
export const UTC_OFFSETS: number[] = (() => {
  const values = new Set<number>();
  for (let minutes = -720; minutes <= 840; minutes += 30) values.add(minutes);
  for (const extra of [345, 525, 765, 825]) values.add(extra);
  return [...values].sort((a, b) => a - b);
})();

export function formatUtcOffset(minutes: number): string {
  const sign = minutes < 0 ? "−" : "+";
  const abs = Math.abs(minutes);
  return `UTC${sign}${String(Math.floor(abs / 60)).padStart(2, "0")}:${String(abs % 60).padStart(2, "0")}`;
}

/** Смещение часового пояса этого компьютера сейчас (минуты к востоку от UTC). */
export function localUtcOffset(): number {
  return -new Date().getTimezoneOffset();
}

/**
 * Этап 5: семестр задаёт даты всех недель. Без семестра даты недель и номер
 * текущей недели неизвестны — приложение их не выдумывает.
 */
export function SemesterPanel() {
  const toast = useToast();
  const { reportError } = useOps();
  const q = useBridgeQuery(() => bridge.getSemester(), []);
  const [startDate, setStartDate] = useState("");
  const [offset, setOffset] = useState(localUtcOffset());
  const [weekCount, setWeekCount] = useState(18);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!q.data) return;
    setStartDate(q.data.startDate);
    setOffset(q.data.utcOffsetMinutes);
    setWeekCount(q.data.weekCount);
  }, [q.data]);

  if (q.error && q.data === null && !q.loading) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  if (q.loading && q.data === null) return <SkeletonRows count={3} />;
  const semester: Semester | null = q.data;

  const save = async () => {
    setSaving(true);
    try {
      const result = await bridge.setSemester({ startDate, utcOffsetMinutes: offset, weekCount }, semester?.revision ?? null);
      q.setData(() => result.semester);
      toast.push({ tone: "ok", title: "Семестр сохранён", text: `Даты обновлены у недель: ${result.updatedWeeks}` });
    } catch (error) {
      reportError(error, "Семестр не сохранён");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="semester-panel" data-testid="semester-panel">
      {!semester && (
        <div className="notice notice--warn">
          <TriangleAlert /> Семестр не настроен: даты недель и номер текущей недели неизвестны.
        </div>
      )}
      <div className="semester-panel__fields">
        <label className="field">
          <span className="field__label">Начало недели 1</span>
          <input className="input" type="date" value={startDate} onChange={(event) => setStartDate(event.target.value)} required />
        </label>
        <label className="field">
          <span className="field__label">Часовой пояс</span>
          <select className="select" value={offset} onChange={(event) => setOffset(Number(event.target.value))}>
            {UTC_OFFSETS.map((minutes) => (
              <option key={minutes} value={minutes}>
                {formatUtcOffset(minutes)}
                {minutes === 180 ? " · Москва" : ""}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span className="field__label">Недель в семестре</span>
          <input className="input" type="number" min={1} max={60} value={weekCount} onChange={(event) => setWeekCount(Number(event.target.value))} />
        </label>
      </div>
      <p className="field__hint">Неделя начинается в 00:00 по выбранному поясу. Сохранение пересчитает даты всех недель; файлы в vault не меняются.</p>
      {semester?.warnings.map((warning) => (
        <div key={warning} className="notice notice--warn">
          <TriangleAlert /> {warning}
        </div>
      ))}
      <div className="button-row">
        <button className="btn btn--primary btn--sm" disabled={saving || !startDate} aria-busy={saving} onClick={() => void save()}>
          <CalendarRange /> {semester ? "Сохранить семестр" : "Настроить семестр"}
        </button>
        {semester && (
          <span className="faint">
            {semester.currentWeek ? `Сейчас неделя ${semester.currentWeek}` : "Сейчас вне семестра"} · {formatDate(semester.weeks[0]!.startsAt)} — {formatDate(semester.weeks[semester.weeks.length - 1]!.endsAt)}
          </span>
        )}
      </div>
    </div>
  );
}
