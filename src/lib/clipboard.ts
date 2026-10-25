/**
 * Копирование текста в буфер обмена — только по нажатию кнопки пользователем.
 * Браузерный Clipboard API; при отказе WebView — выделение текста в поле и
 * `execCommand("copy")`. Ничего, кроме переданного текста, не копируется.
 */
export async function copyText(text: string, fallback?: HTMLTextAreaElement | null): Promise<boolean> {
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // WebView мог запретить Clipboard API — пробуем выделение.
  }
  if (!fallback) return false;
  fallback.focus();
  fallback.select();
  try {
    return document.execCommand("copy");
  } catch {
    return false;
  }
}
