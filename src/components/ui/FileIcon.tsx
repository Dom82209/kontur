import { FileArchive, FileCode2, FileImage, FileSpreadsheet, FileText, File as FileGeneric, Presentation, NotebookPen } from "lucide-react";
import type { FileKind } from "../../lib/bridge";

const MAP: Record<FileKind, typeof FileText> = {
  pdf: FileText,
  doc: FileText,
  sheet: FileSpreadsheet,
  slides: Presentation,
  image: FileImage,
  notebook: NotebookPen,
  text: FileCode2,
  archive: FileArchive,
  other: FileGeneric,
};

export function FileIcon({ kind }: { kind: FileKind }) {
  const Icon = MAP[kind];
  return (
    <span className="file-icon" data-kind={kind} aria-hidden>
      <Icon />
    </span>
  );
}
