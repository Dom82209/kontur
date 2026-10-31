use serde::{Deserialize, Serialize};

use crate::domain::{HomeworkStage, HomeworkWorkflowStage, OperationStatus, VersionAuthor};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectDto {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub current_week: i64,
    pub week_count: i64,
    /// Необязательная ссылка на курс в LMS (allowlist HTTPS-домена).
    pub course_url: Option<String>,
    /// Папка предмета в vault (относительный путь). Не меняется при переименовании.
    pub folder_path: String,
    /// Ключевые слова автоматической классификации (нижний регистр).
    pub keywords: Vec<String>,
    /// Архивный предмет не предлагается в классификации; данные сохраняются.
    pub archived: bool,
    /// Предмет слит в другой (всегда архивный).
    pub merged_into_id: Option<String>,
    /// Optimistic revision: меняется при каждом изменении предмета.
    pub revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Pdf,
    Doc,
    Sheet,
    Slides,
    Image,
    Notebook,
    Text,
    Archive,
    Other,
}

impl FileKind {
    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "pdf" => Ok(Self::Pdf),
            "doc" => Ok(Self::Doc),
            "sheet" => Ok(Self::Sheet),
            "slides" => Ok(Self::Slides),
            "image" => Ok(Self::Image),
            "notebook" => Ok(Self::Notebook),
            "text" => Ok(Self::Text),
            "archive" => Ok(Self::Archive),
            "other" => Ok(Self::Other),
            _ => Err(format!("unknown file kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRefDto {
    pub id: String,
    pub name: String,
    pub kind: FileKind,
    pub size_bytes: i64,
    pub modified_at: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekDto {
    pub id: String,
    pub subject_id: String,
    pub number: i64,
    /// `None` — даты недели неизвестны до настройки семестра.
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub topic: Option<String>,
    pub lectures: Vec<FileRefDto>,
    pub seminars: Vec<FileRefDto>,
    pub materials: Vec<FileRefDto>,
    /// Этап 4: все ДЗ недели в порядке `ordinal` (0..N).
    pub homework_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialCategory {
    Lecture,
    Seminar,
    Material,
    HomeworkTask,
    HomeworkData,
    HomeworkDraft,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationSource {
    Auto,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationDto {
    pub subject_id: Option<String>,
    pub week_number: Option<i64>,
    pub category: MaterialCategory,
    pub confidence: f64,
    pub reasons: Vec<String>,
    pub source: ClassificationSource,
    /// Этап 4: ДЗ недели для файла ДЗ (ручной выбор, если в неделе несколько ДЗ).
    #[serde(default)]
    pub homework_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateInfoDto {
    pub existing_path: String,
    pub same_content: bool,
    pub suggested_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxOrigin {
    Downloads,
    Drop,
    Lms,
}

impl InboxOrigin {
    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "downloads" => Ok(Self::Downloads),
            "drop" => Ok(Self::Drop),
            "lms" => Ok(Self::Lms),
            _ => Err(format!("unknown inbox origin: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxStatus {
    New,
    Classifying,
    Classified,
    Ambiguous,
    Routed,
}

impl InboxStatus {
    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "new" => Ok(Self::New),
            "classifying" => Ok(Self::Classifying),
            "classified" => Ok(Self::Classified),
            "ambiguous" => Ok(Self::Ambiguous),
            "routed" => Ok(Self::Routed),
            _ => Err(format!("unknown inbox status: {value}")),
        }
    }
}

/// Откуда файл попал во «Входящие».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSource {
    /// Системный выбор файлов или перетаскивание.
    Manual,
    /// Обнаружен watcher папки загрузок.
    Watcher,
    /// Запись экспорта CU LMS Enhancer.
    CuExport,
}

/// Как пользователь передал файл или архив.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceVia {
    Picker,
    Drop,
    Watcher,
}

/// Происхождение без абсолютных путей: имя архива и относительный путь внутри него.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvenanceDto {
    pub source: ProvenanceSource,
    pub via: ProvenanceVia,
    pub archive_name: Option<String>,
    pub entry_path: Option<String>,
    pub course_url: Option<String>,
    pub assignment_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxFileDto {
    pub id: String,
    pub name: String,
    pub kind: FileKind,
    pub size_bytes: i64,
    pub added_at: String,
    pub origin: InboxOrigin,
    pub status: InboxStatus,
    pub classification: Option<ClassificationDto>,
    pub duplicate: Option<DuplicateInfoDto>,
    /// `null` у файлов, импортированных до этапа 6.
    #[serde(default)]
    pub provenance: Option<ProvenanceDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteDestinationDto {
    pub subject_id: String,
    pub week_number: i64,
    pub category: MaterialCategory,
    /// Этап 4: ДЗ недели для файлов ДЗ. Обязателен, если в неделе несколько ДЗ.
    #[serde(default)]
    pub homework_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteActionDto {
    Copy,
    Skip,
    Rename,
}

/// Полный read-only план одной файловой операции. Он не содержит абсолютных путей.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePreviewDto {
    pub file_id: String,
    pub source_name: String,
    pub source_relative_path: String,
    pub target_path: String,
    pub target_name: String,
    pub creates_week: bool,
    pub creates_directories: Vec<String>,
    pub sha256_match: bool,
    pub name_conflict: bool,
    pub renamed: bool,
    pub action: RouteActionDto,
    pub undoable: bool,
    pub duplicate: Option<DuplicateInfoDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSectionDto {
    pub id: String,
    pub title: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkVersionDto {
    pub id: String,
    pub homework_id: String,
    pub number: i64,
    pub label: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub created_at: String,
    pub source_file_id: String,
    pub note: Option<String>,
    pub sections: Vec<VersionSectionDto>,
    pub author: VersionAuthor,
    pub prompt_id: Option<String>,
    pub source_bundle_id: Option<String>,
    pub files: Vec<FileRefDto>,
    pub frozen: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkDto {
    pub id: String,
    pub subject_id: String,
    pub week_number: i64,
    /// Порядковый номер ДЗ в неделе (1..N).
    pub ordinal: i64,
    pub title: String,
    /// `None` — срок неизвестен (не выдумывается).
    pub deadline: Option<String>,
    pub stage: HomeworkStage,
    pub task_files: Vec<FileRefDto>,
    pub versions: Vec<HomeworkVersionDto>,
    pub final_version_id: Option<String>,
    pub final_file_name: Option<String>,
    pub submitted_at: Option<String>,
    pub folder_path: String,
    pub workflow_stage: HomeworkWorkflowStage,
    pub assignment_url: Option<String>,
    /// Optimistic revision агрегата: меняется при каждой мутации рабочего цикла.
    pub revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Classify,
    Route,
    CreateWeek,
    CreateVersion,
    MarkFinal,
    MarkSubmitted,
    PrepareSources,
    AiRun,
    RevisionPrompt,
    SendPrompt,
    SaveTeacherComments,
    BuildPackage,
    MarkReady,
    ConfirmSubmission,
    UpdateRules,
    Import,
    /// Этап 7: пакетное извлечение текста из материалов.
    Extract,
    /// Этап 8: явный переход этапа рабочего цикла пользователем.
    WorkflowTransition,
    /// Этап 8: подтверждение расхождения с личным стилем выше порога.
    ConfirmStyle,
    Undo,
}

impl OperationKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Classify => "classify",
            Self::Route => "route",
            Self::CreateWeek => "create_week",
            Self::CreateVersion => "create_version",
            Self::MarkFinal => "mark_final",
            Self::MarkSubmitted => "mark_submitted",
            Self::PrepareSources => "prepare_sources",
            Self::AiRun => "ai_run",
            Self::RevisionPrompt => "revision_prompt",
            Self::SendPrompt => "send_prompt",
            Self::SaveTeacherComments => "save_teacher_comments",
            Self::BuildPackage => "build_package",
            Self::MarkReady => "mark_ready",
            Self::ConfirmSubmission => "confirm_submission",
            Self::UpdateRules => "update_rules",
            Self::Import => "import",
            Self::Extract => "extract",
            Self::WorkflowTransition => "workflow_transition",
            Self::ConfirmStyle => "confirm_style",
            Self::Undo => "undo",
        }
    }

    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "classify" => Ok(Self::Classify),
            "route" => Ok(Self::Route),
            "create_week" => Ok(Self::CreateWeek),
            "create_version" => Ok(Self::CreateVersion),
            "mark_final" => Ok(Self::MarkFinal),
            "mark_submitted" => Ok(Self::MarkSubmitted),
            "prepare_sources" => Ok(Self::PrepareSources),
            "ai_run" => Ok(Self::AiRun),
            "revision_prompt" => Ok(Self::RevisionPrompt),
            "send_prompt" => Ok(Self::SendPrompt),
            "save_teacher_comments" => Ok(Self::SaveTeacherComments),
            "build_package" => Ok(Self::BuildPackage),
            "mark_ready" => Ok(Self::MarkReady),
            "confirm_submission" => Ok(Self::ConfirmSubmission),
            "update_rules" => Ok(Self::UpdateRules),
            "import" => Ok(Self::Import),
            "extract" => Ok(Self::Extract),
            "workflow_transition" => Ok(Self::WorkflowTransition),
            "confirm_style" => Ok(Self::ConfirmStyle),
            "undo" => Ok(Self::Undo),
            _ => Err(format!("unknown operation kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationDto {
    pub id: String,
    pub kind: OperationKind,
    pub title: String,
    pub detail: Option<String>,
    pub status: OperationStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub undoable: bool,
    pub paths: Vec<String>,
    pub backup_path: Option<String>,
    pub homework_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeekDetection {
    Calendar,
    FilenameThenCalendar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StudentNameDto {
    pub last_name: String,
    pub first_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubjectRuleDto {
    pub subject_id: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmationRulesDto {
    pub low_confidence_route: bool,
    pub mark_final: bool,
    pub mark_submitted: bool,
    pub allow_overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRolesDto {
    pub analyst: ProviderId,
    pub author: ProviderId,
    pub reviewer: ProviderId,
}

/// Провайдер каждой AI-задачи (этап 9). Задачи настраиваются независимо от
/// исторических имён этапов `codex_analysis`/`claude_solving`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiTaskProvidersDto {
    pub material_analysis: ProviderId,
    pub initial_prompt: ProviderId,
    pub solution_review: ProviderId,
    pub revision_prompt: ProviderId,
    pub teacher_review: ProviderId,
}

impl AiTaskProvidersDto {
    /// Для настроек, сохранённых до этапа 9: задачи наследуют прежние роли.
    pub fn from_roles(roles: &WorkflowRolesDto) -> Self {
        Self {
            material_analysis: roles.analyst,
            initial_prompt: roles.analyst,
            solution_review: roles.reviewer,
            revision_prompt: roles.reviewer,
            teacher_review: roles.reviewer,
        }
    }

    pub fn provider(&self, task: crate::ai::model::AiTask) -> ProviderId {
        use crate::ai::model::AiTask;
        match task {
            AiTask::MaterialAnalysis => self.material_analysis,
            AiTask::InitialPrompt => self.initial_prompt,
            AiTask::SolutionReview => self.solution_review,
            AiTask::RevisionPrompt => self.revision_prompt,
            AiTask::TeacherReview => self.teacher_review,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowSettingsDto {
    pub roles: WorkflowRolesDto,
    pub tasks: AiTaskProvidersDto,
    pub style_threshold: i64,
    pub style_profile_path: String,
    pub require_teacher_review: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationRulesDto {
    pub inbox_path: String,
    pub downloads_watch_path: String,
    pub obsidian_vault_path: String,
    pub student_name: StudentNameDto,
    pub final_name_pattern: String,
    pub version_pattern: String,
    pub week_detection: WeekDetection,
    pub semester_start: String,
    pub subject_rules: Vec<SubjectRuleDto>,
    pub low_confidence_threshold: f64,
    pub backup_before_change: bool,
    pub auto_route_high_confidence: bool,
    pub confirm: ConfirmationRulesDto,
    pub workflow: WorkflowSettingsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum PatchField<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassificationPatchDto {
    pub subject_id: PatchField<String>,
    pub week_number: PatchField<i64>,
    pub category: PatchField<MaterialCategory>,
    #[serde(default)]
    pub homework_id: PatchField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResultDto<T> {
    pub operation: OperationDto,
    pub value: T,
}

impl Default for AutomationRulesDto {
    fn default() -> Self {
        Self {
            inbox_path: "Предметы/00 Входящие".to_owned(),
            downloads_watch_path: "Загрузки".to_owned(),
            obsidian_vault_path: ".".to_owned(),
            student_name: StudentNameDto {
                last_name: "Не задано".to_owned(),
                first_name: "Не задано".to_owned(),
            },
            final_name_pattern: "{Фамилия}_{Имя}_Неделя{Неделя} ДЗ".to_owned(),
            version_pattern: "Работа_Неделя{Неделя}_v{Версия}".to_owned(),
            week_detection: WeekDetection::FilenameThenCalendar,
            semester_start: "1970-01-05T00:00:00Z".to_owned(),
            subject_rules: Vec::new(),
            low_confidence_threshold: 0.7,
            backup_before_change: true,
            auto_route_high_confidence: false,
            confirm: ConfirmationRulesDto {
                low_confidence_route: true,
                mark_final: true,
                mark_submitted: true,
                allow_overwrite: false,
            },
            workflow: WorkflowSettingsDto {
                roles: WorkflowRolesDto {
                    analyst: ProviderId::Codex,
                    author: ProviderId::Claude,
                    reviewer: ProviderId::Codex,
                },
                tasks: AiTaskProvidersDto {
                    material_analysis: ProviderId::Codex,
                    initial_prompt: ProviderId::Codex,
                    solution_review: ProviderId::Codex,
                    revision_prompt: ProviderId::Codex,
                    teacher_review: ProviderId::Codex,
                },
                style_threshold: 4,
                style_profile_path: "ПРОФИЛЬ_СТИЛЯ_ДЗ.md".to_owned(),
                require_teacher_review: true,
            },
        }
    }
}
