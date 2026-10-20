/**
 * Подстановка шаблона имени. Используется для ПРЕДПРОСМОТРА во frontend и в mock.
 * Настоящее имя файла всегда определяет backend и возвращает в ответе операции.
 */
export interface NameTokens {
  lastName: string;
  firstName: string;
  week: number;
  subject: string;
  version?: string;
}

export const NAME_TOKENS = ["{Фамилия}", "{Имя}", "{Неделя}", "{Предмет}", "{Версия}"] as const;

export function applyNamePattern(pattern: string, t: NameTokens, ext?: string): string {
  const base = pattern
    .replaceAll("{Фамилия}", t.lastName)
    .replaceAll("{Имя}", t.firstName)
    .replaceAll("{Неделя}", String(t.week))
    .replaceAll("{Предмет}", t.subject)
    .replaceAll("{Версия}", t.version ?? "");
  return ext ? `${base}.${ext}` : base;
}
