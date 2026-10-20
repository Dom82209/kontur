import { motion } from "motion/react";
import type { FileKind } from "../../lib/bridge";
import { FileIcon } from "../ui/FileIcon";

interface Props {
  id: string;
  name: string;
  kind: FileKind;
  sub?: string;
}

/** Компактная плашка файла. Общий layoutId позволяет ей «перелетать» между списками. */
export function FileChip({ id, name, kind, sub }: Props) {
  return (
    <motion.div layoutId={`file-${id}`} className="chip" transition={{ type: "spring", stiffness: 340, damping: 32 }}>
      <FileIcon kind={kind} />
      <span className="chip__text">
        <span className="chip__name">{name}</span>
        {sub && <span className="chip__sub">{sub}</span>}
      </span>
    </motion.div>
  );
}
