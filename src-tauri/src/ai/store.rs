//! SQL этапа 9: задания `ai_jobs` и явное подключение CLI (`app_settings`).

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    dto::ProviderId,
    error::{AppError, BackendError},
};

use super::model::{
    provider_db_str, provider_from_db, AiIssueDto, AiJobDto, AiLogEntryDto, AiTask,
    ContextSourceDto, JobStatus, LogLevel, ParseStatus, ResponseSource, Transport,
};

const SETTINGS_KEY: &str = "ai_providers";
const MAX_LOG_ENTRIES: usize = 200;

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

pub fn timestamp() -> String {
    now()
}

fn db_error(error: impl std::fmt::Display) -> AppError {
    BackendError::database(error.to_string())
}

fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str, column: &str) -> Result<T, AppError> {
    serde_json::from_str(raw).map_err(|error| BackendError::database(format!("{column}: {error}")))
}

fn enum_column<T>(
    raw: &str,
    column: &str,
    parse: fn(&str) -> Result<T, String>,
) -> Result<T, AppError> {
    parse(raw).map_err(|error| BackendError::database(format!("{column}: {error}")))
}

// ───────────────────────── Подключение CLI ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSetting {
    /// Пользователь явно подключил официальный CLI. Только флаг — никаких
    /// путей, токенов и учётных данных.
    pub cli_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSettings {
    #[serde(default)]
    pub codex: ProviderSetting,
    #[serde(default)]
    pub claude: ProviderSetting,
}

impl ProviderSettings {
    pub fn get(&self, provider: ProviderId) -> ProviderSetting {
        match provider {
            ProviderId::Codex => self.codex,
            ProviderId::Claude => self.claude,
        }
    }

    pub fn set(&mut self, provider: ProviderId, setting: ProviderSetting) {
        match provider {
            ProviderId::Codex => self.codex = setting,
            ProviderId::Claude => self.claude = setting,
        }
    }
}

pub fn read_settings(connection: &Connection) -> Result<ProviderSettings, AppError> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            [SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()?;
    raw.map_or(Ok(ProviderSettings::default()), |raw| {
        parse_json(&raw, "app_settings.ai_providers")
    })
}

pub fn write_settings(
    connection: &Connection,
    settings: &ProviderSettings,
) -> Result<(), AppError> {
    let value = serde_json::to_string(settings).map_err(db_error)?;
    connection.execute(
        "INSERT INTO app_settings(key, value_json, created_at, updated_at)
         VALUES (?1, ?2, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = CURRENT_TIMESTAMP",
        params![SETTINGS_KEY, value],
    )?;
    Ok(())
}

// ───────────────────────── Задания ─────────────────────────

pub struct NewJob<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub run_id: &'a str,
    pub task: AiTask,
    pub provider: ProviderId,
    pub transport: Transport,
    pub adapter_label: Option<&'a str>,
    pub include_prompt: bool,
    pub prompt_text: &'a str,
    pub prompt_sha256: &'a str,
    pub context: &'a [ContextSourceDto],
    pub context_truncated: bool,
    pub limit_chars: u64,
    pub issues: &'a [AiIssueDto],
    pub log: &'a [AiLogEntryDto],
}

pub fn insert_job(connection: &Connection, job: &NewJob<'_>) -> Result<(), AppError> {
    let at = now();
    connection.execute(
        "INSERT INTO ai_jobs(id, homework_id, run_id, task, provider, transport, adapter_label, status,
                             include_prompt, prompt_text, prompt_sha256, prompt_chars, context_json,
                             context_truncated, limit_chars, issues_json, log_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'prepared', ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
        params![
            job.id,
            job.homework_id,
            job.run_id,
            job.task.as_db_str(),
            provider_db_str(job.provider),
            job.transport.as_db_str(),
            job.adapter_label,
            job.include_prompt,
            job.prompt_text,
            job.prompt_sha256,
            job.prompt_text.chars().count() as i64,
            serde_json::to_string(job.context).map_err(db_error)?,
            job.context_truncated,
            job.limit_chars as i64,
            serde_json::to_string(job.issues).map_err(db_error)?,
            serde_json::to_string(job.log).map_err(db_error)?,
            at,
        ],
    )
    .map_err(|error| match error {
        rusqlite::Error::SqliteFailure(_, Some(ref message)) if message.contains("submitted") => {
            BackendError::immutable("Работа отправлена — новые запуски невозможны")
        }
        other => other.into(),
    })?;
    Ok(())
}

const COLUMNS: &str = "id, homework_id, run_id, task, provider, transport, adapter_label, status,
    prompt_text, prompt_chars, prompt_sha256, context_json, context_truncated, limit_chars,
    response_text, response_source, response_file_name, response_truncated, parse_status, draft_json,
    issues_json, log_json, error_code, error_message, exit_code, duration_ms, created_at, sent_at,
    finished_at, confirmed_at, edited, prompt_id, include_prompt";

fn job_row(row: &Row<'_>, full: bool) -> Result<(AiJobDto, bool), AppError> {
    let task: String = row.get(3)?;
    let provider: String = row.get(4)?;
    let transport: String = row.get(5)?;
    let status: String = row.get(7)?;
    let context: String = row.get(11)?;
    let response_text: Option<String> = row.get(14)?;
    let response_source: Option<String> = row.get(15)?;
    let parse_status: Option<String> = row.get(18)?;
    let draft: Option<String> = row.get(19)?;
    let issues: String = row.get(20)?;
    let log: String = row.get(21)?;
    let duration: Option<i64> = row.get(25)?;
    let edited: Option<i64> = row.get(30)?;
    let include_prompt: bool = row.get(32)?;
    let dto = AiJobDto {
        id: row.get(0)?,
        homework_id: row.get(1)?,
        run_id: row.get(2)?,
        task: enum_column(&task, "ai_jobs.task", AiTask::try_from_db)?,
        provider: enum_column(&provider, "ai_jobs.provider", provider_from_db)?,
        transport: enum_column(&transport, "ai_jobs.transport", Transport::try_from_db)?,
        adapter_label: row.get(6)?,
        status: enum_column(&status, "ai_jobs.status", JobStatus::try_from_db)?,
        prompt_text: if full { Some(row.get(8)?) } else { None },
        prompt_chars: row.get::<_, i64>(9)?.max(0) as u64,
        prompt_sha256: row.get(10)?,
        context: parse_json(&context, "ai_jobs.context_json")?,
        context_truncated: row.get(12)?,
        limit_chars: row.get::<_, i64>(13)?.max(0) as u64,
        response_chars: response_text
            .as_deref()
            .map_or(0, |text| text.chars().count() as u64),
        response_text: if full { response_text } else { None },
        response_source: response_source
            .as_deref()
            .map(|value| {
                enum_column(
                    value,
                    "ai_jobs.response_source",
                    ResponseSource::try_from_db,
                )
            })
            .transpose()?,
        response_file_name: row.get(16)?,
        response_truncated: row.get(17)?,
        parse_status: parse_status
            .as_deref()
            .map(|value| enum_column(value, "ai_jobs.parse_status", ParseStatus::try_from_db))
            .transpose()?,
        draft: if full {
            draft
                .as_deref()
                .map(|raw| parse_json::<Value>(raw, "ai_jobs.draft_json"))
                .transpose()?
        } else {
            None
        },
        issues: parse_json(&issues, "ai_jobs.issues_json")?,
        log: parse_json(&log, "ai_jobs.log_json")?,
        error_code: row.get(22)?,
        error_message: row.get(23)?,
        exit_code: row.get(24)?,
        duration_ms: duration.map(|value| value.max(0) as u64),
        created_at: row.get(26)?,
        sent_at: row.get(27)?,
        finished_at: row.get(28)?,
        confirmed_at: row.get(29)?,
        edited: edited.map(|value| value == 1),
        prompt_id: row.get(31)?,
    };
    Ok((dto, include_prompt))
}

/// Задание целиком (с текстом промта, ответом и черновиком).
pub fn job(connection: &Connection, job_id: &str) -> Result<AiJobDto, AppError> {
    job_with_flags(connection, job_id).map(|(job, _)| job)
}

/// Задание и признак «анализ просил и первичный промт».
pub fn job_with_flags(connection: &Connection, job_id: &str) -> Result<(AiJobDto, bool), AppError> {
    let mut statement =
        connection.prepare(&format!("SELECT {COLUMNS} FROM ai_jobs WHERE id = ?1"))?;
    let mut rows = statement.query([job_id])?;
    match rows.next()? {
        Some(row) => job_row(row, true),
        None => Err(BackendError::not_found("Задание не найдено")),
    }
}

/// Задания ДЗ без текстов (один запрос).
pub fn jobs_for_homework(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<AiJobDto>, AppError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM ai_jobs WHERE homework_id = ?1 ORDER BY created_at, id"
    ))?;
    let mut rows = statement.query([homework_id])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        output.push(job_row(row, false)?.0);
    }
    Ok(output)
}

pub fn log_entry(level: LogLevel, message: impl Into<String>) -> AiLogEntryDto {
    AiLogEntryDto {
        at: now(),
        level,
        message: message.into(),
    }
}

fn with_log(current: &str, entries: &[AiLogEntryDto]) -> Result<String, AppError> {
    let mut log: Vec<AiLogEntryDto> = parse_json(current, "ai_jobs.log_json")?;
    log.extend_from_slice(entries);
    if log.len() > MAX_LOG_ENTRIES {
        let drop = log.len() - MAX_LOG_ENTRIES;
        log.drain(..drop);
    }
    serde_json::to_string(&log).map_err(db_error)
}

fn current_log(connection: &Connection, job_id: &str) -> Result<String, AppError> {
    connection
        .query_row(
            "SELECT log_json FROM ai_jobs WHERE id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| BackendError::not_found("Задание не найдено"))
}

fn expect_one(changed: usize, message: &str) -> Result<(), AppError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(BackendError::conflict(message.to_owned()))
    }
}

pub fn append_log(
    connection: &Connection,
    job_id: &str,
    entries: &[AiLogEntryDto],
) -> Result<(), AppError> {
    let log = with_log(&current_log(connection, job_id)?, entries)?;
    connection.execute(
        "UPDATE ai_jobs SET log_json = ?2, updated_at = ?3
         WHERE id = ?1 AND status NOT IN ('confirmed', 'discarded', 'cancelled', 'failed')",
        params![job_id, log, now()],
    )?;
    Ok(())
}

/// prepared → running; запуск получает статус `running`.
pub fn mark_running(
    connection: &Connection,
    job_id: &str,
    run_id: &str,
    adapter_label: &str,
    entry: AiLogEntryDto,
) -> Result<(), AppError> {
    let log = with_log(&current_log(connection, job_id)?, &[entry])?;
    let at = now();
    let changed = connection
        .execute(
            "UPDATE ai_jobs SET status = 'running', adapter_label = ?2, sent_at = ?3, log_json = ?4, updated_at = ?3
             WHERE id = ?1 AND status = 'prepared' AND transport = 'cli'",
            params![job_id, adapter_label, at, log],
        )
        .map_err(|error| match error {
            rusqlite::Error::SqliteFailure(_, _) => {
                BackendError::conflict("У этого запуска уже выполняется другое задание")
            }
            other => other.into(),
        })?;
    expect_one(changed, "Задание уже отправлено или закрыто")?;
    set_run_progress(
        connection,
        run_id,
        "running",
        "Провайдер обрабатывает материалы",
    )?;
    Ok(())
}

/// Статус открытого запуска во время работы задания. Ревизия ДЗ не меняется:
/// это промежуточное состояние, а не решение пользователя.
pub fn set_run_progress(
    connection: &Connection,
    run_id: &str,
    status: &str,
    step: &str,
) -> Result<(), AppError> {
    connection.execute(
        "UPDATE review_runs SET status = ?2, step = ?3
         WHERE id = ?1 AND status IN ('queued', 'waiting_for_provider', 'manual_handoff', 'running')",
        params![run_id, status, step],
    )?;
    Ok(())
}

pub struct ResponseUpdate<'a> {
    pub text: &'a str,
    pub source: ResponseSource,
    pub file_name: Option<&'a str>,
    pub truncated: bool,
    pub parse_status: ParseStatus,
    pub draft: &'a Value,
    pub issues: &'a [AiIssueDto],
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub error: Option<(&'a str, &'a str)>,
    pub log: &'a [AiLogEntryDto],
}

/// Ответ получен (CLI или вручную): `response_ready`, ждёт подтверждения.
pub fn save_response(
    connection: &Connection,
    job_id: &str,
    update: &ResponseUpdate<'_>,
) -> Result<(), AppError> {
    use sha2::{Digest, Sha256};
    let sha = format!("{:x}", Sha256::digest(update.text.as_bytes()));
    let log = with_log(&current_log(connection, job_id)?, update.log)?;
    let at = now();
    let changed = connection.execute(
        "UPDATE ai_jobs SET status = 'response_ready', response_text = ?2, response_source = ?3,
                response_file_name = ?4, response_sha256 = ?5, response_truncated = ?6, parse_status = ?7,
                draft_json = ?8, issues_json = ?9, exit_code = ?10, duration_ms = ?11, error_code = ?12,
                error_message = ?13, log_json = ?14, finished_at = ?15, updated_at = ?15
         WHERE id = ?1 AND status IN ('prepared', 'running', 'response_ready')",
        params![
            job_id,
            update.text,
            update.source.as_db_str(),
            update.file_name,
            sha,
            update.truncated,
            update.parse_status.as_db_str(),
            serde_json::to_string(update.draft).map_err(db_error)?,
            serde_json::to_string(update.issues).map_err(db_error)?,
            update.exit_code,
            update.duration_ms.map(|value| value as i64),
            update.error.map(|(code, _)| code),
            update.error.map(|(_, message)| message),
            log,
            at,
        ],
    )?;
    expect_one(changed, "Задание уже закрыто — ответ не сохранён")
}

pub struct FailureUpdate<'a> {
    pub status: JobStatus,
    pub code: &'a str,
    pub message: &'a str,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub log: &'a [AiLogEntryDto],
}

/// Ошибка или отмена задания (терминальное состояние).
pub fn fail_job(
    connection: &Connection,
    job_id: &str,
    update: &FailureUpdate<'_>,
) -> Result<(), AppError> {
    let log = with_log(&current_log(connection, job_id)?, update.log)?;
    let at = now();
    let changed = connection.execute(
        "UPDATE ai_jobs SET status = ?2, error_code = ?3, error_message = ?4, exit_code = ?5,
                duration_ms = ?6, log_json = ?7, finished_at = ?8, updated_at = ?8
         WHERE id = ?1 AND status IN ('prepared', 'running', 'response_ready')",
        params![
            job_id,
            update.status.as_db_str(),
            update.code,
            update.message,
            update.exit_code,
            update.duration_ms.map(|value| value as i64),
            log,
            at,
        ],
    )?;
    expect_one(changed, "Задание уже закрыто")
}

pub fn discard_job(connection: &Connection, job_id: &str) -> Result<(), AppError> {
    let log = with_log(
        &current_log(connection, job_id)?,
        &[log_entry(LogLevel::Info, "Ответ отклонён пользователем")],
    )?;
    let at = now();
    let changed = connection.execute(
        "UPDATE ai_jobs SET status = 'discarded', log_json = ?2, updated_at = ?3, finished_at = coalesce(finished_at, ?3)
         WHERE id = ?1 AND status IN ('prepared', 'response_ready')",
        params![job_id, log, at],
    )?;
    expect_one(
        changed,
        "Отклонить можно только подготовленное задание или полученный ответ",
    )
}

/// Подтверждение пользователем: задание закрывается с отметкой о правке.
pub fn confirm_job(
    connection: &Connection,
    job_id: &str,
    edited: bool,
    prompt_id: Option<&str>,
) -> Result<(), AppError> {
    let log = with_log(
        &current_log(connection, job_id)?,
        &[log_entry(
            LogLevel::Info,
            if edited {
                "Результат подтверждён пользователем с правками"
            } else {
                "Результат подтверждён пользователем без правок"
            },
        )],
    )?;
    let at = now();
    let changed = connection.execute(
        "UPDATE ai_jobs SET status = 'confirmed', confirmed_at = ?2, edited = ?3, prompt_id = ?4,
                log_json = ?5, updated_at = ?2
         WHERE id = ?1 AND status = 'response_ready'",
        params![job_id, at, edited, prompt_id, log],
    )?;
    expect_one(changed, "Подтвердить можно только полученный ответ")
}

/// Остальные незакрытые задания запуска отклоняются, когда его результат сохранён.
pub fn close_other_jobs(
    connection: &Connection,
    run_id: &str,
    keep: &[&str],
) -> Result<(), AppError> {
    let mut statement = connection.prepare(
        "SELECT id FROM ai_jobs WHERE run_id = ?1 AND status IN ('prepared', 'response_ready')",
    )?;
    let ids: Vec<String> = statement
        .query_map([run_id], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for id in ids.iter().filter(|id| !keep.contains(&id.as_str())) {
        discard_job(connection, id)?;
    }
    Ok(())
}

pub fn running_job_for_run(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<String>, AppError> {
    Ok(connection
        .query_row(
            "SELECT id FROM ai_jobs WHERE run_id = ?1 AND status = 'running'",
            [run_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// После перезапуска приложения выполнявшиеся задания честно завершаются
/// ошибкой: процесс CLI уже не существует, результата нет.
pub fn recover_interrupted(connection: &Connection) -> Result<usize, AppError> {
    let mut statement =
        connection.prepare("SELECT id, run_id FROM ai_jobs WHERE status = 'running'")?;
    let rows: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (job_id, run_id) in &rows {
        fail_job(
            connection,
            job_id,
            &FailureUpdate {
                status: JobStatus::Failed,
                code: "interrupted",
                message: "Приложение было закрыто во время запуска — результат не получен",
                exit_code: None,
                duration_ms: None,
                log: &[log_entry(
                    LogLevel::Error,
                    "Запуск прерван закрытием приложения",
                )],
            },
        )?;
        set_run_progress(
            connection,
            run_id,
            "waiting_for_provider",
            "Запуск прерван — подготовьте передачу заново",
        )?;
    }
    Ok(rows.len())
}
