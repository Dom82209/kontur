import { AnimatePresence, motion } from "motion/react";
import { BrainCircuit, CalendarRange, CornerDownLeft, FileCheck2, Gauge, Moon, Plug, Search, Undo2, Upload } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { NAV, weekPath } from "../../app/nav";
import { useOps } from "../../app/operations";
import { usePrefs } from "../../app/preferences";
import { useToast } from "../../app/toasts";
import { bridge, type Homework, type Subject } from "../../lib/bridge";

interface Command {
  id: string;
  group: string;
  title: string;
  hint?: string;
  icon: ReactNode;
  keywords: string;
  run: () => void;
}

export function CommandPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const { undo } = useOps();
  const toast = useToast();
  const { resolvedTheme, setTheme, reduceMotion, setMotion } = usePrefs();
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const [subjects, setSubjects] = useState<Subject[]>([]);
  const [homework, setHomework] = useState<Homework[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setIndex(0);
    void bridge.getSubjects().then(setSubjects);
    void bridge.getHomeworkList().then(setHomework);
    const t = setTimeout(() => inputRef.current?.focus(), 20);
    return () => clearTimeout(t);
  }, [open]);

  const commands = useMemo<Command[]>(() => {
    const go = (to: string) => () => navigate(to);
    const list: Command[] = [
      ...NAV.map((n) => {
        const Icon = n.icon;
        return { id: `nav:${n.to}`, group: "Переход", title: n.label, icon: <Icon />, keywords: n.keywords, hint: n.shortcut ? `⌘${n.shortcut}` : undefined, run: go(n.to) };
      }),
      { id: "act:undo", group: "Действия", title: "Отменить последнюю файловую операцию", icon: <Undo2 />, keywords: "отмена откат undo", hint: "⌘Z", run: () => void undo() },
      {
        id: "act:drop",
        group: "Действия",
        title: "Симулировать загрузку файлов во «Входящие»",
        icon: <Upload />,
        keywords: "загрузка файлы drop входящие демо",
        run: () => {
          navigate("/inbox");
          void bridge.importDroppedFiles([]).then((f) => toast.push({ tone: "info", title: `Добавлено файлов: ${f.length}`, text: "Идёт классификация" }));
        },
      },
      { id: "act:theme", group: "Действия", title: resolvedTheme === "dark" ? "Включить светлую тему" : "Включить тёмную тему", icon: <Moon />, keywords: "тема светлая тёмная dark light", run: () => setTheme(resolvedTheme === "dark" ? "light" : "dark") },
      { id: "act:motion", group: "Действия", title: reduceMotion ? "Включить анимации" : "Уменьшить движение", icon: <Gauge />, keywords: "анимации reduced motion движение", run: () => setMotion(reduceMotion ? "full" : "reduced") },
      { id: "act:claude", group: "Действия", title: "Подключить Claude", icon: <Plug />, keywords: "claude подключить вход", run: () => void bridge.connectProvider("claude") },
      { id: "act:codex", group: "Действия", title: "Подключить Codex", icon: <Plug />, keywords: "codex подключить вход", run: () => void bridge.connectProvider("codex") },
      ...subjects.map((s) => ({ id: `subj:${s.id}`, group: "Предметы", title: s.name, hint: `неделя ${s.currentWeek}`, icon: <CalendarRange />, keywords: `${s.shortName} неделя`, run: go(weekPath(s.id, s.currentWeek)) })),
      ...homework.map((h) => ({
        id: `hw:${h.id}`,
        group: "Домашние задания",
        title: h.title,
        hint: `${subjects.find((s) => s.id === h.subjectId)?.shortName ?? ""} · нед. ${h.weekNumber}`,
        icon: <FileCheck2 />,
        keywords: `дз ${subjects.find((s) => s.id === h.subjectId)?.name ?? ""}`,
        run: go(`/homework/${h.id}`),
      })),
      ...homework.slice(0, 3).map((h) => ({ id: `ai:${h.id}`, group: "AI", title: `Подготовить пакет: ${h.title}`, icon: <BrainCircuit />, keywords: "ai пакет codex claude промпт", run: go(`/ai?homework=${h.id}`) })),
    ];
    return list;
  }, [navigate, undo, toast, resolvedTheme, setTheme, reduceMotion, setMotion, subjects, homework]);

  const filtered = useMemo(() => {
    const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
    if (words.length === 0) return commands.filter((c) => c.group !== "Домашние задания" && c.group !== "AI").slice(0, 14);
    return commands.filter((c) => {
      const hay = `${c.title} ${c.keywords} ${c.hint ?? ""}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    });
  }, [commands, query]);

  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index]);

  const runAt = (i: number) => {
    const c = filtered[i];
    if (!c) return;
    onClose();
    c.run();
  };

  let lastGroup = "";
  return (
    <AnimatePresence>
      {open && (
        <>
          <motion.div className="scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.15 }} onClick={onClose} />
          <div className="dialog-wrap">
            <motion.div
              className="palette"
              role="dialog"
              aria-modal="true"
              aria-label="Команды"
              initial={{ opacity: 0, y: -8, scale: 0.985 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -4, scale: 0.99, transition: { duration: 0.12 } }}
              onKeyDown={(e) => {
                if (e.key === "Escape") onClose();
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setIndex((i) => Math.min(filtered.length - 1, i + 1));
                }
                if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setIndex((i) => Math.max(0, i - 1));
                }
                if (e.key === "Enter") {
                  e.preventDefault();
                  runAt(index);
                }
              }}
            >
              <div className="palette__search">
                <Search />
                <input
                  ref={inputRef}
                  className="palette__input"
                  placeholder="Перейти, найти ДЗ или выполнить действие…"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  role="combobox"
                  aria-expanded="true"
                  aria-controls="palette-list"
                  aria-activedescendant={filtered[index] ? `cmd-${filtered[index]!.id}` : undefined}
                />
                <span className="kbd">esc</span>
              </div>
              <ul id="palette-list" ref={listRef} className="palette__list scroll" role="listbox">
                {filtered.length === 0 && <li className="palette__empty">Ничего не найдено</li>}
                {filtered.map((c, i) => {
                  const header = c.group !== lastGroup ? c.group : null;
                  lastGroup = c.group;
                  return (
                    <li key={c.id} role="presentation">
                      {header && <div className="palette__group eyebrow">{header}</div>}
                      <div
                        id={`cmd-${c.id}`}
                        role="option"
                        aria-selected={i === index}
                        data-index={i}
                        className="palette__item"
                        onMouseMove={() => setIndex(i)}
                        onClick={() => runAt(i)}
                      >
                        {i === index && <motion.span layoutId="palette-hl" className="palette__hl" transition={{ type: "spring", stiffness: 700, damping: 45 }} />}
                        <span className="palette__icon">{c.icon}</span>
                        <span className="palette__title">{c.title}</span>
                        {c.hint && <span className="palette__hint">{c.hint}</span>}
                        {i === index && <CornerDownLeft className="palette__enter" />}
                      </div>
                    </li>
                  );
                })}
              </ul>
            </motion.div>
          </div>
        </>
      )}
    </AnimatePresence>
  );
}
