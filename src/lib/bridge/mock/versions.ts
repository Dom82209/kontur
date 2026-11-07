import type { ContentChange, ContentComparison, HomeworkVersion, VersionLineage, VersionOrigin } from "../types";

/** Этап 10 (демо): происхождение по автору версии. */
export function originFor(author: HomeworkVersion["author"]): VersionOrigin {
  return author === "claude" ? "claude_result" : author === "codex" ? "codex_result" : "user_import";
}

export function lineageFor(author: HomeworkVersion["author"], parentVersionId: string | null, creationReason: string, reviewRunId: string | null = null): VersionLineage {
  return {
    parentVersionId,
    origin: originFor(author),
    createdBy: author,
    creationReason,
    reviewRunId,
    inputsFingerprint: null,
    decision: "pending",
    decisionReason: null,
    decidedAt: null,
    supersededByVersionId: null,
  };
}

/** Версия до этапа 10: родитель неизвестен, источник — по автору. */
export function legacyLineage(author: HomeworkVersion["author"]): VersionLineage {
  return lineageFor(author, null, "Создана до учёта родословной (этап 10): родитель неизвестен");
}

/** Итоговой назначена `version`: прежняя итоговая — «заменена». */
export function recordFinal(versions: HomeworkVersion[], version: HomeworkVersion): void {
  const now = new Date().toISOString();
  for (const v of versions) {
    if (v.id !== version.id && v.lineage.decision === "final") {
      v.lineage = { ...v.lineage, decision: "superseded", decisionReason: "Итоговой назначена другая версия", decidedAt: now, supersededByVersionId: version.id };
    }
  }
  if (version.lineage.decision !== "final") version.lineage = { ...version.lineage, decision: "final", decisionReason: null, decidedAt: now, supersededByVersionId: null };
}

const ext = (name: string) => name.split(".").pop()?.toLowerCase() ?? "";

/**
 * Демо-сравнение: разделы по хешам, файлы по имени. Содержимое файлов в демо
 * не читается — там, где настоящий backend разбирает PDF/Excel, демо честно
 * просит ручную проверку.
 */
export function mockContentComparison(base: HomeworkVersion, target: HomeworkVersion): ContentComparison {
  const changes: ContentChange[] = [];
  const warnings: string[] = [];
  const manualChecks: string[] = [];
  const baseSections = new Map(base.sections.map((s) => [s.title, s.hash]));
  const targetSections = new Map(target.sections.map((s) => [s.title, s.hash]));
  for (const [title, hash] of targetSections) {
    const before = baseSections.get(title);
    if (before === undefined) changes.push({ kind: "added", category: "heading", file: target.fileName, location: null, before: null, after: title, detail: null });
    else if (before !== hash) changes.push({ kind: "modified", category: "text", file: target.fileName, location: title, before: null, after: null, detail: "Содержимое раздела изменилось (демо сравнивает только хеш раздела)" });
  }
  for (const title of baseSections.keys()) if (!targetSections.has(title)) changes.push({ kind: "removed", category: "heading", file: target.fileName, location: null, before: title, after: null, detail: null });
  // Файлы сопоставляются по позиции (основной — первый), как основной файл в backend.
  const count = Math.max(base.files.length, target.files.length);
  for (let i = 0; i < count; i++) {
    const a = base.files[i];
    const b = target.files[i];
    if (a && b) changes.push({ kind: "modified", category: "file", file: b.name, location: null, before: a.relativePath === b.relativePath ? null : a.name, after: a.relativePath === b.relativePath ? null : b.name, detail: `Размер ${a.sizeBytes} → ${b.sizeBytes} байт` });
    else if (b) changes.push({ kind: "added", category: "file", file: b.name, location: null, before: null, after: b.relativePath, detail: null });
    else if (a) changes.push({ kind: "removed", category: "file", file: a.name, location: null, before: a.relativePath, after: null, detail: null });
  }
  for (const f of target.files) {
    const e = ext(f.name);
    if (e === "xlsx") warnings.push("Формулы не пересчитываются: сравниваются сохранённые значения и текст формул");
    if (e === "pdf" || e === "xlsx") manualChecks.push(`Демо-режим не читает файлы: сравните «${f.name}» вручную`);
    else if (!["md", "txt", "docx"].includes(e)) manualChecks.push(`Сравните «${f.name}» вручную: формат не поддерживается для сравнения содержимого`);
  }
  const status = manualChecks.length ? "manual_required" : changes.length ? "complete" : "identical";
  return {
    id: `cmp-${base.id}-${target.id}`,
    homeworkId: target.homeworkId,
    baseVersionId: base.id,
    targetVersionId: target.id,
    baseFingerprint: base.id,
    targetFingerprint: target.id,
    extractorVersion: 1,
    status,
    summary: changes.length ? `Изменения — ${changes.length}${manualChecks.length ? `, требуется ручная проверка: ${manualChecks.length}` : ""}` : "Содержимое версий совпадает",
    changes,
    warnings: [...new Set(warnings)],
    manualChecks: [...new Set(manualChecks)],
    createdAt: new Date().toISOString(),
    stale: false,
  };
}
