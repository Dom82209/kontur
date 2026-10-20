import { AnimatePresence, motion } from "motion/react";

export function SaveBar({ visible, saving, onSave, onReset }: { visible: boolean; saving: boolean; onSave: () => void; onReset: () => void }) {
  return (
    <AnimatePresence>
      {visible && (
        <motion.div className="savebar" role="region" aria-label="Несохранённые изменения" initial={{ opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 12 }}>
          <span className="dot dot--accent" />
          <span>Есть несохранённые изменения</span>
          <button className="btn btn--sm btn--ghost" onClick={onReset} disabled={saving}>
            Сбросить
          </button>
          <button className="btn btn--sm btn--primary" onClick={onSave} disabled={saving}>
            {saving ? "Сохранение…" : "Сохранить"}
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
