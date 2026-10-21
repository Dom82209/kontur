/**
 * Подсказка для UI: похожа ли ссылка на страницу LMS Центрального университета.
 * Окончательную проверку allowlist выполняет backend при сохранении и перед
 * открытием во внешнем браузере; frontend ссылки сам не открывает.
 */
export const LMS_HOSTS = ["my.centraluniversity.ru"] as const;

export function lmsUrlProblem(raw: string): string | null {
  const value = raw.trim();
  if (!value) return null;
  if (/\s/.test(value)) return "Ссылка не должна содержать пробелы";
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return "Это не ссылка";
  }
  if (url.protocol !== "https:") return "Нужна ссылка https://";
  if (url.username || url.password) return "Ссылка не должна содержать логин или пароль";
  if (url.port) return "Ссылка не должна указывать порт";
  if (!(LMS_HOSTS as readonly string[]).includes(url.hostname.toLowerCase())) return `Разрешены только ссылки на ${LMS_HOSTS.join(", ")}`;
  return null;
}
