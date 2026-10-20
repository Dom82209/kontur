import { AnimatePresence, motion } from "motion/react";
import { Moon, Search, Sun, Undo2 } from "lucide-react";
import { useLocation } from "react-router-dom";
import { NAV } from "../../app/nav";
import { useOps } from "../../app/operations";
import { usePrefs } from "../../app/preferences";

function useTitle(): string {
  const { pathname } = useLocation();
  if (pathname.startsWith("/subjects/")) return "Неделя";
  if (pathname.startsWith("/homework/")) return "Домашнее задание";
  return NAV.find((n) => n.to === pathname)?.label ?? "Контур";
}

export function TitleBar({ onOpenPalette }: { onOpenPalette: () => void }) {
  const title = useTitle();
  const { running, lastUndoable, undo } = useOps();
  const { resolvedTheme, setTheme } = usePrefs();

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="titlebar__gutter" data-tauri-drag-region />
      <div className="titlebar__title" data-tauri-drag-region>
        <AnimatePresence mode="wait" initial={false}>
          <motion.span key={title} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -4 }} transition={{ duration: 0.16 }}>
            {title}
          </motion.span>
        </AnimatePresence>
      </div>
      <button className="titlebar__search" onClick={onOpenPalette} aria-keyshortcuts="Meta+K">
        <Search />
        <span>Поиск и команды</span>
        <span className="kbd">⌘K</span>
      </button>
      <div className="titlebar__right" data-tauri-drag-region>
        <AnimatePresence>
          {running && (
            <motion.div className="titlebar__op" initial={{ opacity: 0, x: 8 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0 }} role="status" aria-live="polite">
              <span className="titlebar__op-text">{running.message}</span>
              <span className="meter" style={{ width: 64 }}>
                <motion.span className="meter__fill" initial={false} animate={{ scaleX: running.progress }} transition={{ type: "spring", stiffness: 200, damping: 30 }} />
              </span>
            </motion.div>
          )}
        </AnimatePresence>
        {lastUndoable && (
          <button className="btn btn--sm btn--ghost" onClick={() => void undo(lastUndoable.id)} title={`Отменить: ${lastUndoable.title} (⌘Z)`}>
            <Undo2 /> Отменить
          </button>
        )}
        <button className="btn btn--sm btn--ghost btn--icon" aria-label={resolvedTheme === "dark" ? "Светлая тема" : "Тёмная тема"} onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}>
          {resolvedTheme === "dark" ? <Sun /> : <Moon />}
        </button>
      </div>
    </header>
  );
}
