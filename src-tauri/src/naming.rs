//! Этап 6: итоговое имя файла в «03 На сдачу» по шаблону из настроек.
//!
//! Подстановка совпадает с `src/lib/naming.ts` (предпросмотр во frontend), но
//! настоящее имя всегда вычисляет backend: шаблон проверяется при сохранении
//! правил, а значения токенов очищаются от разделителей пути.

use crate::error::BackendError;

pub const TOKENS: &[&str] = &["{Фамилия}", "{Имя}", "{Неделя}", "{Предмет}", "{Версия}"];
const UNSET_NAME: &str = "Не задано";
const MAX_PATTERN_CHARS: usize = 120;
const MAX_NAME_CHARS: usize = 60;

pub struct NameTokens<'a> {
    pub last_name: &'a str,
    pub first_name: &'a str,
    pub week: i64,
    pub subject: &'a str,
    pub version: &'a str,
}

/// Шаблон: непустой, известные токены, обязательно `{Неделя}`, без разделителей пути.
pub fn validate_pattern(pattern: &str) -> Result<(), BackendError> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return Err(BackendError::conflict("Шаблон итогового имени пуст"));
    }
    if pattern.chars().count() > MAX_PATTERN_CHARS {
        return Err(BackendError::conflict(format!(
            "Шаблон итогового имени длиннее {MAX_PATTERN_CHARS} символов"
        )));
    }
    if has_forbidden_chars(pattern) {
        return Err(BackendError::conflict(
            "В шаблоне итогового имени нельзя использовать «/», «\\», «:» и управляющие символы",
        ));
    }
    if !pattern.contains("{Неделя}") {
        return Err(BackendError::conflict(
            "В шаблоне итогового имени нужен {Неделя}: иначе имена разных недель совпадут",
        ));
    }
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        if rest[..open].contains('}') {
            return Err(BackendError::conflict("Шаблон итогового имени некорректен"));
        }
        let after = &rest[open..];
        let Some(close) = after.find('}') else {
            return Err(BackendError::conflict(
                "В шаблоне итогового имени незакрытая «{»",
            ));
        };
        let token = &after[..=close];
        if !TOKENS.contains(&token) {
            return Err(BackendError::conflict(format!(
                "Неизвестный токен {token}; доступны {}",
                TOKENS.join(" ")
            )));
        }
        rest = &after[close + 1..];
    }
    if rest.contains('}') || pattern.starts_with('.') {
        return Err(BackendError::conflict("Шаблон итогового имени некорректен"));
    }
    Ok(())
}

pub fn validate_name_part(value: &str) -> Result<(), BackendError> {
    if value.chars().count() > MAX_NAME_CHARS || has_forbidden_chars(value) {
        return Err(BackendError::conflict(
            "Фамилия и имя — до 60 символов, без «/», «\\», «:» и управляющих символов",
        ));
    }
    Ok(())
}

fn has_forbidden_chars(value: &str) -> bool {
    value
        .chars()
        .any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
}

fn is_unset(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || value == UNSET_NAME
}

/// Значение токена не может создать путь: разделители заменяются на «-».
fn clean(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect()
}

/// Итоговое имя. Блокирует, если шаблон невалиден или использует незаданные фамилию/имя.
pub fn final_file_name(
    pattern: &str,
    tokens: &NameTokens<'_>,
    extension: Option<&str>,
) -> Result<String, BackendError> {
    validate_pattern(pattern)?;
    let needs_last = pattern.contains("{Фамилия}");
    let needs_first = pattern.contains("{Имя}");
    if (needs_last && is_unset(tokens.last_name)) || (needs_first && is_unset(tokens.first_name)) {
        return Err(BackendError::blocked(
            "Укажите фамилию и имя в «Настройки → Имя итогового файла»: они входят в итоговое имя файла",
        ));
    }
    let base = pattern
        .replace("{Фамилия}", &clean(tokens.last_name))
        .replace("{Имя}", &clean(tokens.first_name))
        .replace("{Неделя}", &tokens.week.to_string())
        .replace("{Предмет}", &clean(tokens.subject))
        .replace("{Версия}", &clean(tokens.version));
    let base = base.trim().to_owned();
    let name = match extension.map(clean).filter(|ext| !ext.is_empty()) {
        Some(ext) => format!("{base}.{ext}"),
        None => base,
    };
    if name.is_empty() || name.starts_with('.') || name.len() > 255 {
        return Err(BackendError::blocked(
            "Итоговое имя файла по шаблону получилось некорректным",
        ));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens<'a>(last: &'a str, first: &'a str) -> NameTokens<'a> {
        NameTokens {
            last_name: last,
            first_name: first,
            week: 7,
            subject: "Статистика",
            version: "v03",
        }
    }

    #[test]
    fn substitutes_tokens_like_frontend_preview() {
        let name = final_file_name(
            "{Фамилия}_{Имя}_Неделя{Неделя} ДЗ {Предмет} {Версия}",
            &tokens("Гончарова", "Доминика"),
            Some("pdf"),
        )
        .unwrap();
        assert_eq!(name, "Гончарова_Доминика_Неделя7 ДЗ Статистика v03.pdf");
    }

    #[test]
    fn token_values_cannot_build_paths() {
        let mut t = tokens("Гончарова", "Доминика");
        t.subject = "Алгебра/Геометрия\\2";
        let name = final_file_name("{Предмет} {Неделя}", &t, None).unwrap();
        assert_eq!(name, "Алгебра-Геометрия-2 7");
        // Скрытое имя («..-…») не создаётся.
        t.subject = "../x";
        assert!(final_file_name("{Предмет} {Неделя}", &t, None).is_err());
    }

    #[test]
    fn blocks_unset_student_name_only_when_used() {
        let error = final_file_name(
            "{Фамилия} {Неделя}",
            &tokens("Не задано", "Не задано"),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::Blocked);
        assert_eq!(
            final_file_name("ДЗ {Неделя}", &tokens("Не задано", ""), Some("md")).unwrap(),
            "ДЗ 7.md"
        );
    }

    #[test]
    fn rejects_bad_patterns() {
        for bad in [
            "",
            "   ",
            "{Фамилия}",
            "{Неделя}/x",
            "{Неделя} {Группа}",
            "{Неделя",
            "Неделя}{Неделя}",
            ".{Неделя}",
            "a:{Неделя}",
        ] {
            assert!(validate_pattern(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(validate_pattern("{Фамилия}_{Имя}_Неделя{Неделя} ДЗ").is_ok());
    }
}
