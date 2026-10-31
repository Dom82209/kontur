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

/**
 * Те же правила, что у backend (`naming::validate_pattern`): известные токены,
 * обязательный {Неделя}, без разделителей пути. Возвращает текст ошибки или null.
 */
export function validateNamePattern(pattern: string): string | null {
  if (!pattern.trim()) return "Шаблон итогового имени пуст";
  if ([...pattern].length > 120) return "Шаблон итогового имени длиннее 120 символов";
  if (/[/\\:\u0000-\u001f\u007f]/.test(pattern)) return "В шаблоне итогового имени нельзя использовать «/», «\\», «:» и управляющие символы";
  if (!pattern.includes("{Неделя}")) return "В шаблоне итогового имени нужен {Неделя}: иначе имена разных недель совпадут";
  if (pattern.startsWith(".")) return "Шаблон итогового имени некорректен";
  const rest = pattern.replace(/\{[^{}]*\}/g, (token) => ((NAME_TOKENS as readonly string[]).includes(token) ? "" : `\u0000${token}`));
  const unknown = /\u0000(\{[^{}]*\})/.exec(rest);
  if (unknown) return `Неизвестный токен ${unknown[1]}; доступны ${NAME_TOKENS.join(" ")}`;
  if (/[{}]/.test(rest)) return "Шаблон итогового имени некорректен";
  return null;
}
