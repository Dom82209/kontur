import { motion } from "motion/react";
import { FolderInput, Upload } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { bridge, type DroppedItem } from "../../lib/bridge";

interface Props {
  onFiles: (items: DroppedItem[]) => void;
  onDemo: () => void;
  onError?: (error: unknown) => void;
  busy?: boolean;
}

const REACH = 420; // px — с какого расстояния зона начинает «притягивать»

/**
 * Drag-and-drop зона с магнитным свечением: пока файл тащат по окну,
 * свечение усиливается по мере приближения и смещается к курсору.
 * В Tauri drop приходит событием окна с настоящими путями (`onNativeFileDrop`),
 * а «Выбрать файлы» открывает системный диалог через backend.
 */
export function DropZone({ onFiles, onDemo, onError, busy }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const [over, setOver] = useState(false);
  const raf = useRef(0);
  const depth = useRef(0);

  // Native drag-and-drop: backend передаёт настоящие sourcePath. Подписка не
  // пересоздаётся при каждом рендере — обработчик берётся из ref.
  const onFilesRef = useRef(onFiles);
  onFilesRef.current = onFiles;
  useEffect(() => {
    if (bridge.kind !== "tauri") return;
    const offDrop = bridge.onNativeFileDrop((items) => onFilesRef.current(items));
    const offDrag = bridge.onNativeDragState((active) => {
      setDragging(active);
      setOver(active);
    });
    return () => {
      offDrop();
      offDrag();
    };
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const hasFiles = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].includes("Files");

    const update = (x: number, y: number) => {
      cancelAnimationFrame(raf.current);
      raf.current = requestAnimationFrame(() => {
        const r = el.getBoundingClientRect();
        const cx = Math.min(Math.max(x, r.left), r.right);
        const cy = Math.min(Math.max(y, r.top), r.bottom);
        const dist = Math.hypot(x - cx, y - cy);
        const pull = Math.max(0, 1 - dist / REACH);
        el.style.setProperty("--glow-x", `${((cx - r.left) / r.width) * 100}%`);
        el.style.setProperty("--glow-y", `${((cy - r.top) / r.height) * 100}%`);
        el.style.setProperty("--pull", pull.toFixed(3));
      });
    };
    const onEnter = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      depth.current += 1;
      setDragging(true);
    };
    const onOver = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      update(e.clientX, e.clientY);
    };
    const reset = () => {
      depth.current = 0;
      setDragging(false);
      setOver(false);
      el.style.setProperty("--pull", "0");
    };
    const onLeave = () => {
      depth.current = Math.max(0, depth.current - 1);
      if (depth.current === 0) reset();
    };
    const onDropWindow = (e: DragEvent) => {
      if (hasFiles(e)) e.preventDefault();
      reset();
    };
    window.addEventListener("dragenter", onEnter);
    window.addEventListener("dragover", onOver);
    window.addEventListener("dragleave", onLeave);
    window.addEventListener("drop", onDropWindow);
    return () => {
      cancelAnimationFrame(raf.current);
      window.removeEventListener("dragenter", onEnter);
      window.removeEventListener("dragover", onOver);
      window.removeEventListener("dragleave", onLeave);
      window.removeEventListener("drop", onDropWindow);
    };
  }, []);

  const take = (list: FileList | null) => {
    if (!list || list.length === 0) return;
    onFiles([...list].map((f) => ({ name: f.name, sizeBytes: f.size })));
  };

  return (
    <motion.div
      ref={ref}
      className="dropzone"
      data-dragging={dragging}
      data-over={over}
      onDragOver={(e) => {
        e.preventDefault();
        if (!over) setOver(true);
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOver(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        take(e.dataTransfer.files);
      }}
      animate={{ scale: over ? 1.006 : 1 }}
      transition={{ type: "spring", stiffness: 400, damping: 30 }}
    >
      <div className="dropzone__glow" aria-hidden />
      <div className="dropzone__grid" aria-hidden />
      <div className="dropzone__content">
        <motion.div className="dropzone__icon" animate={{ y: over ? -3 : 0 }}>
          <Upload />
        </motion.div>
        <div>
          <div className="dropzone__title">{over ? "Отпустите — файлы попадут во «Входящие»" : "Перетащите лекции, семинары и материалы ДЗ"}</div>
          <div className="dropzone__hint">Предмет, неделя и тип определятся автоматически. Ничего не перезаписывается.</div>
        </div>
        <div className="dropzone__actions">
          <button
            className="btn"
            onClick={() => {
              if (bridge.kind === "mock") input.current?.click();
              else void bridge.pickFiles().then((items) => items.length > 0 && onFiles(items), onError);
            }}
            disabled={busy}
          >
            <FolderInput /> Выбрать файлы
          </button>
          {bridge.kind === "mock" && (
            <button className="btn btn--ghost" onClick={onDemo} disabled={busy}>
              Симулировать загрузку
            </button>
          )}
        </div>
      </div>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        disabled={bridge.kind !== "mock"}
        onChange={(e) => {
          take(e.target.files);
          e.target.value = "";
        }}
      />
    </motion.div>
  );
}
