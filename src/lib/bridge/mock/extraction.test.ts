import { describe, expect, it } from "vitest";
import { newOperationId } from "../intake";
import type { MaterialExtractionItem } from "../types";
import { MockDesktopBridge } from "./MockDesktopBridge";

const pick = (items: MaterialExtractionItem[], kind: string) => items.find((i) => i.fileKind === kind)!;

describe("mock: извлечение текста этапа 7", () => {
  it("извлекает, переиспользует cache по содержимому и перестраивает по force", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const items = await bridge.listMaterialExtractions();
    expect(items.length).toBeGreaterThan(0);
    expect(items.some((i) => i.material.kind === "inbox")).toBe(true);
    expect(items.some((i) => i.material.kind === "file")).toBe(true);
    const pdf = pick(items, "pdf");
    await expect(bridge.getMaterialExtraction(pdf.material)).resolves.toBeNull();

    const first = await bridge.analyzeMaterial(pdf.material);
    expect(first.status).toBe("complete");
    expect(first.cached).toBe(false);
    expect(first.blocks.some((b) => b.type === "page")).toBe(true);
    const second = await bridge.analyzeMaterial(pdf.material);
    expect(second.cached).toBe(true);
    const rebuilt = await bridge.analyzeMaterial(pdf.material, { force: true });
    expect(rebuilt.cached).toBe(false);

    const listed = (await bridge.listMaterialExtractions()).find((i) => i.material.id === pdf.material.id)!;
    expect(listed.extraction?.status).toBe("complete");
  });

  it("пакет сообщает прогресс, его можно остановить, готовые результаты сохраняются", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const items = await bridge.listMaterialExtractions();
    const refs = items.slice(0, 4).map((i) => i.material);
    const operationId = newOperationId();
    const progress: number[] = [];
    bridge.on("extraction:progress", (p) => {
      if (p.operationId !== operationId) return;
      progress.push(p.completed);
      if (p.completed === 1) void bridge.cancelExtraction(operationId);
    });
    const failed: string[] = [];
    bridge.on("operation:failed", ({ error }) => failed.push(error.code));
    const result = await bridge.analyzeMaterials(refs, { operationId });
    expect(progress[0]).toBe(0);
    expect(result.value.stoppedByUser).toBe(true);
    expect(result.value.cancelled).toBeGreaterThan(0);
    expect(result.operation.kind).toBe("extract");
    expect(result.operation.status).toBe("failed");
    expect(failed).toContain("cancelled");
    await expect(bridge.getMaterialExtraction(refs[0]!)).resolves.not.toBeNull();

    const replay = await bridge.analyzeMaterials(refs, { operationId });
    expect(replay.value.replayed).toBe(true);
    await expect(bridge.cancelExtraction(operationId)).resolves.toBe(false);
  });

  it("изображение без OCR получает needs_ocr, а не ошибку; архив — unsupported", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const items = await bridge.listMaterialExtractions();
    const image = items.find((i) => i.fileKind === "image");
    if (image) {
      const result = await bridge.analyzeMaterial(image.material);
      expect(result.status).toBe("needs_ocr");
      expect(result.metadata.imageWidth).toBeGreaterThan(0);
      expect(result.warnings.map((w) => w.code)).toContain("ocr_disabled");
    }
    const archive = items.find((i) => i.fileKind === "archive");
    if (archive) await expect(bridge.analyzeMaterial(archive.material)).resolves.toMatchObject({ status: "unsupported" });
    const settings = await bridge.getExtractionSettings();
    expect(settings.ocrEnabled).toBe(false);
    await expect(bridge.setExtractionSettings({ ocrEnabled: true })).resolves.toMatchObject({ ocrEnabled: true });
  });

  it("подборка принимает только материалы с извлечённым текстом, предпросмотр сохраняет порядок", async () => {
    const bridge = new MockDesktopBridge({ latency: 0 });
    const items = await bridge.listMaterialExtractions();
    const pdf = pick(items, "pdf");
    const doc = items.find((i) => i.fileKind === "doc") ?? items.find((i) => i.fileKind === "sheet")!;
    await expect(bridge.setCorpusSelection(null, [pdf.material])).rejects.toMatchObject({ code: "blocked" });
    await bridge.analyzeMaterial(pdf.material);
    await bridge.analyzeMaterial(doc.material);
    const selection = await bridge.setCorpusSelection(null, [doc.material, pdf.material, doc.material]);
    expect(selection.items).toHaveLength(2);
    expect(selection.items.every((i) => i.ready)).toBe(true);
    const preview = await bridge.previewCorpus(null);
    expect(preview.documents.map((d) => d.name)).toEqual([doc.name, pdf.name]);
    expect(preview.totalChars).toBeGreaterThan(0);

    const homework = await bridge.getHomeworkList();
    const submitted = homework.find((h) => h.stage === "submitted");
    if (submitted) await expect(bridge.setCorpusSelection(submitted.id, [])).rejects.toMatchObject({ code: "immutable" });
    await expect(bridge.getCorpusSelection("missing")).rejects.toMatchObject({ code: "not_found" });

    await expect(bridge.clearExtractionCache()).resolves.toMatchObject({ removed: 2 });
    const after = await bridge.getCorpusSelection(null);
    expect(after.items.every((i) => !i.ready)).toBe(true);
    const excluded = await bridge.previewCorpus(null);
    expect(excluded.documents).toHaveLength(0);
    expect(excluded.excluded).toHaveLength(2);
  });
});
