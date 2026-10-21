//! Ссылки на курс и задание в LMS Центрального университета.
//!
//! Ссылки — необязательные пользовательские метаданные. Они хранятся только
//! после проверки allowlist и открываются во внешнем браузере пользователя
//! из Rust (официальный `tauri-plugin-opener`). Авторизация, cookies и сессия
//! LMS приложением не используются и не сохраняются.

use url::{Host, Url};

use crate::error::{AppError, BackendError};

/// Разрешённые HTTPS-хосты LMS (точное совпадение, без поддоменов).
pub const ALLOWED_LMS_HOSTS: &[&str] = &["my.centraluniversity.ru"];
const MAX_URL_LENGTH: usize = 2048;

/// Проверяет ссылку и возвращает её каноническую форму.
///
/// Разрешены только `https://` на хосте из [`ALLOWED_LMS_HOSTS`], без
/// логина/пароля в URL, без нестандартного порта и без фрагментов-скриптов.
/// Пустая строка означает «ссылки нет».
pub fn normalize_lms_url(raw: &str) -> Result<Option<String>, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_URL_LENGTH {
        return Err(BackendError::blocked("Ссылка слишком длинная"));
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(BackendError::blocked(
            "Ссылка содержит пробелы или управляющие символы",
        ));
    }
    let url = Url::parse(trimmed)
        .map_err(|_| BackendError::blocked("Ссылка не похожа на адрес страницы LMS"))?;
    if url.scheme() != "https" {
        return Err(BackendError::blocked("Разрешены только ссылки https://"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BackendError::blocked(
            "Ссылка не должна содержать логин или пароль",
        ));
    }
    if url.port().is_some() {
        return Err(BackendError::blocked("Ссылка не должна указывать порт"));
    }
    let host = match url.host() {
        Some(Host::Domain(domain)) => domain.to_ascii_lowercase(),
        _ => {
            return Err(BackendError::blocked(
                "Ссылка должна вести на домен LMS, а не на IP-адрес",
            ))
        }
    };
    if !ALLOWED_LMS_HOSTS.contains(&host.as_str()) {
        return Err(BackendError::blocked(format!(
            "Разрешены только ссылки на {}",
            ALLOWED_LMS_HOSTS.join(", ")
        )));
    }
    let normalized = url.to_string();
    if normalized.len() > MAX_URL_LENGTH {
        return Err(BackendError::blocked("Ссылка слишком длинная"));
    }
    Ok(Some(normalized))
}

/// Повторная проверка сохранённой ссылки перед открытием во внешнем браузере.
pub fn require_lms_url(stored: Option<&str>, missing: &str) -> Result<String, AppError> {
    let stored = stored.ok_or_else(|| BackendError::not_found(missing))?;
    normalize_lms_url(stored)?.ok_or_else(|| BackendError::not_found(missing))
}

#[cfg(test)]
mod tests {
    use super::normalize_lms_url;
    use crate::error::ErrorCode;

    #[test]
    fn accepts_only_https_lms_host() {
        assert_eq!(
            normalize_lms_url("https://my.centraluniversity.ru/learn/courses/view/actual/123")
                .unwrap()
                .as_deref(),
            Some("https://my.centraluniversity.ru/learn/courses/view/actual/123")
        );
        assert_eq!(
            normalize_lms_url("  HTTPS://MY.centraluniversity.ru/x  ")
                .unwrap()
                .as_deref(),
            Some("https://my.centraluniversity.ru/x")
        );
        assert_eq!(normalize_lms_url("").unwrap(), None);
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "http://my.centraluniversity.ru/x",
            "https://evil.example/x",
            "https://my.centraluniversity.ru.evil.example/x",
            "https://evil.my.centraluniversity.ru/x",
            "https://user:pass@my.centraluniversity.ru/x",
            "https://my.centraluniversity.ru:8443/x",
            "https://93.184.216.34/x",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "my.centraluniversity.ru/x",
            "https://my.centraluniversity.ru/a b",
            "https://my.centraluniversity.ru/\u{0007}",
        ] {
            let error = normalize_lms_url(bad).expect_err(bad);
            assert_eq!(error.code, ErrorCode::Blocked, "{bad}");
        }
        assert!(normalize_lms_url(&format!(
            "https://my.centraluniversity.ru/{}",
            "a".repeat(3000)
        ))
        .is_err());
    }
}
