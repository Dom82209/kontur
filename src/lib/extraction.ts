import type { ExtractionFormat, ExtractionItemOutcome, ExtractionStatus, MaterialRef } from "./bridge";

export const EXTRACTION_STATUS: Record<ExtractionStatus, { label: string; tone: string; hint: string }> = {
  complete: { label: "Извлечено", tone: "badge--ok", hint: "Всё поддерживаемое содержимое извлечено" },
  partial: { label: "Частично", tone: "badge--warn", hint: "Часть содержимого пропущена или обрезана — см. предупреждения" },
  needs_ocr: { label: "Нужен OCR", tone: "badge--info", hint: "Текстового слоя нет: скан или изображение. Метаданные сохранены" },
  unsupported: { label: "Не поддерживается", tone: "", hint: "Формат не поддерживается для извлечения текста" },
  encrypted: { label: "Защищён паролем", tone: "badge--warn", hint: "Файл зашифрован — текст не извлекается" },
  corrupt: { label: "Повреждён", tone: "badge--danger", hint: "Файл не удалось разобрать" },
  rejected: { label: "Отклонён", tone: "badge--danger", hint: "Файл отклонён по соображениям безопасности" },
  too_large: { label: "Слишком большой", tone: "badge--warn", hint: "Файл превышает лимит размера" },
  timeout: { label: "Время вышло", tone: "badge--warn", hint: "Разбор не уложился в лимит времени — можно повторить" },
  failed: { label: "Ошибка", tone: "badge--danger", hint: "Разбор прервался — можно повторить" },
};

export const EXTRACTION_FORMAT: Record<ExtractionFormat, string> = {
  pdf: "PDF",
  docx: "DOCX",
  xlsx: "XLSX",
  pptx: "PPTX",
  text: "Текст",
  markdown: "Markdown",
  csv: "CSV",
  tsv: "TSV",
  notebook: "Jupyter",
  image: "Изображение",
  unknown: "Неизвестный",
};

export const BATCH_OUTCOME: Record<ExtractionItemOutcome, string> = {
  extracted: "извлечено",
  cached: "из cache",
  failed: "ошибка",
  cancelled: "остановлено",
};

/** Пригоден для подборки: есть нормализованный текст. */
export const usableForCorpus = (status: ExtractionStatus | null | undefined): boolean => status === "complete" || status === "partial";

export const materialKey = (material: MaterialRef): string => `${material.kind}:${material.id}`;

export const sameMaterial = (a: MaterialRef, b: MaterialRef): boolean => a.kind === b.kind && a.id === b.id;

export function formatChars(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1).replace(".", ",")} млн симв.`;
  if (count >= 1_000) return `${Math.round(count / 1_000)} тыс. симв.`;
  return `${count} симв.`;
}
