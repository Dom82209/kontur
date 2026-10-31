//! Точные DTO рабочего цикла ДЗ (camelCase) и их enum-преобразования в
//! значения SQLite. Каждый enum имеет исчерпывающие `as_db_str`/`try_from_db`:
//! неизвестное значение из базы — структурированная ошибка `database`, а не
//! молчаливый default.

use serde::{Deserialize, Serialize};

use crate::{
    domain::{HomeworkWorkflowStage, VersionAuthor},
    dto::{FileRefDto, HomeworkDto, InboxFileDto, ProviderId},
    extraction::MaterialRef,
};

macro_rules! db_enum {
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

db_enum!(SourceKind {
    Lecture => "lecture",
    Seminar => "seminar",
    Task => "task",
    Data => "data",
    Extra => "extra",
    Style => "style",
    PreviousWork => "previous_work",
    CurrentVersion => "current_version",
    PreviousVersion => "previous_version",
    TeacherComments => "teacher_comments",
});

db_enum!(RunKind {
    AnalyzeMaterials => "analyze_materials",
    ReviewVersion => "review_version",
    RevisionPrompt => "revision_prompt",
    ReviewTeacher => "review_teacher",
});

db_enum!(WorkflowRole {
    Analyst => "analyst",
    Author => "author",
    Reviewer => "reviewer",
});

db_enum!(
    /// Честные состояния запуска. `waiting_for_provider` — роль назначена
    /// провайдеру, но адаптера ещё нет; `manual_handoff` — пользователь
    /// выполняет шаг вручную и сохраняет результат с provenance `manual`.
    ReviewRunStatus {
        Queued => "queued",
        WaitingForProvider => "waiting_for_provider",
        ManualHandoff => "manual_handoff",
        Running => "running",
        Completed => "completed",
        Failed => "failed",
        Cancelled => "cancelled",
    }
);

impl ReviewRunStatus {
    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::WaitingForProvider | Self::ManualHandoff | Self::Running
        )
    }
}

db_enum!(
    /// Происхождение результата: `manual` — сохранён пользователем, `provider` —
    /// получен от AI-адаптера (этап 9), `legacy` — строка схемы v1–v5.
    Provenance {
        Legacy => "legacy",
        Manual => "manual",
        Provider => "provider",
    }
);

db_enum!(CriterionId {
    Correctness => "correctness",
    TaskFit => "task_fit",
    CourseGrounding => "course_grounding",
    Style => "style",
    Teacher => "teacher",
    Completeness => "completeness",
});

db_enum!(CriterionStatus {
    Pass => "pass",
    Warn => "warn",
    Fail => "fail",
    Manual => "manual",
    Na => "na",
});

db_enum!(FindingSeverity {
    Critical => "critical",
    Normal => "normal",
});

db_enum!(ReviewVerdict {
    Pass => "pass",
    NeedsFixes => "needs_fixes",
    NeedsTeacherReview => "needs_teacher_review",
    Blocked => "blocked",
});

db_enum!(RecommendedActionKind {
    SendPromptToAuthor => "send_prompt_to_author",
    CreateRevisionPrompt => "create_revision_prompt",
    AddTeacherComments => "add_teacher_comments",
    MarkReady => "mark_ready",
    CompleteMaterials => "complete_materials",
});

db_enum!(StyleReasonKind {
    Template => "template",
    UncharacteristicWords => "uncharacteristic_words",
    FormalTone => "formal_tone",
    Repetition => "repetition",
    Unsupported => "unsupported",
});

db_enum!(PromptKind {
    Initial => "initial",
    Revision => "revision",
    Teacher => "teacher",
});

db_enum!(PromptStatus {
    Draft => "draft",
    Committed => "committed",
});

db_enum!(TeacherCommentSource {
    Chat => "chat",
    Email => "email",
    Lms => "lms",
    InPerson => "in_person",
    Other => "other",
});

db_enum!(BundleOrigin {
    Legacy => "legacy",
    CorpusSelection => "corpus_selection",
});

db_enum!(MissingItemKind {
    File => "file",
    Link => "link",
});

db_enum!(PackageItemRole {
    Main => "main",
    Attachment => "attachment",
    Extra => "extra",
    Cloud => "cloud",
});

db_enum!(PackageCheckId {
    FinalSelected => "final_selected",
    NameChecked => "name_checked",
    Attachments => "attachments",
    Links => "links",
    Teacher => "teacher",
    ManualReview => "manual_review",
});

db_enum!(ReadinessCheckId {
    Materials => "materials",
    ReviewCurrent => "review_current",
    ReviewPassed => "review_passed",
    NoCritical => "no_critical",
    TaskCoverage => "task_coverage",
    Teacher => "teacher",
    Attachments => "attachments",
    Style => "style",
    SourcesFresh => "sources_fresh",
    FinalVersion => "final_version",
});

db_enum!(IterationStatus {
    Draft => "draft",
    Reviewing => "reviewing",
    Failed => "failed",
    Passed => "passed",
    Final => "final",
    Submitted => "submitted",
});

db_enum!(SectionDiff {
    Same => "same",
    Changed => "changed",
    Added => "added",
    Removed => "removed",
});

// ───────────── Источники ─────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceItemDto {
    pub id: String,
    pub kind: SourceKind,
    pub file: FileRefDto,
    pub required: bool,
    pub included: bool,
    pub material: MaterialRef,
    /// SHA-256 содержимого в момент снимка (`null` только у legacy-строк).
    pub sha256: Option<String>,
    pub extraction_id: Option<String>,
    pub extractor_version: Option<u32>,
    pub text_chars: u64,
    /// Коды предупреждений извлечения на момент снимка.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissingSourceDto {
    pub kind: SourceKind,
    pub label: String,
    pub required: bool,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleWarningDto {
    pub code: String,
    pub message: String,
    pub material_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBundleDto {
    pub id: String,
    pub homework_id: String,
    pub purpose: RunKind,
    pub version_id: Option<String>,
    pub items: Vec<SourceItemDto>,
    pub missing: Vec<MissingSourceDto>,
    pub complete: bool,
    pub created_at: String,
    pub origin: BundleOrigin,
    pub selection_id: Option<String>,
    pub extractor_version: Option<u32>,
    pub warnings: Vec<BundleWarningDto>,
    pub total_chars: u64,
    pub limit_chars: u64,
}

// ───────────── Проверка ─────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceReferenceDto {
    pub source_id: String,
    pub file_name: String,
    pub locator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewCriterionDto {
    pub id: CriterionId,
    pub label: String,
    pub status: CriterionStatus,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewFindingDto {
    pub id: String,
    pub severity: FindingSeverity,
    pub criterion: CriterionId,
    pub text: String,
    pub location: Option<String>,
    pub evidence: Vec<EvidenceReferenceDto>,
    pub introduced_in: String,
    pub fixed_in: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsupportedClaimDto {
    pub id: String,
    pub text: String,
    pub location: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleReasonDto {
    pub kind: StyleReasonKind,
    pub text: String,
    pub example: Option<String>,
}

/// Расхождение с личным стилем 0…10. Это эвристика сравнения с профилем
/// стиля, а не определение авторства текста.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleDivergenceDto {
    pub score: u8,
    pub threshold: u8,
    pub reasons: Vec<StyleReasonDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecommendedActionDto {
    pub kind: RecommendedActionKind,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewMissingItemDto {
    pub kind: MissingItemKind,
    pub label: String,
    pub required: bool,
}

/// Сохранённая часть результата (`review_runs.result_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredReviewResult {
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub criteria: Vec<ReviewCriterionDto>,
    pub critical: Vec<ReviewFindingDto>,
    pub remarks: Vec<ReviewFindingDto>,
    pub unsupported_claims: Vec<UnsupportedClaimDto>,
    pub style: Option<StyleDivergenceDto>,
    pub teacher_requirements: Vec<TeacherRequirementDto>,
    pub recommended_action: RecommendedActionDto,
    #[serde(default)]
    pub missing: Vec<ReviewMissingItemDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResultDto {
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub criteria: Vec<ReviewCriterionDto>,
    pub critical: Vec<ReviewFindingDto>,
    pub remarks: Vec<ReviewFindingDto>,
    pub unsupported_claims: Vec<UnsupportedClaimDto>,
    pub style: Option<StyleDivergenceDto>,
    pub teacher_requirements: Vec<TeacherRequirementDto>,
    pub recommended_action: RecommendedActionDto,
    pub missing: Vec<ReviewMissingItemDto>,
    pub prompt: Option<RevisionPromptDto>,
}

impl ReviewResultDto {
    pub fn from_stored(stored: StoredReviewResult, prompt: Option<RevisionPromptDto>) -> Self {
        Self {
            verdict: stored.verdict,
            summary: stored.summary,
            criteria: stored.criteria,
            critical: stored.critical,
            remarks: stored.remarks,
            unsupported_claims: stored.unsupported_claims,
            style: stored.style,
            teacher_requirements: stored.teacher_requirements,
            recommended_action: stored.recommended_action,
            missing: stored.missing,
            prompt,
        }
    }
}

/// Отдельно сохранённые критерии запуска (колонки `review_runs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAssessmentDto {
    pub verdict: ReviewVerdict,
    pub correctness: CriterionStatus,
    pub task_fit: CriterionStatus,
    pub course_grounding: CriterionStatus,
    pub teacher: CriterionStatus,
    pub style_score: Option<u8>,
    pub critical_count: u32,
    pub unsupported_count: u32,
    pub missing_required: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRunDto {
    pub id: String,
    pub homework_id: String,
    pub kind: RunKind,
    pub role: WorkflowRole,
    pub provider: ProviderId,
    pub version_id: Option<String>,
    /// `null` только у legacy-запусков схемы v1.
    pub source_bundle_id: Option<String>,
    pub sources_used: Vec<EvidenceReferenceDto>,
    pub status: ReviewRunStatus,
    pub progress: u8,
    pub step: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub saved_as_iteration: bool,
    pub result: Option<ReviewResultDto>,
    pub error_message: Option<String>,
    pub provenance: Provenance,
    /// Порог, применённый в момент запуска (не меняется вместе с настройкой).
    pub style_threshold: u8,
    pub assessment: Option<ReviewAssessmentDto>,
}

// ───────────── Промты ─────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPromptDto {
    pub id: String,
    pub homework_id: String,
    pub kind: PromptKind,
    pub text: String,
    pub created_at: String,
    pub created_by: VersionAuthor,
    pub based_on_version_id: Option<String>,
    pub review_run_id: Option<String>,
    pub addresses: Vec<String>,
    pub sources: Vec<EvidenceReferenceDto>,
    pub sent_at: Option<String>,
    pub status: PromptStatus,
    pub committed_at: Option<String>,
    pub provenance: Provenance,
    pub updated_at: String,
}

// ───────────── Преподаватель ─────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeacherCommentDto {
    pub id: String,
    pub homework_id: String,
    pub source: TeacherCommentSource,
    pub date: String,
    pub question: String,
    pub answer: String,
    pub added_at: String,
    pub attachment: Option<FileRefDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TeacherRequirementDto {
    pub id: String,
    pub text: String,
    /// `null` только у legacy-строк v1 или после удаления комментария.
    pub comment_id: Option<String>,
    pub origin: String,
    pub status: crate::domain::RequirementStatus,
    pub note: String,
    pub checked_version_id: Option<String>,
}

// ───────────── Подтверждение стиля ─────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleConfirmationDto {
    pub id: String,
    pub homework_id: String,
    pub version_id: String,
    pub review_run_id: String,
    pub score: u8,
    pub threshold: u8,
    /// Всегда `user`: подтверждение ставит только пользователь.
    pub confirmed_by: &'static str,
    pub confirmed_at: String,
}

// ───────────── Комплект и снимок (только чтение на этапе 8) ─────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageItemDto {
    pub id: String,
    pub role: PackageItemRole,
    pub file: Option<FileRefDto>,
    pub final_name: String,
    pub required: bool,
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageLinkDto {
    pub id: String,
    pub label: String,
    pub url: String,
    pub required: bool,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCheckDto {
    pub id: PackageCheckId,
    pub label: String,
    pub done: bool,
    pub auto: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionPackageDto {
    pub id: String,
    pub homework_id: String,
    pub version_id: String,
    pub folder_path: String,
    pub items: Vec<PackageItemDto>,
    pub links: Vec<PackageLinkDto>,
    pub warnings: Vec<String>,
    pub checklist: Vec<PackageCheckDto>,
    pub upload_list: Vec<String>,
    pub built_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionReceiptDto {
    pub id: String,
    pub homework_id: String,
    pub version_id: String,
    pub package_id: String,
    pub submitted_at: String,
    pub confirmed_at: String,
    pub assignment_url: String,
    pub proof: Option<FileRefDto>,
    pub snapshot_path: String,
    pub files: Vec<FileRefDto>,
    pub snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleProfileDto {
    pub id: String,
    pub file_name: String,
    pub relative_path: String,
    pub updated_at: String,
    pub traits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmissionProofInput {
    pub name: String,
    pub size_bytes: u64,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmissionConfirmationInput {
    pub submitted_at: String,
    pub assignment_url: String,
    pub proof: Option<SubmissionProofInput>,
    pub user_confirmed: bool,
}

// ───────────── Проекции ─────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessCheckDto {
    pub id: ReadinessCheckId,
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessDto {
    pub homework_id: String,
    pub ready: bool,
    pub checks: Vec<ReadinessCheckDto>,
    /// Проверяемая (текущая) версия и запуск, по которому построен ответ.
    pub version_id: Option<String>,
    pub review_run_id: Option<String>,
    pub revision: i64,
}

impl ReadinessDto {
    pub fn blockers(&self) -> Vec<ReadinessCheckId> {
        self.checks
            .iter()
            .filter(|check| !check.ok)
            .map(|check| check.id)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkIterationDto {
    pub version_id: String,
    pub label: String,
    pub author: VersionAuthor,
    pub created_at: String,
    pub files: Vec<FileRefDto>,
    pub prompt: Option<RevisionPromptDto>,
    pub source_bundle_id: Option<String>,
    pub review: Option<ReviewRunDto>,
    pub teacher_review: Option<ReviewRunDto>,
    pub revision_prompt: Option<RevisionPromptDto>,
    pub next_version_id: Option<String>,
    pub fixed_findings: Vec<ReviewFindingDto>,
    pub open_findings: Vec<ReviewFindingDto>,
    pub status: IterationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionComparisonDto {
    pub title: String,
    pub diff: SectionDiff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleComparisonDto {
    pub from: Option<u8>,
    pub to: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundingComparisonDto {
    pub from: Option<CriterionStatus>,
    pub to: Option<CriterionStatus>,
    pub unsupported_from: u32,
    pub unsupported_to: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoneTotalDto {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeacherComparisonDto {
    pub from: Option<DoneTotalDto>,
    pub to: Option<DoneTotalDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionComparisonDto {
    pub from_version_id: String,
    pub to_version_id: String,
    pub fixed: Vec<ReviewFindingDto>,
    pub introduced: Vec<ReviewFindingDto>,
    pub sections: Vec<SectionComparisonDto>,
    pub style: StyleComparisonDto,
    pub grounding: GroundingComparisonDto,
    pub teacher: TeacherComparisonDto,
}

// ───────────── Главная ─────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcademicWeekDto {
    pub number: i64,
    pub starts_at: String,
    pub ends_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardInboxDto {
    pub total: u32,
    pub ambiguous: u32,
    pub preview: Vec<InboxFileDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlineDto {
    pub homework_id: String,
    pub subject_id: String,
    pub title: String,
    pub deadline: String,
    pub stage: crate::domain::HomeworkStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationState {
    Active,
    Paused,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardAutomationDto {
    pub state: AutomationState,
    /// Только отображаемая метка, без абсолютного пути.
    pub watched_path: String,
    pub last_run_at: String,
    pub pending: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStateDto {
    Disconnected,
    Connecting,
    Connected,
    Error,
    ReauthRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAdapterDto {
    /// Официальный CLI, явно подключённый пользователем.
    Cli,
    DesktopApp,
    Mock,
    /// Ручная передача: промт копируется пользователем, ответ вставляется.
    ManualHandoff,
    /// CLI подключён, но сейчас недоступен (удалён, не поддерживается).
    Unavailable,
}

/// Runtime-проекция состояния провайдера (этап 9). Не содержит секретов,
/// аккаунта и абсолютных путей.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusDto {
    pub provider: ProviderId,
    pub state: ProviderStateDto,
    /// Отображаемая метка клиента («Codex CLI · 0.47.0»), никогда не аккаунт и не токен.
    pub account_label: Option<String>,
    pub last_checked_at: String,
    pub error_message: Option<String>,
    pub adapter: ProviderAdapterDto,
    pub roles: Vec<WorkflowRole>,
    pub availability: crate::ai::model::ProviderAvailability,
    pub tasks: Vec<crate::ai::model::AiTask>,
    pub cli: crate::ai::model::ProviderCliDto,
    pub isolation: crate::ai::model::ProviderIsolationDto,
    pub handoff: crate::ai::model::ProviderHandoffDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekProgressItemDto {
    pub label: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekProgressDto {
    pub done: u32,
    pub total: u32,
    pub items: Vec<WeekProgressItemDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDto {
    pub today: String,
    /// `None` — семестр не настроен или сейчас вне семестра.
    pub academic_week: Option<AcademicWeekDto>,
    pub inbox: DashboardInboxDto,
    pub active_homework: Vec<HomeworkDto>,
    pub deadlines: Vec<DeadlineDto>,
    pub recent_files: Vec<FileRefDto>,
    pub automation: DashboardAutomationDto,
    pub providers: Vec<ProviderStatusDto>,
    pub week_progress: WeekProgressDto,
}

// ───────────── Агрегат рабочего цикла ─────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionOptionDto {
    pub to: HomeworkWorkflowStage,
    pub allowed: bool,
    /// Почему переход недоступен (или что он сделает).
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkWorkflowDto {
    pub homework: HomeworkDto,
    pub revision: i64,
    pub stage: HomeworkWorkflowStage,
    pub transitions: Vec<TransitionOptionDto>,
    pub bundles: Vec<SourceBundleDto>,
    pub runs: Vec<ReviewRunDto>,
    pub prompts: Vec<RevisionPromptDto>,
    pub teacher_comments: Vec<TeacherCommentDto>,
    pub teacher_requirements: Vec<TeacherRequirementDto>,
    pub style_confirmations: Vec<StyleConfirmationDto>,
    pub readiness: ReadinessDto,
    pub iterations: Vec<HomeworkIterationDto>,
    pub submission_package: Option<SubmissionPackageDto>,
    pub submission_receipt: Option<SubmissionReceiptDto>,
    /// Задания передачи провайдерам (этап 9) — без текстов промта и ответа.
    pub ai_jobs: Vec<crate::ai::model::AiJobDto>,
    /// Этап 9: AI-шаги выполняются через задания передачи (официальный CLI
    /// после явного подключения или ручная передача) с подтверждением результата.
    pub provider_mode: ProviderModeDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderModeDto {
    AiJobs,
}
