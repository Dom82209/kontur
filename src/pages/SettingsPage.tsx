import { BrainCircuit, Gauge, Monitor, Moon, PenLine, ScanSearch, ShieldCheck, Sun } from "lucide-react";
import { usePrefs } from "../app/preferences";
import { ProviderCard } from "../components/domain/ProviderCard";
import { SaveBar } from "../components/ui/SaveBar";
import { Segmented } from "../components/ui/Segmented";
import { ErrorState, SkeletonRows } from "../components/ui/States";
import { bridge } from "../lib/bridge";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
import { useRulesDraft } from "../lib/hooks/useRulesDraft";
import { NAME_TOKENS, applyNamePattern } from "../lib/naming";

const PROVIDERS = [
  { value: "codex", label: "Codex" },
  { value: "claude", label: "Claude" },
] as const;

export function SettingsPage() {
  const prefs = usePrefs();
  const { rules, error, reload, dirty, saving, update, save, reset } = useRulesDraft();
  const providers = useBridgeQuery(() => Promise.all([bridge.getProviderStatus("codex"), bridge.getProviderStatus("claude")]), [], ["provider:status-changed"]);

  if (error && !rules) return <ErrorState message={error} onRetry={() => void reload()} />;

  return (
    <div className="settings">
      <header className="pagehead">
        <div>
          <span className="eyebrow">Настройки</span>
          <h1 className="pagehead__title">Хранилище, имена и внешний вид</h1>
        </div>
      </header>

      <div className="settings__grid">
        <section className="settings__group">
          <h2 className="settings__h">Хранилище</h2>
          {!rules ? (
            <SkeletonRows count={3} />
          ) : (
            <div className="settings__fields">
              <label className="field">
                <span className="field__label">Путь к хранилищу Obsidian</span>
                <input className="input mono" value={rules.obsidianVaultPath} onChange={(e) => update({ obsidianVaultPath: e.target.value })} spellCheck={false} />
                <span className="field__hint">Структура предметов и недель создаётся внутри. Выбор папки через Finder подключит backend.</span>
              </label>
              <label className="field">
                <span className="field__label">Папка «Входящие»</span>
                <input className="input mono" value={rules.inboxPath} onChange={(e) => update({ inboxPath: e.target.value })} spellCheck={false} />
              </label>
              <label className="field">
                <span className="field__label">Наблюдать за загрузками</span>
                <input className="input mono" value={rules.downloadsWatchPath} onChange={(e) => update({ downloadsWatchPath: e.target.value })} spellCheck={false} />
              </label>
            </div>
          )}
        </section>

        <section className="settings__group">
          <h2 className="settings__h">Имя итогового файла</h2>
          {rules && (
            <div className="settings__fields">
              <div className="settings__row2">
                <label className="field">
                  <span className="field__label">Фамилия</span>
                  <input className="input" value={rules.studentName.lastName} onChange={(e) => update({ studentName: { ...rules.studentName, lastName: e.target.value } })} />
                </label>
                <label className="field">
                  <span className="field__label">Имя</span>
                  <input className="input" value={rules.studentName.firstName} onChange={(e) => update({ studentName: { ...rules.studentName, firstName: e.target.value } })} />
                </label>
              </div>
              <label className="field">
                <span className="field__label">Шаблон</span>
                <input id="pattern" className="input mono" value={rules.finalNamePattern} onChange={(e) => update({ finalNamePattern: e.target.value })} spellCheck={false} />
              </label>
              <div className="tokens" aria-label="Вставить токен">
                {NAME_TOKENS.map((t) => (
                  <button key={t} className="btn btn--sm mono" onClick={() => update({ finalNamePattern: `${rules.finalNamePattern}${t}` })}>
                    {t}
                  </button>
                ))}
              </div>
              <div className="namepreview">
                <span className="eyebrow">Предпросмотр</span>
                <span className="mono">{applyNamePattern(rules.finalNamePattern, { ...rules.studentName, week: 7, subject: "Основы бизнес-аналитики", version: "v02" }, "xlsx")}</span>
              </div>
              {!rules.finalNamePattern.includes("{Неделя}") && <div className="notice notice--warn">Без {"{Неделя}"} имена разных недель совпадут — сохранение будет отклонено.</div>}
            </div>
          )}
        </section>

        <section className="settings__group">
          <h2 className="settings__h">Внешний вид</h2>
          <div className="settings__fields">
            <div className="settings__line">
              <div>
                <div className="toggles__title">Тема</div>
                <div className="toggles__hint">Светлая тема собрана отдельно: холодная бумага и медные акценты.</div>
              </div>
              <Segmented
                label="Тема"
                value={prefs.theme}
                onChange={prefs.setTheme}
                options={[
                  { value: "dark", label: "Тёмная", icon: <Moon /> },
                  { value: "light", label: "Светлая", icon: <Sun /> },
                  { value: "system", label: "Система", icon: <Monitor /> },
                ]}
              />
            </div>
            <div className="settings__line">
              <div>
                <div className="toggles__title">Анимации</div>
                <div className="toggles__hint">«Меньше» отключает перемещения и циклы, оставляя смену прозрачности. «Система» следует настройке macOS.</div>
              </div>
              <Segmented
                label="Анимации"
                value={prefs.motion}
                onChange={prefs.setMotion}
                options={[
                  { value: "full", label: "Полные", icon: <Gauge /> },
                  { value: "reduced", label: "Меньше" },
                  { value: "system", label: "Система" },
                ]}
              />
            </div>
            <button
              className="btn btn--sm btn--ghost"
              style={{ justifySelf: "start" }}
              onClick={() => {
                sessionStorage.removeItem("kontur.intro-shown");
                location.reload();
              }}
            >
              Показать стартовую анимацию снова
            </button>
          </div>
        </section>

        <section className="settings__group">
          <h2 className="settings__h">Нейросети</h2>
          <div className="settings__fields">{providers.data ? providers.data.map((p) => <ProviderCard key={p.provider} status={p} />) : <SkeletonRows count={2} />}</div>
        </section>

        <section className="settings__group settings__group--wide">
          <h2 className="settings__h">
            <BrainCircuit size={16} /> Рабочий процесс AI
          </h2>
          {!rules ? (
            <SkeletonRows count={4} />
          ) : (
            <div className="workflow-settings">
              <div className="workflow-settings__roles">
                {([
                  ["analyst", "Анализ материалов", "Аналитик изучает материалы и создаёт первоначальный промт.", BrainCircuit],
                  ["author", "Создание решения", "Автор создаёт решение по подготовленному промту.", PenLine],
                  ["reviewer", "Проверка решения", "Проверяющий проверяет решение и формирует исправления.", ScanSearch],
                ] as const).map(([role, label, hint, Icon]) => (
                  <label className="workflow-role" key={role}>
                    <span className="workflow-role__icon" aria-hidden><Icon /></span>
                    <span className="workflow-role__copy">
                      <span className="field__label">{label}</span>
                      <span className="field__hint">{hint}</span>
                    </span>
                    <select
                      className="select workflow-role__select"
                      aria-label={`${label}: провайдер`}
                      value={rules.workflow.roles[role]}
                      onChange={(e) => update((r) => ({ ...r, workflow: { ...r.workflow, roles: { ...r.workflow.roles, [role]: e.target.value as "codex" | "claude" } } }))}
                    >
                      {PROVIDERS.map((p) => <option key={p.value} value={p.value}>{p.label}</option>)}
                    </select>
                  </label>
                ))}
              </div>
              <label className="style-threshold">
                <span className="style-threshold__copy">
                  <span className="field__label">Допустимое расхождение с личным стилем</span>
                  <span className="field__hint">Оценка 0–10 сравнивает работу с профилем вашего стиля. Это не определение AI-авторства.</span>
                </span>
                <input
                  className="style-threshold__range"
                  type="range"
                  min="0"
                  max="10"
                  step="1"
                  value={rules.workflow.styleThreshold}
                  aria-valuetext={`${rules.workflow.styleThreshold} из 10`}
                  onChange={(e) => update((r) => ({ ...r, workflow: { ...r.workflow, styleThreshold: Number(e.target.value) } }))}
                />
                <output className="style-threshold__value num" aria-live="polite">{rules.workflow.styleThreshold}/10</output>
              </label>
            </div>
          )}
        </section>

        <section className="settings__group settings__group--wide">
          <h2 className="settings__h">
            <ShieldCheck size={15} /> Безопасность и хранение данных
          </h2>
          <ul className="security">
            <li>
              <strong>Пароли, cookies, токены и API-ключи «Контур» не хранит и не запрашивает.</strong> Вход в Codex и Claude выполняется в их официальных клиентах; приложение получает только статус подключения.
            </li>
            <li>Веб-интерфейсы claude.ai и ChatGPT не встраиваются в приложение.</li>
            <li>Все файлы остаются в вашем хранилище Obsidian. Frontend не обращается к файловой системе напрямую — только через типизированный мост.</li>
            <li>Перед каждым изменением файлов создаётся резервная копия; любую файловую операцию можно отменить из журнала.</li>
            <li>Отправка работ на сайт университета не автоматизируется — статус «Отправлено» ставится только вручную.</li>
            <li>В локальном хранилище окна сохраняются только тема и настройки анимации.</li>
          </ul>
          <p className="faint" style={{ marginTop: 10 }}>
            Режим данных: <span className="mono">{bridge.kind === "mock" ? "демонстрационный (mock adapter)" : "desktop backend"}</span>
          </p>
        </section>
      </div>
      <SaveBar visible={dirty} saving={saving} onSave={() => void save()} onReset={reset} />
    </div>
  );
}
