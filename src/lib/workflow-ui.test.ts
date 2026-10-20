import { describe, expect, it } from "vitest";
import { getWorkflowPresentation } from "./workflow-ui";

describe("представление этапов workflow", () => {
  it("определяет ближайшее действие и логичную вкладку для каждого этапа", () => {
    expect(getWorkflowPresentation("materials_collected")).toEqual({ tab: "materials", action: "Проверить материалы и запустить анализ" });
    expect(getWorkflowPresentation("prompt_ready")).toEqual({ tab: "cycle", action: "Передать промт автору" });
    expect(getWorkflowPresentation("needs_fixes")).toEqual({ tab: "review", action: "Подготовить промт исправлений" });
    expect(getWorkflowPresentation("teacher_review")).toEqual({ tab: "teacher", action: "Сверить комментарии преподавателя" });
    expect(getWorkflowPresentation("ready_to_submit")).toEqual({ tab: "package", action: "Отправить вручную и подтвердить" });
    expect(getWorkflowPresentation("submitted")).toEqual({ tab: "package", action: "Открыть неизменяемый снимок" });
  });

  it("ведёт к комплекту, когда teacher review уже дал готовность", () => {
    expect(getWorkflowPresentation("teacher_review", true)).toEqual({ tab: "package", action: "Собрать комплект на сдачу" });
  });
});
