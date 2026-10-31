import { describe, expect, it } from "vitest";
import { applyNamePattern, validateNamePattern } from "./naming";

describe("шаблон итогового имени (те же правила, что naming.rs)", () => {
  it("отклоняет те же шаблоны, что и backend", () => {
    for (const bad of ["", "   ", "{Фамилия}", "{Неделя}/x", "{Неделя} {Группа}", "{Неделя", "Неделя}{Неделя}", ".{Неделя}", "a:{Неделя}"]) {
      expect(validateNamePattern(bad), bad).not.toBeNull();
    }
    expect(validateNamePattern("{Неделя} {Группа}")).toContain("{Группа}");
    expect(validateNamePattern("{Фамилия}_{Имя}_Неделя{Неделя} ДЗ")).toBeNull();
  });

  it("предпросмотр совпадает с примером из naming.rs", () => {
    expect(applyNamePattern("{Фамилия}_{Имя}_Неделя{Неделя} ДЗ {Предмет} {Версия}", { lastName: "Гончарова", firstName: "Доминика", week: 7, subject: "Статистика", version: "v03" }, "pdf")).toBe(
      "Гончарова_Доминика_Неделя7 ДЗ Статистика v03.pdf",
    );
  });
});
