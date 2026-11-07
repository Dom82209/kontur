import { FilePlus2, RefreshCw, Sparkles, Trash2, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { useOps } from "../../app/operations";
import { useToast } from "../../app/toasts";
import { bridge, type StyleLanguage, type StyleProfileState, type StyleSourceKind } from "../../lib/bridge";
import { formatDateTime } from "../../lib/format";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";
import { ErrorState, SkeletonRows } from "../ui/States";

export const STYLE_SOURCE_KIND_LABEL: Record<StyleSourceKind, string> = {
  style_markdown: "Описание стиля (Markdown)",
  previous_homework: "Предыдущая домашняя работа",
  confirmed_sample: "Подтверждённый образец",
};

const PROFILE_STATUS_LABEL = { ready: "готов", insufficient_sources: "недостаточно источников", unsupported: "источники не прочитаны" } as const;
const READ_STATUS_LABEL: Record<string, string> = { unread: "не прочитан", ok: "прочитан", missing: "файл не найден", unsupported: "формат не поддерживается", empty: "нет текста" };

/** Короткий разбор признаков ревизии — только числа и списки, без текста работ. */
function FeatureSummary({ state }: { state: StyleProfileState }) {
  const f = state.current?.content?.features;
  if (!f) return null;
  const rows: [string, string][] = [
    ["Слов в образцах", String(f.words)],
    ["Длина предложения (среднее / медиана)", `${f.sentenceLengthMean} / ${f.sentenceLengthMedian}`],
    ["Предложений в абзаце", String(f.paragraphLengthMean)],
    ["Формальность", `${Math.round(f.formality * 100)}%`],
    ["Вводные на 100 предложений", String(f.introductoryPer100)],
    ["Категоричные на 100 предложений", String(f.categoricalPer100)],
    ["Запятых на предложение", String(f.commasPerSentence)],
    ["Строк-списков", `${Math.round(f.listLineShare * 100)}%`],
  ];
  return (
    <div className="style-profile__features">
      <dl className="style-profile__grid">
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt className="faint">{k}</dt>
            <dd className="num">{v}</dd>
          </div>
        ))}
      </dl>
      {f.connectives.length > 0 && <p className="faint">Связки: {f.connectives.join(", ")}</p>}
      {f.examples.length > 0 && (
        <div>
          <div className="eyebrow">Подтверждённые короткие примеры</div>
          <ul className="style-profile__examples">
            {f.examples.map((e) => (
              <li key={e}>«{e}»</li>
            ))}
          </ul>
        </div>
      )}
      {(state.current?.content?.declaredRules.length ?? 0) > 0 && (
        <div>
          <div className="eyebrow">Правила из описания стиля</div>
          <ul className="style-profile__examples">
            {state.current!.content!.declaredRules.map((r) => (
              <li key={r}>{r}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/**
 * Этап 9: профиль личного стиля. Источники выбирает только пользователь; каждая
 * сборка с изменёнными источниками — новая неизменяемая ревизия.
 */
export function StyleProfilePanel() {
  const toast = useToast();
  const { reportError } = useOps();
  const q = useBridgeQuery(() => bridge.getStyleProfileState(), []);
  const [path, setPath] = useState("");
  const [kind, setKind] = useState<StyleSourceKind>("previous_homework");
  const [language, setLanguage] = useState<StyleLanguage>("ru");
  const [busy, setBusy] = useState(false);

  if (q.error && q.data === null && !q.loading) return <ErrorState message={q.error} onRetry={() => void q.reload()} />;
  if (q.loading && q.data === null) return <SkeletonRows count={3} />;
  const state = q.data!;
  const active = state.sources.filter((s) => !s.removedAt);
  const removed = state.sources.filter((s) => s.removedAt);

  const run = async (fn: () => Promise<StyleProfileState>, title: string, failure: string) => {
    setBusy(true);
    try {
      const next = await fn();
      q.setData(() => next);
      toast.push({ tone: "ok", title });
    } catch (error) {
      reportError(error, failure);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="style-profile" data-testid="style-profile">
      <p className="field__hint">
        Эвристическая оценка соответствия личному стилю по объяснимым признакам (длина предложений, связки, пунктуация, клише). Это не «детектор ИИ»: низкое соответствие не доказывает использование нейросети. Папки не сканируются —
        используются только файлы, которые вы выбрали здесь.
      </p>

      <div className="style-profile__add">
        <label className="field">
          <span className="field__label">Путь к файлу внутри хранилища</span>
          <input className="input mono" placeholder="Предметы/ОБА/Неделя 03/03 ДЗ/04 Отправлено/ДЗ.docx" value={path} onChange={(e) => setPath(e.target.value)} />
        </label>
        <label className="field">
          <span className="field__label">Тип источника</span>
          <select className="select" value={kind} onChange={(e) => setKind(e.target.value as StyleSourceKind)}>
            {(Object.keys(STYLE_SOURCE_KIND_LABEL) as StyleSourceKind[]).map((k) => (
              <option key={k} value={k}>
                {STYLE_SOURCE_KIND_LABEL[k]}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span className="field__label">Язык</span>
          <select className="select" value={language} onChange={(e) => setLanguage(e.target.value as StyleLanguage)}>
            <option value="ru">Русский</option>
            <option value="en">English</option>
          </select>
        </label>
        <button
          className="btn btn--sm"
          disabled={busy || !path.trim()}
          onClick={() =>
            void run(
              async () => {
                const next = await bridge.addStyleSource(path.trim(), kind, language);
                setPath("");
                return next;
              },
              "Источник добавлен",
              "Источник не добавлен",
            )
          }
        >
          <FilePlus2 /> Добавить источник
        </button>
      </div>

      {active.length === 0 ? (
        <p className="faint">Источники не выбраны — проверка стиля потребует ручного подтверждения.</p>
      ) : (
        <ul className="pkg__files" data-testid="style-sources">
          {active.map((s) => (
            <li key={s.id}>
              <span className="mono">{s.relativePath}</span>
              <span className="faint">
                {" "}
                · {STYLE_SOURCE_KIND_LABEL[s.kind]} · {s.language} · {READ_STATUS_LABEL[s.readStatus] ?? s.readStatus}
                {s.sha256 ? ` · SHA ${s.sha256.slice(0, 8)}` : ""}
                {s.lastReadAt ? ` · прочитан ${formatDateTime(s.lastReadAt)}` : ""}
              </span>
              <button className="btn btn--sm btn--ghost" aria-label={`Исключить «${s.relativePath}» из следующей ревизии`} disabled={busy} onClick={() => void run(() => bridge.removeStyleSource(s.id), "Источник исключён из следующей ревизии", "Источник не исключён")}>
                <Trash2 size={13} /> Исключить
              </button>
            </li>
          ))}
        </ul>
      )}
      {removed.length > 0 && <p className="faint">Исключено из следующих ревизий: {removed.map((s) => s.relativePath).join(", ")}</p>}

      <div className="button-row">
        <button className="btn btn--sm btn--primary" disabled={busy} onClick={() => void run(() => bridge.rebuildStyleProfile(language), "Профиль стиля обновлён", "Профиль не построен")}>
          <Sparkles /> {state.current ? "Обновить профиль" : "Построить профиль"}
        </button>
        <button className="btn btn--sm" disabled={busy || active.length === 0} onClick={() => void run(() => bridge.refreshStyleSources(), "Источники перечитаны", "Источники не перечитаны")}>
          <RefreshCw /> Проверить источники
        </button>
      </div>

      {state.current && (
        <div className="style-profile__revision" data-testid="style-revision">
          <div>
            <strong>Ревизия {state.current.revision}</strong> · {formatDateTime(state.current.createdAt)} · язык {state.current.language} ·{" "}
            <span className={`badge ${state.current.status === "ready" ? "badge--ok" : "badge--warn"}`}>{PROFILE_STATUS_LABEL[state.current.status]}</span>
          </div>
          {state.stale && (
            <div className="notice notice--warn" data-testid="style-stale">
              <TriangleAlert /> Источники изменились после этой ревизии: проверки стиля, готовность, промты и комплекты по ней устарели. Обновите профиль.
            </div>
          )}
          {state.current.unusableReasons.map((r) => (
            <div key={r} className="notice notice--warn">
              <TriangleAlert /> {r}
            </div>
          ))}
          <p className="faint">Использованы: {state.current.sources.map((s) => `${s.relativePath}${s.words ? ` (${s.words} слов)` : ""}`).join(", ") || "—"}</p>
          <FeatureSummary state={state} />
          <ul className="style-profile__limits faint">
            {state.current.limitations.map((l) => (
              <li key={l}>{l}</li>
            ))}
          </ul>
          {state.history.length > 1 && <p className="faint">История: {state.history.map((h) => `ревизия ${h.revision} — ${PROFILE_STATUS_LABEL[h.status]}`).join(" · ")}</p>}
        </div>
      )}
    </div>
  );
}
