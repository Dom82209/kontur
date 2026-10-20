import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import { useEffect, useState } from "react";
import { useLocation, useNavigate, useOutlet } from "react-router-dom";
import { NAV } from "../../app/nav";
import { useOps } from "../../app/operations";
import { useHotkey } from "../../lib/hooks/useHotkey";
import { CommandPalette } from "./CommandPalette";
import { IntroSequence } from "./IntroSequence";
import { Sidebar } from "./Sidebar";
import { TitleBar } from "./TitleBar";

/** Каркас окна: заголовок с учётом traffic lights, навигация, контент со связными переходами. */
export function AppShell() {
  const [palette, setPalette] = useState(false);
  const location = useLocation();
  const outlet = useOutlet();
  const navigate = useNavigate();
  const { undo } = useOps();

  useHotkey("KeyK", () => setPalette((v) => !v), { meta: true, allowInInput: true });
  useHotkey("KeyZ", () => void undo(), { meta: true });
  useEffect(() => {
    // ⌘1…⌘6 и ⌘, — переходы по разделам
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey) return;
      const item = NAV.find((n) => n.shortcut === e.key);
      if (!item) return;
      e.preventDefault();
      navigate(item.to);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate]);

  // Ключ перехода — верхний сегмент пути, чтобы смена недели внутри предмета не перерисовывала весь экран.
  const key = location.pathname.split("/").slice(0, 3).join("/");

  return (
    <LayoutGroup>
      <div className="shell">
        <TitleBar onOpenPalette={() => setPalette(true)} />
        <Sidebar />
        <main className="content scroll" id="main">
          <AnimatePresence mode="wait" initial={false}>
            <motion.div
              key={key}
              className="page"
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0, transition: { duration: 0.24, ease: [0.2, 0.8, 0.2, 1] } }}
              exit={{ opacity: 0, y: -4, transition: { duration: 0.14, ease: [0.6, 0, 1, 1] } }}
            >
              {outlet}
            </motion.div>
          </AnimatePresence>
        </main>
        <CommandPalette open={palette} onClose={() => setPalette(false)} />
        <IntroSequence />
      </div>
    </LayoutGroup>
  );
}
