import { ArrowRight, Download, FileSignature, GitBranch, Hash, Lock, ShieldCheck, Tags, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { ActivityLog } from "../components/domain/ActivityLog";
import { DownloadCandidates } from "../components/domain/DownloadsWatcher";
import { SaveBar } from "../components/ui/SaveBar";
import { Segmented } from "../components/ui/Segmented";
import { ErrorState, SkeletonRows } from "../components/ui/States";
import { Switch } from "../components/ui/Switch";
import { formatDate } from "../lib/format";
import { useRulesDraft } from "../lib/hooks/useRulesDraft";
import { useSubjectNames } from "../lib/hooks/useSubjectNames";
import { applyNamePattern } from "../lib/naming";

function Rule({ icon, n, title, children }: { icon: ReactNode; n: string; title: string; children: ReactNode }) {
  return (
    <section className="rule">
      <div className="rule__gutter">
        <span className="rule__n num">{n}</span>
        <span className="rule__icon">{icon}</span>
      </div>
      <div className="rule__body">
        <h2 className="rule__title">{title}</h2>
        {children}
      </div>
    </section>
  );
}

function KeywordEditor({ keywords, onChange, label }: { keywords: string[]; onChange: (k: string[]) => void; label: string }) {
  const [value, setValue] = useState("");
  return (
    <div className="kw">
      {keywords.map((k) => (
        <span key={k} className="kw__chip mono">
          {k}
          <button aria-label={`Удалить «${k}» (${label})`} onClick={() => onChange(keywords.filter((x) => x !== k))}>
            <X size={11} />
          </button>
        </span>
      ))}
      <input
        className="kw__input"
        placeholder="+ слово"
        aria-label={`Добавить ключевое слово: ${label}`}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && value.trim()) {
            const v = value.trim().toLowerCase();
            if (!keywords.includes(v)) onChange([...keywords, v]);
            setValue("");
          }
        }}
      />
    </div>
  );
}

export function AutomationPage() {
  const { rules, error, reload, dirty, saving, update, save, reset } = useRulesDraft();
  const names = useSubjectNames();

  if (error && !rules) return <ErrorState message={error} onRetry={() => void reload()} />;

  return (
    <div className="auto">
      <header className="pagehead">
        <div>
          <span className="eyebrow">Автоматизация</span>
          <h1 className="pagehead__title">Правила, по которым «Контур» раскладывает файлы</h1>
          <p className="muted">Правила исполняет backend. Здесь — то, что он делает, в человеческом виде. Перезапись файлов и отправка работ никогда не автоматизируются.</p>
        </div>
      </header>

      <div className="auto__layout">
        <div className="rules">
          {!rules ? (
            <SkeletonRows count={6} />
          ) : (
            <>
              <Rule icon={<Download />} n="01" title="Куда попадают скачанные файлы">
                <p className="rule__sentence">
                  Если наблюдение включено, «Контур» замечает завершённые загрузки в выбранной папке и показывает их кандидатами. Ничего не копируется, пока вы не откроете предпросмотр и не подтвердите импорт во <span className="token mono">{rules.inboxPath}</span>.
                </p>
                <p className="rule__sentence">
                  Экспорт CU LMS Enhancer (ZIP с материалами и ДЗ, ZIP без ДЗ или PDF) проверяется до распаковки: опасные пути, ссылки, вложенные архивы и zip-бомбы отклоняются. Вход в LMS и отправка работ не автоматизируются.
                </p>
                <DownloadCandidates />
                <Link to="/settings" className="rule__link">
                  Папка загрузок и включение наблюдения — в настройках <ArrowRight size={12} />
                </Link>
              </Rule>

              <Rule icon={<Tags />} n="02" title="Как определяется предмет">
                <p className="rule__sentence">По ключевым словам в имени файла. Выигрывает самое длинное совпадение; «продвинутые» курсы — по словам «продв», «adv», «+».</p>
                <div className="kwtable">
                  {rules.subjectRules.map((r) => (
                    <div key={r.subjectId} className="kwtable__row">
                      <span className="kwtable__subject">{names(r.subjectId)}</span>
                      <KeywordEditor
                        label={names(r.subjectId)}
                        keywords={r.keywords}
                        onChange={(k) => update((d) => ({ ...d, subjectRules: d.subjectRules.map((x) => (x.subjectId === r.subjectId ? { ...x, keywords: k } : x)) }))}
                      />
                    </div>
                  ))}
                </div>
              </Rule>

              <Rule icon={<Hash />} n="03" title="Как определяется неделя">
                <Segmented
                  label="Способ определения недели"
                  value={rules.weekDetection}
                  onChange={(v) => update({ weekDetection: v })}
                  options={[
                    { value: "filename_then_calendar", label: "Имя файла, затем календарь" },
                    { value: "calendar", label: "Только календарь" },
                  ]}
                />
                <p className="rule__sentence">
                  Неделя 1 начинается <span className="token">{formatDate(rules.semesterStart)}</span>. Файлы без номера в имени получают неделю по дате загрузки.
                </p>
                <div className="rule__inline">
                  <label htmlFor="threshold">Файл считается неоднозначным, если уверенность ниже</label>
                  <input id="threshold" type="range" min={0.4} max={0.95} step={0.05} value={rules.lowConfidenceThreshold} onChange={(e) => update({ lowConfidenceThreshold: Number(e.target.value) })} className="range" />
                  <span className="token num">{Math.round(rules.lowConfidenceThreshold * 100)}%</span>
                </div>
              </Rule>

              <Rule icon={<GitBranch />} n="04" title="Как назначается версия">
                <p className="rule__sentence">
                  Каждый новый черновик в «02 Работа» получает следующий номер: <span className="token mono">v01</span> → <span className="token mono">v02</span> → <span className="token mono">v03</span>. Номера не переиспользуются, даже после отмены. Шаблон: <span className="token mono">{rules.versionPattern}</span>.
                </p>
              </Rule>

              <Rule icon={<FileSignature />} n="05" title="Формат итогового имени">
                <p className="rule__sentence">
                  Итоговая версия копируется в «03 На сдачу» как <span className="token mono">{rules.finalNamePattern}</span>
                </p>
                <p className="rule__example">
                  Пример: <span className="mono">{applyNamePattern(rules.finalNamePattern, { ...rules.studentName, week: 7, subject: "Статистика", version: "v03" }, "pdf")}</span>
                </p>
              </Rule>

              <Rule icon={<ShieldCheck />} n="06" title="Резервная копия и подтверждения">
                <ul className="toggles">
                  <li>
                    <div>
                      <div className="toggles__title">Резервная копия перед любым изменением</div>
                      <div className="toggles__hint">Копия кладётся в .kontur/backup, по ней работает «Отменить».</div>
                    </div>
                    <Switch label="Резервная копия" checked={rules.backupBeforeChange} onChange={(v) => update({ backupBeforeChange: v })} />
                  </li>
                  <li>
                    <div>
                      <div className="toggles__title">Раскладывать уверенные файлы без участия</div>
                      <div className="toggles__hint">Если выключено, файлы ждут во «Входящих» нажатия «Разложить».</div>
                    </div>
                    <Switch label="Автораскладка" checked={rules.autoRouteHighConfidence} onChange={(v) => update({ autoRouteHighConfidence: v })} />
                  </li>
                  <li>
                    <div>
                      <div className="toggles__title">Подтверждать неуверенную раскладку</div>
                    </div>
                    <Switch label="Подтверждать неуверенную раскладку" checked={rules.confirm.lowConfidenceRoute} onChange={(v) => update((d) => ({ ...d, confirm: { ...d.confirm, lowConfidenceRoute: v } }))} />
                  </li>
                  <li>
                    <div>
                      <div className="toggles__title">Подтверждать назначение итоговой версии</div>
                    </div>
                    <Switch label="Подтверждать итоговую" checked={rules.confirm.markFinal} onChange={(v) => update((d) => ({ ...d, confirm: { ...d.confirm, markFinal: v } }))} />
                  </li>
                  <li data-locked>
                    <div>
                      <div className="toggles__title">
                        <Lock size={12} /> Статус «Отправлено» — только вручную
                      </div>
                      <div className="toggles__hint">Всегда требует подтверждения. «Контур» не загружает работы на сайт.</div>
                    </div>
                    <Switch label="Подтверждение отправки" checked onChange={() => undefined} disabled />
                  </li>
                  <li data-locked>
                    <div>
                      <div className="toggles__title">
                        <Lock size={12} /> Перезапись файлов
                      </div>
                      <div className="toggles__hint">Запрещена. Дубли сохраняются с суффиксом « (2)».</div>
                    </div>
                    <Switch label="Перезапись" checked={false} onChange={() => undefined} disabled />
                  </li>
                </ul>
              </Rule>
            </>
          )}
        </div>

        <section className="panel auto__journal" aria-label="Журнал операций">
          <div className="panel__head">
            <h2 className="panel__title">Журнал последних операций</h2>
          </div>
          <div className="panel__body">
            <ActivityLog />
          </div>
        </section>
      </div>
      <SaveBar visible={dirty} saving={saving} onSave={() => void save()} onReset={reset} />
    </div>
  );
}
