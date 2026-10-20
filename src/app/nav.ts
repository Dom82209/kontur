import { BrainCircuit, CalendarRange, FileCheck2, Inbox, LayoutDashboard, Settings2, Workflow, type LucideIcon } from "lucide-react";

export interface NavItem {
  to: string;
  label: string;
  icon: LucideIcon;
  /** Подсказка для command palette */
  keywords: string;
  shortcut?: string;
}

/** Единственный источник навигации — используется в боковой панели и в command palette. */
export const NAV: NavItem[] = [
  { to: "/", label: "Главная", icon: LayoutDashboard, keywords: "главная дашборд обзор неделя", shortcut: "1" },
  { to: "/inbox", label: "Входящие", icon: Inbox, keywords: "входящие файлы разобрать скачанные", shortcut: "2" },
  { to: "/week", label: "Неделя", icon: CalendarRange, keywords: "неделя предмет лекции семинары", shortcut: "3" },
  { to: "/homework", label: "Домашние задания", icon: FileCheck2, keywords: "дз домашние задания версии сдача", shortcut: "4" },
  { to: "/ai", label: "AI Workspace", icon: BrainCircuit, keywords: "ai нейросеть codex claude промпт пакет", shortcut: "5" },
  { to: "/automation", label: "Автоматизация", icon: Workflow, keywords: "автоматизация правила журнал операции", shortcut: "6" },
  { to: "/settings", label: "Настройки", icon: Settings2, keywords: "настройки тема анимации obsidian путь", shortcut: "," },
];

export const weekPath = (subjectId: string, n: number): string => `/subjects/${subjectId}/week/${n}`;
