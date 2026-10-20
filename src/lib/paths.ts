import type { MaterialCategory } from "./bridge/types";
import { pad2 } from "./format";

/** Каноническая структура внутри хранилища Obsidian. Все пути относительные. */
export const STUDY_ROOT = "Предметы";
export const INBOX_FOLDER = `${STUDY_ROOT}/00 Входящие`;

export const CATEGORY_FOLDER: Record<MaterialCategory, string> = {
  lecture: "01 Лекции",
  seminar: "02 Семинары",
  homework_task: "03 ДЗ/01 Условие и данные",
  homework_data: "03 ДЗ/01 Условие и данные",
  homework_draft: "03 ДЗ/02 Работа",
  material: "04 Дополнительно",
  unknown: "00 Разобрать",
};

export function weekFolderForSubject(subjectName: string, weekNumber: number): string {
  return `${STUDY_ROOT}/${subjectName}/Неделя ${pad2(weekNumber)}`;
}

export function homeworkFolderForWeek(weekFolder: string): string {
  return `${weekFolder}/03 ДЗ`;
}
