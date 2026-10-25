//! Адаптеры официальных CLI.
//!
//! Codex CLI (`codex exec`) и Claude Code (`claude -p`) — официальные
//! неинтерактивные режимы. «Контур» не хранит их учётные данные: вход
//! выполняется самим CLI (`codex login`, `claude auth login`), а «Контур»
//! проверяет только код выхода `codex login status` / `claude auth status`,
//! не читая и не сохраняя их вывод.
//!
//! Перед запуском проверяется версия и наличие нужных флагов в `--help`.
//! Codex запускается в песочнице `read-only` в пустом рабочем каталоге;
//! Claude — без инструментов (`--tools ""`) и с одним ходом.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde_json::Value;

use crate::dto::ProviderId;

use super::{
    locate::Executable,
    process::{run_process, safe_env, ExitKind, ProcessOutcome, ProcessSpec, Tick},
    redact::{redact, tail_chars},
};

/// Лимиты вывода CLI.
pub const RESPONSE_CAP_BYTES: usize = 2 * 1024 * 1024;
const STDERR_CAP_BYTES: usize = 64 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

pub fn executable_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Claude => "claude",
    }
}

/// Флаги, без которых безопасный неинтерактивный запуск невозможен.
fn required_flags(provider: ProviderId) -> &'static [&'static str] {
    match provider {
        ProviderId::Codex => &[
            "--sandbox",
            "--skip-git-repo-check",
            "--output-last-message",
            "--cd",
            "--color",
        ],
        ProviderId::Claude => &["--print", "--output-format", "--max-turns", "--tools"],
    }
}

/// Необязательные флаги, которые добавляются, если версия их знает.
fn optional_flags(provider: ProviderId) -> &'static [&'static str] {
    match provider {
        ProviderId::Codex => &["--ephemeral"],
        ProviderId::Claude => &[
            "--no-session-persistence",
            "--strict-mcp-config",
            "--disable-slash-commands",
        ],
    }
}

fn help_args(provider: ProviderId) -> Vec<&'static str> {
    match provider {
        ProviderId::Codex => vec!["exec", "--help"],
        ProviderId::Claude => vec!["--help"],
    }
}

fn auth_args(provider: ProviderId) -> Vec<&'static str> {
    match provider {
        ProviderId::Codex => vec!["login", "status"],
        ProviderId::Claude => vec!["auth", "status"],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub version: Option<String>,
    pub supported: bool,
    pub missing_flags: Vec<String>,
    pub optional_flags: Vec<String>,
    /// `None` — состояние входа определить не удалось.
    pub logged_in: Option<bool>,
    pub error: Option<String>,
}

fn flag_present(help: &str, flag: &str) -> bool {
    help.split(|c: char| c.is_whitespace() || matches!(c, ',' | '[' | ']' | '=' | '<'))
        .any(|token| token == flag)
}

fn sanitize_version(raw: &str) -> Option<String> {
    let line = raw.lines().find(|line| !line.trim().is_empty())?.trim();
    let clean: String = line
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | ' ' | '(' | ')' | '+'))
        .take(60)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

/// Короткий запуск для проверки возможностей: пустой каталог, чистое окружение.
fn probe_run(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    home: Option<&Path>,
    keep_output: bool,
) -> ProcessOutcome {
    run_process(
        &ProcessSpec {
            program: exe.to_path_buf(),
            args: args.iter().map(OsString::from).collect(),
            cwd: cwd.to_path_buf(),
            env: safe_env(home, exe, cwd),
            stdin: None,
            timeout: PROBE_TIMEOUT,
            stdout_cap: if keep_output { 256 * 1024 } else { 0 },
            stderr_cap: if keep_output { 16 * 1024 } else { 0 },
        },
        &AtomicBool::new(false),
        &mut |_| {},
    )
}

/// Проверяет версию, нужные флаги и состояние входа. Вывод проверки входа
/// не читается и не сохраняется (он может содержать маскированный ключ).
pub fn probe(
    provider: ProviderId,
    exe: &Executable,
    cwd: &Path,
    home: Option<&Path>,
) -> ProbeReport {
    let version_run = probe_run(&exe.path, &["--version"], cwd, home, true);
    let version = match &version_run.exit {
        ExitKind::Exited(0) => sanitize_version(&String::from_utf8_lossy(&version_run.stdout)),
        _ => None,
    };
    if version.is_none() {
        return ProbeReport {
            version: None,
            supported: false,
            missing_flags: Vec::new(),
            optional_flags: Vec::new(),
            logged_in: None,
            error: Some(match version_run.exit {
                ExitKind::TimedOut => "CLI не ответил на --version за 15 секунд".to_owned(),
                ExitKind::SpawnFailed(message) => format!("CLI не запустился: {message}"),
                _ => "CLI не сообщил версию — возможно, установка повреждена".to_owned(),
            }),
        };
    }
    let help_run = probe_run(&exe.path, &help_args(provider), cwd, home, true);
    let mut help = String::from_utf8_lossy(&help_run.stdout).into_owned();
    help.push('\n');
    help.push_str(&String::from_utf8_lossy(&help_run.stderr));
    let missing_flags: Vec<String> = required_flags(provider)
        .iter()
        .filter(|flag| !flag_present(&help, flag))
        .map(|flag| (*flag).to_owned())
        .collect();
    let optional: Vec<String> = optional_flags(provider)
        .iter()
        .filter(|flag| flag_present(&help, flag))
        .map(|flag| (*flag).to_owned())
        .collect();
    let auth_known = match provider {
        ProviderId::Codex => true,
        // `claude auth status` есть не во всех версиях — проверяем по справке.
        ProviderId::Claude => help
            .lines()
            .any(|line| line.trim_start().starts_with("auth")),
    };
    let logged_in = if auth_known {
        let auth = probe_run(&exe.path, &auth_args(provider), cwd, home, false);
        match auth.exit {
            ExitKind::Exited(0) => Some(true),
            ExitKind::Exited(_) => Some(false),
            _ => None,
        }
    } else {
        None
    };
    let supported = missing_flags.is_empty();
    ProbeReport {
        error: (!supported).then(|| {
            format!(
                "Версия CLI не поддерживает безопасный неинтерактивный запуск (нет {})",
                missing_flags.join(", ")
            )
        }),
        version,
        supported,
        missing_flags,
        optional_flags: optional,
        logged_in,
    }
}

// ───────────────────────── Выполнение ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Timeout,
    Cancelled,
    AuthRequired,
    ExitNonZero,
    ProviderError,
    Spawn,
    Empty,
}

impl FailureKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::AuthRequired => "reauth_required",
            Self::ExitNonZero => "provider_error",
            Self::ProviderError => "provider_error",
            Self::Spawn => "provider_unavailable",
            Self::Empty => "empty_response",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterResponse {
    pub text: String,
    pub truncated: bool,
    pub exit_code: Option<i64>,
    pub stderr_excerpt: String,
    pub duration_ms: u64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterFailure {
    pub kind: FailureKind,
    pub message: String,
    pub exit_code: Option<i64>,
    pub stderr_excerpt: String,
    pub duration_ms: u64,
    /// Частичный вывод (например, до тайм-аута) — сохраняется для пользователя.
    pub partial_text: Option<String>,
}

pub type AdapterResult = Result<AdapterResponse, AdapterFailure>;

pub struct AdapterRequest<'a> {
    pub prompt: &'a str,
    /// Пустой рабочий каталог задания.
    pub workdir: &'a Path,
    pub timeout: Duration,
}

pub struct RunControl<'a> {
    pub cancel: &'a AtomicBool,
    pub on_tick: &'a mut dyn FnMut(Tick),
}

/// Исполнитель запроса к провайдеру. Production — `CliAdapter`, тесты —
/// fake-адаптеры без процессов и сети.
pub trait AiAdapter: Send + Sync {
    fn label(&self) -> String;
    fn execute(&self, request: &AdapterRequest<'_>, control: RunControl<'_>) -> AdapterResult;
}

#[derive(Debug, Clone)]
pub struct CliAdapter {
    pub provider: ProviderId,
    pub executable: Executable,
    pub version: Option<String>,
    pub optional_flags: Vec<String>,
    pub home: Option<PathBuf>,
}

impl CliAdapter {
    pub fn args(&self, workdir: &Path) -> Vec<OsString> {
        let has = |flag: &str| self.optional_flags.iter().any(|item| item == flag);
        let mut args: Vec<OsString> = Vec::new();
        match self.provider {
            ProviderId::Codex => {
                args.extend(
                    [
                        "exec",
                        "--sandbox",
                        "read-only",
                        "--skip-git-repo-check",
                        "--color",
                        "never",
                    ]
                    .map(OsString::from),
                );
                if has("--ephemeral") {
                    args.push("--ephemeral".into());
                }
                args.push("--cd".into());
                args.push(workdir.as_os_str().to_owned());
                args.push("--output-last-message".into());
                args.push(workdir.join(LAST_MESSAGE_FILE).into_os_string());
                // Промт читается из stdin.
                args.push("-".into());
            }
            ProviderId::Claude => {
                args.extend(
                    [
                        "--print",
                        "--output-format",
                        "json",
                        "--max-turns",
                        "1",
                        "--tools",
                        "",
                    ]
                    .map(OsString::from),
                );
                for flag in [
                    "--no-session-persistence",
                    "--strict-mcp-config",
                    "--disable-slash-commands",
                ] {
                    if has(flag) {
                        args.push(flag.into());
                    }
                }
            }
        }
        args
    }
}

const LAST_MESSAGE_FILE: &str = "last-message.txt";

fn looks_like_auth_problem(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "not logged in",
        "login",
        "log in",
        "invalid api key",
        "authentication",
        "unauthorized",
        "401",
        "please run /login",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn read_capped(path: &Path) -> Option<(String, bool)> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buffer = Vec::new();
    file.take(RESPONSE_CAP_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .ok()?;
    let truncated = buffer.len() > RESPONSE_CAP_BYTES;
    buffer.truncate(RESPONSE_CAP_BYTES);
    Some((String::from_utf8_lossy(&buffer).into_owned(), truncated))
}

impl AiAdapter for CliAdapter {
    fn label(&self) -> String {
        let name = match self.provider {
            ProviderId::Codex => "Codex CLI",
            ProviderId::Claude => "Claude Code CLI",
        };
        match &self.version {
            Some(version) => format!("{name} · {version}"),
            None => name.to_owned(),
        }
    }

    fn execute(&self, request: &AdapterRequest<'_>, control: RunControl<'_>) -> AdapterResult {
        let tmp = request.workdir.join("tmp");
        let _ = std::fs::create_dir_all(&tmp);
        let outcome = run_process(
            &ProcessSpec {
                program: self.executable.path.clone(),
                args: self.args(request.workdir),
                cwd: request.workdir.to_path_buf(),
                env: safe_env(self.home.as_deref(), &self.executable.path, &tmp),
                stdin: Some(request.prompt.as_bytes().to_vec()),
                timeout: request.timeout,
                stdout_cap: RESPONSE_CAP_BYTES,
                stderr_cap: STDERR_CAP_BYTES,
            },
            control.cancel,
            control.on_tick,
        );
        interpret(
            self.provider,
            outcome,
            request.workdir,
            self.home.as_deref(),
        )
    }
}

/// Превращает итог процесса в ответ или типизированную ошибку.
pub fn interpret(
    provider: ProviderId,
    outcome: ProcessOutcome,
    workdir: &Path,
    home: Option<&Path>,
) -> AdapterResult {
    let duration_ms = outcome.duration.as_millis() as u64;
    let stderr_text = String::from_utf8_lossy(&outcome.stderr).into_owned();
    let stderr_excerpt = tail_chars(&redact(&stderr_text, home), 4000);
    let stdout_text = String::from_utf8_lossy(&outcome.stdout).into_owned();
    let exit_code = match outcome.exit {
        ExitKind::Exited(code) => Some(i64::from(code)),
        _ => None,
    };
    let fail = |kind: FailureKind, message: String, partial: Option<String>| AdapterFailure {
        kind,
        message: redact(&message, home),
        exit_code,
        stderr_excerpt: stderr_excerpt.clone(),
        duration_ms,
        partial_text: partial.filter(|text| !text.trim().is_empty()),
    };
    match &outcome.exit {
        ExitKind::Cancelled => {
            return Err(fail(
                FailureKind::Cancelled,
                "Запуск остановлен пользователем".to_owned(),
                None,
            ))
        }
        ExitKind::TimedOut => {
            return Err(fail(
                FailureKind::Timeout,
                "Провайдер не ответил за отведённое время — процесс остановлен".to_owned(),
                Some(stdout_text),
            ))
        }
        ExitKind::SpawnFailed(message) => {
            return Err(fail(FailureKind::Spawn, message.clone(), None))
        }
        ExitKind::Signaled => {
            return Err(fail(
                FailureKind::ExitNonZero,
                "Процесс CLI завершён сигналом".to_owned(),
                Some(stdout_text),
            ))
        }
        ExitKind::Exited(_) => {}
    }
    let mut notes = Vec::new();
    let (text, truncated) = match provider {
        ProviderId::Codex => match read_capped(&workdir.join(LAST_MESSAGE_FILE)) {
            Some((text, truncated)) if !text.trim().is_empty() => (text, truncated),
            _ => {
                notes.push("Файл итогового сообщения Codex пуст — ответ взят из stdout".to_owned());
                (stdout_text.clone(), outcome.stdout_truncated)
            }
        },
        ProviderId::Claude => match serde_json::from_str::<Value>(stdout_text.trim()) {
            Ok(Value::Object(object)) => {
                let result = object
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if object.get("is_error").and_then(Value::as_bool) == Some(true) {
                    let kind = if looks_like_auth_problem(&result) {
                        FailureKind::AuthRequired
                    } else {
                        FailureKind::ProviderError
                    };
                    return Err(fail(
                        kind,
                        format!("Claude вернул ошибку: {}", tail_chars(&result, 300)),
                        None,
                    ));
                }
                (result, outcome.stdout_truncated)
            }
            _ => {
                notes
                    .push("Вывод Claude не является JSON-конвертом — сохранён как есть".to_owned());
                (stdout_text.clone(), outcome.stdout_truncated)
            }
        },
    };
    if exit_code != Some(0) {
        let kind = if looks_like_auth_problem(&stderr_text) || looks_like_auth_problem(&text) {
            FailureKind::AuthRequired
        } else {
            FailureKind::ExitNonZero
        };
        let message = match kind {
            FailureKind::AuthRequired => format!(
                "Требуется вход в официальном CLI: выполните `{}` в Терминале",
                match provider {
                    ProviderId::Codex => "codex login",
                    ProviderId::Claude => "claude auth login",
                }
            ),
            _ => format!(
                "CLI завершился с кодом {}",
                exit_code.map_or("—".to_owned(), |code| code.to_string())
            ),
        };
        return Err(fail(kind, message, Some(text)));
    }
    if text.trim().is_empty() {
        return Err(fail(
            FailureKind::Empty,
            "Провайдер вернул пустой ответ".to_owned(),
            None,
        ));
    }
    Ok(AdapterResponse {
        text,
        truncated,
        exit_code,
        stderr_excerpt,
        duration_ms,
        notes,
    })
}

/// Фабрика адаптеров. Production строит `CliAdapter` по результату проверки,
/// тесты подставляют fake-адаптеры.
pub trait AdapterFactory: Send + Sync {
    fn adapter(&self, provider: ProviderId) -> Result<Arc<dyn AiAdapter>, crate::error::AppError>;
}

#[cfg(test)]
mod tests {
    use std::{
        fs, os::unix::fs::PermissionsExt, path::Path, sync::atomic::AtomicBool, time::Duration,
    };

    use tempfile::TempDir;

    use super::{probe, AdapterRequest, AiAdapter, CliAdapter, FailureKind, RunControl};
    use crate::{ai::locate::Executable, dto::ProviderId};

    /// Фиктивный CLI в TempDir: никакой сети, только ответы по аргументам.
    fn fake_cli(dir: &Path, name: &str, body: &str) -> Executable {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        Executable {
            path,
            display: name.to_owned(),
        }
    }

    const CODEX: &str = r#"
case "$1" in
  --version) echo "codex-cli 0.99.0"; exit 0;;
  login) exit 0;;
  exec)
    if [ "$2" = "--help" ]; then
      echo "Usage: codex exec [OPTIONS] [PROMPT]"
      echo "  -s, --sandbox <SANDBOX_MODE>"
      echo "      --skip-git-repo-check"
      echo "      --ephemeral"
      echo "  -C, --cd <DIR>"
      echo "      --color <COLOR>"
      echo "  -o, --output-last-message <FILE>"
      exit 0
    fi
    out=""; prev=""
    for a in "$@"; do [ "$prev" = "--output-last-message" ] && out="$a"; prev="$a"; done
    prompt=$(cat)
    echo "thinking..."
    printf '{"summary":"Итог: %s","promptText":"Сделай ДЗ"}' "$(printf '%s' "$prompt" | head -c 21)" > "$out"
    exit 0;;
esac
exit 2
"#;

    #[test]
    fn probe_detects_version_flags_and_login_without_reading_auth_output() {
        let tmp = TempDir::new().unwrap();
        let exe = fake_cli(tmp.path(), "codex", CODEX);
        let report = probe(ProviderId::Codex, &exe, tmp.path(), None);
        assert_eq!(report.version.as_deref(), Some("codex-cli 0.99.0"));
        assert!(report.supported, "{report:?}");
        assert_eq!(report.optional_flags, vec!["--ephemeral".to_owned()]);
        assert_eq!(report.logged_in, Some(true));
    }

    #[test]
    fn probe_rejects_versions_without_safe_flags() {
        let tmp = TempDir::new().unwrap();
        let exe = fake_cli(
            tmp.path(),
            "claude",
            r#"case "$1" in --version) echo "1.0.0 (Claude Code)";; --help) echo "  -p, --print"; echo "  --output-format <format>";; esac"#,
        );
        let report = probe(ProviderId::Claude, &exe, tmp.path(), None);
        assert!(!report.supported);
        assert_eq!(
            report.missing_flags,
            vec!["--max-turns".to_owned(), "--tools".to_owned()]
        );
        assert_eq!(report.logged_in, None, "no auth subcommand in help");
    }

    #[test]
    fn codex_adapter_reads_last_message_from_isolated_workdir() {
        let tmp = TempDir::new().unwrap();
        let work = tmp.path().join("job");
        fs::create_dir(&work).unwrap();
        let adapter = CliAdapter {
            provider: ProviderId::Codex,
            executable: fake_cli(tmp.path(), "codex", CODEX),
            version: Some("codex-cli 0.99.0".into()),
            optional_flags: vec!["--ephemeral".into()],
            home: None,
        };
        let args: Vec<String> = adapter
            .args(&work)
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(
            &args[..6],
            [
                "exec",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--color",
                "never"
            ]
        );
        assert!(args.contains(&"--ephemeral".to_owned()));
        assert_eq!(args.last().unwrap(), "-", "prompt goes through stdin");
        assert!(
            !args.iter().any(|a| a.contains("Материалы")),
            "prompt is never an argument"
        );
        let cancel = AtomicBool::new(false);
        let response = adapter
            .execute(
                &AdapterRequest {
                    prompt: "Материалы курса",
                    workdir: &work,
                    timeout: Duration::from_secs(10),
                },
                RunControl {
                    cancel: &cancel,
                    on_tick: &mut |_| {},
                },
            )
            .unwrap();
        assert!(
            response.text.contains("\"summary\":\"Итог: Материалы к"),
            "{}",
            response.text
        );
        assert_eq!(adapter.label(), "Codex CLI · codex-cli 0.99.0");
    }

    #[test]
    fn claude_adapter_unwraps_json_envelope_and_maps_auth_errors() {
        let tmp = TempDir::new().unwrap();
        let ok = CliAdapter {
            provider: ProviderId::Claude,
            executable: fake_cli(
                tmp.path(),
                "claude",
                r#"cat >/dev/null; echo '{"type":"result","is_error":false,"result":"{\"text\":\"Исправь пункт 2\"}"}'"#,
            ),
            version: None,
            optional_flags: vec![],
            home: None,
        };
        let cancel = AtomicBool::new(false);
        let request = AdapterRequest {
            prompt: "x",
            workdir: tmp.path(),
            timeout: Duration::from_secs(10),
        };
        let response = ok
            .execute(
                &request,
                RunControl {
                    cancel: &cancel,
                    on_tick: &mut |_| {},
                },
            )
            .unwrap();
        assert_eq!(response.text, r#"{"text":"Исправь пункт 2"}"#);
        let args: Vec<String> = ok
            .args(tmp.path())
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "--print",
                "--output-format",
                "json",
                "--max-turns",
                "1",
                "--tools",
                ""
            ]
        );

        let other = TempDir::new().unwrap();
        let denied = CliAdapter {
            executable: fake_cli(
                other.path(),
                "claude",
                r#"cat >/dev/null; echo '{"type":"result","is_error":true,"result":"Invalid API key · Please run /login"}'; exit 1"#,
            ),
            ..ok
        };
        let failure = denied
            .execute(
                &request,
                RunControl {
                    cancel: &cancel,
                    on_tick: &mut |_| {},
                },
            )
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::AuthRequired);
    }

    #[test]
    fn nonzero_exit_keeps_partial_output_and_redacts_stderr() {
        let tmp = TempDir::new().unwrap();
        let adapter = CliAdapter {
            provider: ProviderId::Codex,
            executable: fake_cli(tmp.path(), "codex", "cat >/dev/null; echo '{\"summary\":\"полу'; echo 'error: key sk-abcdefghijklmnopqrstu rejected' >&2; exit 7"),
            version: None,
            optional_flags: vec![],
            home: None,
        };
        let cancel = AtomicBool::new(false);
        let failure = adapter
            .execute(
                &AdapterRequest {
                    prompt: "x",
                    workdir: tmp.path(),
                    timeout: Duration::from_secs(10),
                },
                RunControl {
                    cancel: &cancel,
                    on_tick: &mut |_| {},
                },
            )
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::ExitNonZero);
        assert_eq!(failure.exit_code, Some(7));
        assert!(failure.partial_text.unwrap().contains("полу"));
        assert!(!failure.stderr_excerpt.contains("sk-abcdefghijklmnopqrstu"));
    }
}
