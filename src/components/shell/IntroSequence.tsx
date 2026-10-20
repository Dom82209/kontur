import { AnimatePresence, motion } from "motion/react";
import { useEffect } from "react";
import { useIntro } from "../../app/intro";
import { ProcessRail } from "../domain/ProcessRail";

/**
 * Стартовая история: точка → линия учебного процесса → узлы этапов.
 * Затем рельс «приземляется» на главную (shared layoutId "process-rail").
 * Пропускается кликом или любой клавишей; при reduced motion не показывается.
 */
export function IntroSequence() {
  const { done, finish } = useIntro();

  useEffect(() => {
    if (done) return;
    const t = setTimeout(finish, 2100);
    const skip = () => finish();
    window.addEventListener("keydown", skip, { once: true });
    return () => {
      clearTimeout(t);
      window.removeEventListener("keydown", skip);
    };
  }, [done, finish]);

  return (
    <AnimatePresence>
      {!done && (
        <motion.div className="intro" onClick={finish} initial={{ opacity: 1 }} exit={{ opacity: 0, transition: { duration: 0.32, delay: 0.05 } }} aria-hidden>
          <div className="intro__stage">
            <motion.div className="intro__brand" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 1.35, duration: 0.3 }}>
              Контур
            </motion.div>
            <ProcessRail variant="intro" layoutId="process-rail" reached={-1} />
            <motion.div className="intro__skip" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.8 }}>
              Нажмите любую клавишу, чтобы пропустить
            </motion.div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
