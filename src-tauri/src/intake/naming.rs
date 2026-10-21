//! Безопасные имена файлов во «Входящих» и определение типа по расширению.

use std::ffi::OsStr;

use crate::{dto::FileKind, error::AppError};

/// Максимальная длина имени во «Входящих» в байтах UTF-8 (APFS/ext4 допускают 255).
/// Запас нужен для суффикса « (N)» при разрешении конфликта имён.
pub const MAX_NAME_BYTES: usize = 200;
const MAX_EXTENSION_BYTES: usize = 16;
const FALLBACK_STEM: &str = "Файл";

/// Превращает имя внешнего файла в безопасный компонент пути во «Входящих».
///
/// - разделители `/`, `\`, `:` и управляющие символы заменяются на `-`;
/// - bidi-override и zero-width символы удаляются (защита от подмены расширения);
/// - ведущие точки убираются (скрытые файлы и зарезервированный префикс `.kontur-`);
/// - пробелы и точки по краям обрезаются;
/// - длина ограничивается [`MAX_NAME_BYTES`] с сохранением расширения;
/// - кириллица и остальной Unicode сохраняются.
pub fn safe_inbox_name(raw: &OsStr) -> Result<String, AppError> {
    let lossy = raw.to_string_lossy();
    let mut cleaned = String::with_capacity(lossy.len());
    for character in lossy.chars() {
        if is_invisible_formatting(character) {
            continue;
        }
        if character.is_control() || matches!(character, '/' | '\\' | ':' | '\u{FFFD}') {
            cleaned.push('-');
        } else {
            cleaned.push(character);
        }
    }
    let trimmed = cleaned
        .trim_matches(|character: char| character.is_whitespace() || character == '.')
        .to_owned();
    let (stem, extension) = split_extension(&trimmed);
    let stem = stem.trim_start_matches('.').trim();
    let stem = if stem.is_empty() { FALLBACK_STEM } else { stem };
    Ok(compose(stem, extension, ""))
}

/// Имя с номером для разрешения конфликта: `Лекция (2).pdf`, `Лекция (3).pdf`…
pub fn numbered_name(name: &str, index: u32) -> String {
    if index <= 1 {
        return name.to_owned();
    }
    let (stem, extension) = split_extension(name);
    let stem = if stem.is_empty() { FALLBACK_STEM } else { stem };
    compose(stem, extension, &format!(" ({index})"))
}

/// Безопасное отображаемое имя исходного файла для отчёта (без пути).
pub fn display_name(raw: Option<&OsStr>) -> String {
    raw.and_then(|value| safe_inbox_name(value).ok())
        .unwrap_or_else(|| FALLBACK_STEM.to_owned())
}

fn compose(stem: &str, extension: Option<&str>, suffix: &str) -> String {
    let extension_bytes = extension.map_or(0, |value| value.len() + 1);
    let max_stem = MAX_NAME_BYTES.saturating_sub(extension_bytes + suffix.len());
    let stem = truncate_utf8(stem, max_stem).trim_end();
    let stem = if stem.is_empty() { FALLBACK_STEM } else { stem };
    match extension {
        Some(extension) => format!("{stem}{suffix}.{extension}"),
        None => format!("{stem}{suffix}"),
    }
}

fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(index) if index > 0 => {
            let extension = &name[index + 1..];
            if !extension.is_empty()
                && extension.len() <= MAX_EXTENSION_BYTES
                && !extension.contains(char::is_whitespace)
            {
                (&name[..index], Some(extension))
            } else {
                (name, None)
            }
        }
        _ => (name, None),
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn is_invisible_formatting(character: char) -> bool {
    matches!(
        character,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// Расширения, которые macOS может исполнить при «открытии» (Terminal, установщик,
/// Automator, Java и т. п.). Такие файлы можно показать в Finder, но «Контур» их не открывает.
pub fn is_risky_to_open(name: &str) -> bool {
    let extension = split_extension(name)
        .1
        .map(str::to_lowercase)
        .unwrap_or_default();
    matches!(
        extension.as_str(),
        "app"
            | "command"
            | "tool"
            | "sh"
            | "zsh"
            | "bash"
            | "csh"
            | "terminal"
            | "pkg"
            | "mpkg"
            | "dmg"
            | "scpt"
            | "applescript"
            | "workflow"
            | "action"
            | "jar"
            | "py"
            | "rb"
            | "pl"
            | "js"
            | "jxa"
            | "webloc"
            | "inetloc"
            | "fileloc"
            | "url"
            | "exe"
            | "msi"
            | "bat"
    )
}

/// Тип файла и MIME по расширению. Содержимое не интерпретируется и не исполняется.
pub fn detect_kind(name: &str) -> (FileKind, &'static str) {
    let extension = split_extension(name)
        .1
        .map(str::to_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => (FileKind::Pdf, "application/pdf"),
        "doc" => (FileKind::Doc, "application/msword"),
        "docx" => (
            FileKind::Doc,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "odt" => (FileKind::Doc, "application/vnd.oasis.opendocument.text"),
        "rtf" => (FileKind::Doc, "application/rtf"),
        "pages" => (FileKind::Doc, "application/vnd.apple.pages"),
        "xls" => (FileKind::Sheet, "application/vnd.ms-excel"),
        "xlsx" => (
            FileKind::Sheet,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        "csv" => (FileKind::Sheet, "text/csv"),
        "ods" => (
            FileKind::Sheet,
            "application/vnd.oasis.opendocument.spreadsheet",
        ),
        "numbers" => (FileKind::Sheet, "application/vnd.apple.numbers"),
        "ppt" => (FileKind::Slides, "application/vnd.ms-powerpoint"),
        "pptx" => (
            FileKind::Slides,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        "key" => (FileKind::Slides, "application/vnd.apple.keynote"),
        "odp" => (
            FileKind::Slides,
            "application/vnd.oasis.opendocument.presentation",
        ),
        "png" => (FileKind::Image, "image/png"),
        "jpg" | "jpeg" => (FileKind::Image, "image/jpeg"),
        "gif" => (FileKind::Image, "image/gif"),
        "heic" => (FileKind::Image, "image/heic"),
        "webp" => (FileKind::Image, "image/webp"),
        "svg" => (FileKind::Image, "image/svg+xml"),
        "ipynb" => (FileKind::Notebook, "application/x-ipynb+json"),
        "txt" => (FileKind::Text, "text/plain"),
        "md" => (FileKind::Text, "text/markdown"),
        "tex" => (FileKind::Text, "application/x-tex"),
        "json" => (FileKind::Text, "application/json"),
        "py" => (FileKind::Text, "text/x-python"),
        "r" => (FileKind::Text, "text/x-r"),
        "zip" => (FileKind::Archive, "application/zip"),
        "rar" => (FileKind::Archive, "application/vnd.rar"),
        "7z" => (FileKind::Archive, "application/x-7z-compressed"),
        "tar" => (FileKind::Archive, "application/x-tar"),
        "gz" => (FileKind::Archive, "application/gzip"),
        _ => (FileKind::Other, "application/octet-stream"),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{detect_kind, is_risky_to_open, numbered_name, safe_inbox_name, MAX_NAME_BYTES};
    use crate::dto::FileKind;

    fn safe(value: &str) -> String {
        safe_inbox_name(OsStr::new(value)).unwrap()
    }

    #[test]
    fn keeps_cyrillic_and_extension() {
        assert_eq!(safe("Лекция 7 — Регрессия.pdf"), "Лекция 7 — Регрессия.pdf");
    }

    #[test]
    fn replaces_separators_controls_and_drops_bidi_overrides() {
        assert_eq!(safe("a:b\\c\u{0007}.txt"), "a-b-c-.txt");
        assert_eq!(safe("счёт\u{202E}fdp.exe"), "счётfdp.exe");
    }

    #[test]
    fn hidden_and_reserved_prefixes_are_removed() {
        assert_eq!(safe(".kontur-1234.tmp"), "kontur-1234.tmp");
        assert_eq!(safe("..."), "Файл");
        assert_eq!(safe("   "), "Файл");
        assert_eq!(safe(".pdf"), "pdf");
    }

    #[test]
    fn long_unicode_names_are_truncated_on_char_boundary_keeping_extension() {
        let long = format!("{}.xlsx", "Ж".repeat(120));
        let result = safe(&long);
        assert!(result.len() <= MAX_NAME_BYTES);
        assert!(result.ends_with(".xlsx"));
        let numbered = numbered_name(&result, 12);
        assert!(numbered.len() <= MAX_NAME_BYTES);
        assert!(numbered.ends_with(" (12).xlsx"));
    }

    #[test]
    fn numbering_preserves_extension_and_handles_names_without_one() {
        assert_eq!(numbered_name("Лекция.pdf", 2), "Лекция (2).pdf");
        assert_eq!(numbered_name("README", 3), "README (3)");
        assert_eq!(numbered_name("Лекция.pdf", 1), "Лекция.pdf");
    }

    #[test]
    fn executable_and_script_files_are_not_opened() {
        for name in [
            "install.command",
            "run.SH",
            "Setup.pkg",
            "x.webloc",
            "solve.py",
        ] {
            assert!(is_risky_to_open(name), "{name}");
        }
        for name in ["Лекция.pdf", "data.xlsx", "notes.md", "hw.ipynb"] {
            assert!(!is_risky_to_open(name), "{name}");
        }
    }

    #[test]
    fn detects_kind_and_mime_by_extension_case_insensitively() {
        assert_eq!(
            detect_kind("Задание.PDF"),
            (FileKind::Pdf, "application/pdf")
        );
        assert_eq!(detect_kind("data.csv").0, FileKind::Sheet);
        assert_eq!(detect_kind("hw.ipynb").0, FileKind::Notebook);
        assert_eq!(detect_kind("archive.tar.gz").0, FileKind::Archive);
        assert_eq!(
            detect_kind("без расширения"),
            (FileKind::Other, "application/octet-stream")
        );
    }
}
