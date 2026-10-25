//! Редакция журнала запуска: stderr, сообщения об ошибках и строки журнала
//! не должны сохранять секреты, адреса почты и абсолютные пути пользователя.
//! Ответ провайдера — содержание работы, он не редактируется.

use std::path::Path;

const MASK: &str = "[скрыто]";

/// Признаки ключей, значение которых всегда скрывается.
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "token",
    "secret",
    "password",
    "passwd",
    "cookie",
    "session",
    "authorization",
    "bearer",
];

/// Префиксы известных форматов ключей.
const TOKEN_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "pk-",
    "rk-",
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "AIza",
    "ya29.",
    "eyJ",
];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '/' | '=')
}

fn looks_like_secret(word: &str) -> bool {
    let core: &str = word.trim_matches(|c: char| !is_token_char(c));
    if TOKEN_PREFIXES
        .iter()
        .any(|prefix| core.starts_with(prefix) && core.len() >= prefix.len() + 8)
    {
        return true;
    }
    // Длинная слитная строка из символов base64/hex — вероятный ключ или токен.
    core.len() >= 32
        && core.chars().all(is_token_char)
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().any(|c| c.is_ascii_alphabetic())
        && !core.contains('/')
}

fn looks_like_email(word: &str) -> bool {
    let core = word
        .trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '@' | '.' | '_' | '-' | '+')));
    match core.split_once('@') {
        Some((user, domain)) => {
            !user.is_empty() && domain.contains('.') && !domain.starts_with('.')
        }
        None => false,
    }
}

fn secret_key(word: &str) -> bool {
    let lower = word.to_lowercase();
    SECRET_KEYS.iter().any(|key| lower.contains(key))
}

/// Заменяет домашний каталог и абсолютные пользовательские пути на `~`.
fn redact_paths(text: &str, home: Option<&Path>) -> String {
    let mut output = text.to_owned();
    if let Some(home) = home.and_then(|path| path.to_str()) {
        if home.len() > 1 {
            output = output.replace(home, "~");
        }
    }
    for root in [
        "/Users/",
        "/home/",
        "/private/var/folders/",
        "/var/folders/",
    ] {
        let mut result = String::with_capacity(output.len());
        let mut rest = output.as_str();
        while let Some(index) = rest.find(root) {
            result.push_str(&rest[..index]);
            let after = &rest[index + root.len()..];
            // Пропустить имя пользователя (или служебный сегмент каталога).
            let end = after.find('/').unwrap_or(after.len());
            let segment_end = after[..end]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']'))
                .unwrap_or(end);
            result.push('~');
            rest = &after[segment_end..];
        }
        result.push_str(rest);
        output = result;
    }
    output
}

/// Редакция одной строки журнала или stderr.
pub fn redact(text: &str, home: Option<&Path>) -> String {
    let text = redact_paths(text, home);
    let mut output = String::with_capacity(text.len());
    let mut mask_next = false;
    let mut word = String::new();
    let flush = |word: &mut String, output: &mut String, mask_next: &mut bool| {
        if word.is_empty() {
            return;
        }
        let current = std::mem::take(word);
        if *mask_next && !current.trim().is_empty() {
            if current.eq_ignore_ascii_case("bearer") || current.eq_ignore_ascii_case("basic") {
                // «Authorization: Bearer <token>» — скрывается следующее слово.
                output.push_str(&current);
                return;
            }
            output.push_str(MASK);
            *mask_next = false;
            return;
        }
        if let Some((key, value)) = current.split_once(['=', ':']) {
            if secret_key(key) {
                output.push_str(key);
                output.push_str(&current[key.len()..key.len() + 1]);
                if value.is_empty() {
                    *mask_next = true;
                } else {
                    output.push_str(MASK);
                }
                return;
            }
        }
        if looks_like_email(&current) {
            output.push_str("[email]");
        } else if looks_like_secret(&current) {
            output.push_str(MASK);
        } else {
            if current.eq_ignore_ascii_case("bearer")
                || (secret_key(current.trim_end_matches(':')) && current.ends_with(':'))
            {
                *mask_next = true;
            }
            output.push_str(&current);
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut output, &mut mask_next);
            output.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut output, &mut mask_next);
    output
}

/// Последние `max_chars` символов (stderr обычно заканчивается сутью ошибки).
pub fn tail_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let skipped: String = text.chars().skip(count - max_chars).collect();
    format!("…{skipped}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{redact, tail_chars};

    #[test]
    fn masks_keys_tokens_emails_and_home_paths() {
        let home = Path::new("/Users/student");
        let input =
            "Error: OPENAI_API_KEY=sk-proj-abcdefghijklmnop123 invalid for nika@example.com \
                     at /Users/student/.codex/auth.json; Authorization: Bearer abc.def.ghi \
                     token: 9f8e7d6c5b4a39281706f5e4d3c2b1a0ffeeddcc";
        let output = redact(input, Some(home));
        assert!(!output.contains("sk-proj"));
        assert!(!output.contains("nika@example.com"));
        assert!(!output.contains("/Users/student"));
        assert!(!output.contains("abc.def.ghi"));
        assert!(!output.contains("9f8e7d6c5b4a"));
        assert!(output.contains("~/.codex/auth.json"));
        assert!(output.contains("[email]"));
    }

    #[test]
    fn keeps_ordinary_text_and_masks_other_users_paths() {
        let output = redact("Процесс завершён: код 1, файл /home/other/x.txt", None);
        assert_eq!(output, "Процесс завершён: код 1, файл ~/x.txt");
        assert_eq!(
            redact("Не удалось войти: выполните codex login", None),
            "Не удалось войти: выполните codex login"
        );
    }

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail_chars("абвгд", 3), "…вгд");
        assert_eq!(tail_chars("аб", 3), "аб");
    }
}
