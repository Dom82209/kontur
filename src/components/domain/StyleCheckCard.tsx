import { PenLine, ScanText, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { bridge, type StyleCheck, type StyleCheckStatus } from "../../lib/bridge";
import { formatDateTime } from "../../lib/format";

export const STYLE_CHECK_STATUS_LABEL: Record<StyleCheckStatus, string> = {
  checked: "проверено",
  not_checked: "не проверено",
  stale: "устарело",
  insufficient_sources: "недостаточно источников",
  unsupported: "не поддерживается",
  manual_required: "требуется ручная проверка",
};

const FEATURE_LABEL: Record<string, string> = { cliche: "клише", sentence_length: "длина предложения", categorical: "категоричность" };

/**
 * Этап 9: соответствие личному стилю для итоговой версии. Оценка эвристическая;
 * без оценки проверка не считается пройденной.
 */
export function StyleCheckCard({ homeworkId, check, locked, onChanged }: { homeworkId: string; check: StyleCheck | null; locked: boolean; onChanged: () => void }) {
  const { reportError } = useOps();
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const act = async (fn: () => Promise<unknown>, failure: string) => {
    setBusy(true);
    try {
      await fn();
      onChanged();
    } catch (error) {
      reportError(error, failure);
    } finally {
      setBusy(false);
    }
  };
  const canConfirm =
    !!check && !check.confirmedAt && (check.status === "checked" ? (check.score ?? 0) > check.threshold : ["manual_required", "insufficient_sources", "unsupported"].includes(check.status));

  return (
    <section className="panel" data-testid="style-check">
      <div className="panel__head">
        <h2 className="panel__title">Соответствие личному стилю</h2>
        {check && (
          <span className="panel__meta faint">
            {check.profileRevision ? `профиль, ревизия ${check.profileRevision} · ` : ""}
            {formatDateTime(check.createdAt)}
          </span>
        )}
      </div>
      <div className="panel__body">
        <p className="field__hint">Эвристическая оценка по объяснимым признакам, не «детектор ИИ»: низкое соответствие не доказывает использование нейросети и не определяет авторство.</p>
        {!check ? (
          <p className="faint">Стиль итоговой версии ещё не проверялся.</p>
        ) : (
          <>
            <div className="style-check__head">
              <span className={`badge ${check.passes ? "badge--ok" : check.status === "stale" ? "badge--danger" : "badge--warn"}`}>{STYLE_CHECK_STATUS_LABEL[check.status]}</span>
              {check.score !== null && (
                <span className="num">
                  Расхождение {check.score}/10 · порог {check.threshold}
                </span>
              )}
              {check.confirmedAt && <span className="badge badge--ok">подтверждено вручную</span>}
            </div>
            <p>{check.body.explanation}</p>
            {check.status === "stale" && (
              <div className="notice notice--warn">
                <TriangleAlert /> После проверки изменились профиль стиля, версия или требования — проверьте снова.
              </div>
            )}
            {check.body.deltas.length > 0 && (
              <ul className="pkg__files">
                {check.body.deltas.map((d) => (
                  <li key={d.feature}>
                    {d.label}: в ваших работах <span className="num">{d.profile}</span>, здесь <span className="num">{d.checked}</span>
                  </li>
                ))}
              </ul>
            )}
            {check.body.fragments.length > 0 && (
              <div data-testid="style-fragments">
                <div className="eyebrow">Нехарактерные фрагменты</div>
                <ul className="style-check__fragments">
                  {check.body.fragments.map((f) => (
                    <li key={f.text}>
                      <blockquote>{f.text}</blockquote>
                      <div className="faint">
                        {f.features.map((x) => FEATURE_LABEL[x] ?? x).join(", ")} — {f.explanation}
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {check.body.sampleRefs.length > 0 && <p className="faint">Сравнение с: {check.body.sampleRefs.join(", ")}</p>}
          </>
        )}
        {!locked && (
          <div className="button-row">
            <button className="btn btn--sm" disabled={busy} onClick={() => void act(() => bridge.runStyleCheck(homeworkId), "Стиль не проверен")}>
              <ScanText /> {check ? "Проверить снова" : "Проверить стиль"}
            </button>
            {canConfirm && (
              <>
                <input className="input" placeholder="Что проверили вручную" value={note} onChange={(e) => setNote(e.target.value)} />
                <button className="btn btn--sm" disabled={busy} onClick={() => void act(() => bridge.confirmStyleCheck(check!.id, note), "Не подтверждено")}>
                  <PenLine /> Подтвердить вручную
                </button>
              </>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
