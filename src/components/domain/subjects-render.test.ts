import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { MockDesktopBridge } from "../../lib/bridge/mock/MockDesktopBridge";
import { MergePreviewBody, parseKeywords } from "./SubjectsPanel";

describe("Панель предметов", () => {
  it("разбирает ключевые слова через запятую, точку с запятой и перенос строки", () => {
    expect(parseKeywords(" стат, вероятност ;\nтвимс,, ")).toEqual(["стат", "вероятност", "твимс"]);
  });

  it("предпросмотр слияния честно показывает блокирующий конфликт недель", async () => {
    const preview = await new MockDesktopBridge().previewSubjectMerge("matan-adv", "matan");
    const html = renderToStaticMarkup(createElement(MergePreviewBody, { preview }));
    expect(html).toContain("Файлы в vault не перемещаются");
    expect(html).toContain("Слияние таких недель не поддерживается");
  });
});
