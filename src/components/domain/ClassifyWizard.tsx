import { AnimatePresence, motion } from "motion/react";
import { ArrowLeft, ArrowRight, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { bridge, type InboxFile, type RoutePreview } from "../../lib/bridge";
import { formatBytes, plural } from "../../lib/format";
import { Dialog } from "../ui/Dialog";
import { FileIcon } from "../ui/FileIcon";
import { ClassificationFields } from "./ClassificationFields";

interface Props {
  open: boolean;
  files: InboxFile[];
  onClose: () => void;
  onRoute: (ids: string[]) => void;
}

const resolved = (f: InboxFile | undefined) => !!f?.classification?.subjectId && !!f.classification.weekNumber && f.classification.category !== "unknown";

/** Короткий мастер для неоднозначных файлов: один файл — один шаг. */
export function ClassifyWizard({ open, files, onClose, onRoute }: Props) {
  const [queue, setQueue] = useState<string[]>([]);
  const [step, setStep] = useState(0);
  const [dir, setDir] = useState(1);
  const [preview, setPreview] = useState<RoutePreview | null>(null);

  useEffect(() => {
    if (open) {
      setQueue(files.filter((f) => f.status === "ambiguous").map((f) => f.id));
      setStep(0);
    }
    // очередь фиксируется в момент открытия
  }, [open]);

  const done = step >= queue.length;
  const file = files.find((f) => f.id === queue[step]);
  const c = file?.classification;

  useEffect(() => {
    setPreview(null);
    if (!file || !c?.subjectId || !c.weekNumber || c.category === "unknown") return;
    let alive = true;
    void bridge.previewRoute(file.id, { subjectId: c.subjectId, weekNumber: c.weekNumber, category: c.category }).then((p) => alive && setPreview(p));
    return () => {
      alive = false;
    };
  }, [file, c?.subjectId, c?.weekNumber, c?.category]);

  const go = (d: 1 | -1) => {
    setDir(d);
    setStep((s) => Math.max(0, Math.min(queue.length, s + d)));
  };

  const ready = queue.filter((id) => resolved(files.find((f) => f.id === id)));

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={done ? "Готово к раскладке" : "Уточните файл"}
      description={done ? undefined : `Файл ${Math.min(step + 1, queue.length)} из ${queue.length}. Остальные распознаны уверенно.`}
      footer={
        done ? (
          <>
            <button className="btn btn--ghost" onClick={() => go(-1)} disabled={queue.length === 0}>
              <ArrowLeft /> Назад
            </button>
            <button
              className="btn btn--primary"
              disabled={ready.length === 0}
              onClick={() => {
                onRoute(ready);
                onClose();
              }}
            >
              Разложить {ready.length} {plural(ready.length, "файл", "файла", "файлов")}
            </button>
          </>
        ) : (
          <>
            <button className="btn btn--ghost" onClick={() => go(-1)} disabled={step === 0}>
              <ArrowLeft /> Назад
            </button>
            <button className="btn btn--ghost" onClick={() => go(1)}>
              Пропустить
            </button>
            <button className="btn btn--primary" onClick={() => go(1)} disabled={!resolved(file)}>
              Далее <ArrowRight />
            </button>
          </>
        )
      }
    >
      <div className="wizard__dots" aria-hidden>
        {queue.map((id, i) => (
          <span key={id} data-state={i === step ? "current" : resolved(files.find((f) => f.id === id)) ? "done" : "todo"} />
        ))}
      </div>
      <AnimatePresence mode="wait" custom={dir} initial={false}>
        <motion.div key={done ? "done" : queue[step]} custom={dir} initial={{ opacity: 0, x: dir * 18 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: dir * -18 }} transition={{ duration: 0.18 }}>
          {done ? (
            <p className="muted">
              Уточнено {ready.length} из {queue.length}. {queue.length - ready.length > 0 ? "Пропущенные останутся во «Входящих»." : ""}
            </p>
          ) : file ? (
            <div className="wizard">
              <div className="wizard__file">
                <FileIcon kind={file.kind} />
                <div>
                  <div className="row__title">{file.name}</div>
                  <div className="row__sub">{formatBytes(file.sizeBytes)} · {c?.reasons.join(", ")}</div>
                </div>
              </div>
              <ClassificationFields file={file} layout="stacked" />
              {preview && (
                <div className="path mono">
                  {preview.targetPath}
                </div>
              )}
              {preview?.duplicate && (
                <div className="notice notice--warn">
                  <TriangleAlert />
                  <div>
                    Дубль: файл сохранится как <span className="mono">{preview.duplicate.suggestedName}</span>, существующий не перезаписывается.
                  </div>
                </div>
              )}
            </div>
          ) : null}
        </motion.div>
      </AnimatePresence>
    </Dialog>
  );
}
