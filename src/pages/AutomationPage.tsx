import { ArrowRight, Download, FileSignature, GitBranch, Hash, Lock, ShieldCheck, Tags, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { ActivityLog } from "../components/domain/ActivityLog";
import { DownloadCandidates } from "../components/domain/DownloadsWatcher";
import { SaveBar } from "../components/ui/SaveBar";
import { ErrorState, SkeletonRows } from "../components/ui/States";
import { bridge } from "../lib/bridge";
import { formatDate } from "../lib/format";
import { useBridgeQuery } from "../lib/hooks/useBridgeQuery";
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
  const semester = useBridgeQuery(() => bridge.getSemester(), [], ["operation:completed"]);

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
                <p className="rule__sentence">
                  Сначала по номеру в пути: папка «Неделя 07» или «нед7» в имени файла.
                  {semester.data ? (
                    <>
                      {" "}Если номера нет, «Контур» предлагает неделю по дате загрузки: неделя 1 начинается <span className="token" data-testid="semester-start">{formatDate(semester.data.weeks[0]!.startsAt)}</span>. Такой файл всегда ждёт вашего подтверждения.
                    </>
                  ) : (
                    <> Семестр не настроен, поэтому файл без номера недели ждёт ручного выбора.</>
                  )}
                </p>
                <Link to="/settings" className="rule__link">
                  Начало и длина семестра — в настройках <ArrowRight size={12} />
                </Link>
                <div className="rule__inline">
                  <label htmlFor="threshold">Файл считается неоднозначным, если уверенность ниже</label>
                  <input id="threshold" type="range" min={0.4} max={0.95} step={0.05} value={rules.lowConfidenceThreshold} onChange={(e) => update({ lowConfidenceThreshold: Number(e.target.value) })} className="range" />
                  <span className="token num">{Math.round(rules.lowConfidenceThreshold * 100)}%</span>
                </div>
              </Rule>

              <Rule icon={<GitBranch />} n="04" title="Как назначается версия">
                <p className="rule__sentence">
                  Каждая новая версия ДЗ получает следующий номер: <span className="token mono">v01</span> → <span className="token mono">v02</span> → <span className="token mono">v03</span>. Номер хранится в «Контуре»; ваши файлы не переименовываются.
                </p>
              </Rule>

              <Rule icon={<FileSignature />} n="05" title="Формат итогового имени">
                <p className="rule__sentence">
                  При сборке комплекта итоговая версия копируется в «03 На сдачу» как <span className="token mono">{rules.finalNamePattern}</span>. Шаблон меняется в настройках.
                </p>
                <p className="rule__example">
                  Пример: <span className="mono">{applyNamePattern(rules.finalNamePattern, { ...rules.studentName, week: 7, subject: "Статистика", version: "v03" }, "pdf")}</span>
                </p>
              </Rule>

              <Rule icon={<ShieldCheck />} n="06" title="Подтверждения и защита файлов">
                <p className="rule__sentence">Эти гарантии встроены в «Контур» и не отключаются.</p>
                <ul className="toggles">
                  {[
                    ["Неуверенная раскладка — только с подтверждением", "Файл с уверенностью ниже порога или с неделей по дате загрузки ждёт выбора во «Входящих». Раскладка всегда запускается вами."],
                    ["Итоговая версия — через подтверждение", "Перед назначением показывается версия и итоговое имя файла."],
                    ["Статус «Отправлено» — только вручную", "Всегда требует подтверждения. «Контур» не загружает работы на сайт."],
                    ["Перезапись файлов запрещена", "Дубли сохраняются с суффиксом « (2)»; «Отменить» удаляет только файлы, созданные этой операцией."],
                  ].map(([title, hint]) => (
                    <li key={title} data-locked>
                      <div>
                        <div className="toggles__title">
                          <Lock size={12} /> {title}
                        </div>
                        <div className="toggles__hint">{hint}</div>
                      </div>
                    </li>
                  ))}
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
