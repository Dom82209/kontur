import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { MockDesktopBridge } from "../../lib/bridge/mock/MockDesktopBridge";
import { AdoptionPreviewBody, allowedCategories, ReportSummary } from "./VaultAdoptionDialog";

describe("Мастер подключения vault: честное отображение плана", () => {
  it("показывает сводку, неуверенные категории и причины пропуска", async () => {
    const preview = await new MockDesktopBridge().previewVaultAdoption();
    const html = renderToStaticMarkup(createElement(AdoptionPreviewBody, { preview, dryRun: null, overrides: {}, onOverride: () => undefined }));
    expect(html).toContain("Проверьте категорию (1)");
    expect(html).toContain("week1.xlsx");
    expect(html).toContain("Общие материалы предмета (без недели)");
    expect(html).toContain("подключаются как «не указаны»");
    expect(html).not.toContain("Проверка без записи пройдена");
  });

  it("категории ДЗ предлагаются только файлам из «03 ДЗ»", async () => {
    const preview = await new MockDesktopBridge().previewVaultAdoption();
    const lecture = preview.plan.files.find((file) => file.category === "lecture")!;
    const homework = preview.plan.files.find((file) => file.homeworkId)!;
    expect(allowedCategories(lecture)).toEqual(["lecture", "seminar", "material"]);
    expect(allowedCategories(homework)).toContain("homework_data");
  });

  it("отчёт dry-run отличает «будет создано» от «создано»", async () => {
    const bridge = new MockDesktopBridge();
    const preview = await bridge.previewVaultAdoption();
    const dry = await bridge.applyVaultAdoption(preview.id, [], true);
    expect(renderToStaticMarkup(createElement(ReportSummary, { report: dry }))).toContain("Будет создано");
    const done = await bridge.applyVaultAdoption(preview.id, [], false);
    expect(renderToStaticMarkup(createElement(ReportSummary, { report: done }))).toContain("Создано");
  });
});
