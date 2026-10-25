//! Модель этапа 9: задачи AI, способы передачи, статусы заданий и DTO.
//!
//! Бизнес-логика опирается на `AiTask`, а не на исторические имена этапов
//! `codex_analysis`/`claude_solving`: провайдер каждой задачи назначается
//! отдельно в настройках рабочего цикла.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::isolation::{IsolationCapability, IsolationReport, ProviderExecutionPolicy};
use crate::dto::ProviderId;

macro_rules! ai_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $value)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_db_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub fn try_from_db(value: &str) -> Result<Self, String> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    other => Err(format!(concat!("unknown ", stringify!($name), ": {}"), other)),
                }
            }
        }
    };
}

ai_enum!(
    /// Задача, которую можно поручить провайдеру. Провайдер назначается каждой
    /// задаче отдельно (`AutomationRules.workflow.tasks`).
    AiTask {
        MaterialAnalysis => "material_analysis",
        InitialPrompt => "initial_prompt",
        SolutionReview => "solution_review",
        RevisionPrompt => "revision_prompt",
        TeacherReview => "teacher_review",
    }
);

impl AiTask {
    pub fn label(self) -> &'static str {
        match self {
            Self::MaterialAnalysis => "Анализ материалов",
            Self::InitialPrompt => "Первичный промт",
            Self::SolutionReview => "Проверка решения",
            Self::RevisionPrompt => "Промт исправлений",
            Self::TeacherReview => "Проверка по комментариям преподавателя",
        }
    }
}

ai_enum!(
    /// Как материалы попадают к провайдеру: официальный CLI (после явного
    /// подключения пользователем) или ручная передача через буфер обмена.
    Transport {
        Cli => "cli",
        ManualHandoff => "manual_handoff",
    }
);

ai_enum!(
    /// Жизненный цикл задания. `response_ready` — ответ получен и ждёт
    /// проверки пользователем; применяется только после `confirmed`.
    JobStatus {
        Prepared => "prepared",
        Running => "running",
        ResponseReady => "response_ready",
        Failed => "failed",
        Cancelled => "cancelled",
        Confirmed => "confirmed",
        Discarded => "discarded",
    }
);

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Cancelled | Self::Confirmed | Self::Discarded
        )
    }
}

ai_enum!(
    /// Результат разбора ответа. `partial` — часть полей распознана,
    /// `corrupt` — структурированного ответа нет (сырой текст сохранён).
    ParseStatus {
        Valid => "valid",
        Partial => "partial",
        Corrupt => "corrupt",
        Empty => "empty",
    }
);

ai_enum!(ResponseSource {
    Cli => "cli",
    Paste => "paste",
    File => "file",
});

ai_enum!(
    /// Сводное состояние провайдера для UI.
    ProviderAvailability {
        Connected => "connected",
        Manual => "manual",
        Unavailable => "unavailable",
        Error => "error",
    }
);

pub fn provider_db_str(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Claude => "claude",
    }
}

pub fn provider_from_db(value: &str) -> Result<ProviderId, String> {
    match value {
        "codex" => Ok(ProviderId::Codex),
        "claude" => Ok(ProviderId::Claude),
        other => Err(format!("unknown provider: {other}")),
    }
}

pub fn provider_label(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "Codex",
        ProviderId::Claude => "Claude",
    }
}

// ───────────────────────── DTO ─────────────────────────

/// Источник, вошедший в передаваемый текст. Путей нет — только имя и объём.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextSourceDto {
    /// Метка в тексте промта: `S1`, `S2`…
    pub label: String,
    pub source_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub sha256: Option<String>,
    pub chars_total: u64,
    pub chars_included: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiIssueDto {
    pub code: String,
    pub message: String,
}

impl AiIssueDto {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Строка журнала запуска. Сообщения проходят редакцию секретов и путей.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiLogEntryDto {
    pub at: String,
    pub level: LogLevel,
    pub message: String,
}

/// Задание передачи материалов провайдеру. В списках (`full = false`) текст
/// промта, ответ и черновик не передаются.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiJobDto {
    pub id: String,
    pub homework_id: String,
    pub run_id: String,
    pub task: AiTask,
    pub provider: ProviderId,
    pub transport: Transport,
    pub adapter_label: Option<String>,
    pub status: JobStatus,
    pub prompt_text: Option<String>,
    pub prompt_chars: u64,
    pub prompt_sha256: String,
    pub context: Vec<ContextSourceDto>,
    pub context_truncated: bool,
    pub limit_chars: u64,
    pub response_text: Option<String>,
    pub response_chars: u64,
    pub response_source: Option<ResponseSource>,
    pub response_file_name: Option<String>,
    pub response_truncated: bool,
    pub parse_status: Option<ParseStatus>,
    pub draft: Option<Value>,
    pub issues: Vec<AiIssueDto>,
    pub log: Vec<AiLogEntryDto>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub created_at: String,
    pub sent_at: Option<String>,
    pub finished_at: Option<String>,
    pub confirmed_at: Option<String>,
    pub edited: Option<bool>,
    pub prompt_id: Option<String>,
}

/// Состояние официального CLI провайдера. Абсолютных путей нет.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCliDto {
    /// Пользователь явно подключил CLI.
    pub enabled: bool,
    /// Исполняемый файл найден в разрешённых каталогах.
    pub detected: bool,
    /// Отображаемое имя (`codex`, `~/.local/bin/claude`) — без домашнего каталога.
    pub executable: Option<String>,
    pub version: Option<String>,
    /// Версия поддерживает безопасный неинтерактивный запуск.
    pub supported: Option<bool>,
    pub missing_flags: Vec<String>,
    /// Вход в официальном CLI выполнен (`null` — не проверялось или неизвестно).
    pub logged_in: Option<bool>,
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHandoffDto {
    /// Официальное приложение установлено в /Applications.
    pub app_installed: bool,
    pub web_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderIsolationDto {
    pub capability: IsolationCapability,
    pub report: Option<IsolationReport>,
    pub consent_profile_version: Option<u32>,
    pub consent_valid: bool,
    pub execution_policy: ProviderExecutionPolicy,
    pub blocking_reason: Option<String>,
}
