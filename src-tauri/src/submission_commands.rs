use serde::Deserialize;
use tauri::{AppHandle, Runtime, State};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    dto::OperationResultDto,
    error::{AppError, BackendError},
    file_service::SourceFile,
    submission::SubmissionService,
    workflow::{
        dto::{
            PackageCheckId, StyleProfileDto, SubmissionConfirmationInput, SubmissionPackageDto,
            SubmissionReceiptDto, TeacherCommentDto, TeacherCommentSource,
        },
        service::{CommandMeta, TeacherCommentInput},
    },
    workflow_commands::{finish, with_workflow},
};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptFile {
    source_path: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptInput {
    text: Option<String>,
    file: Option<TranscriptFile>,
    source: TeacherCommentSource,
    date: String,
    #[serde(default)]
    confirmed: bool,
}

#[tauri::command]
pub fn get_style_profile(state: State<'_, AppState>) -> Result<Option<StyleProfileDto>, AppError> {
    SubmissionService::new(state.database(), state.home_dir()).style_profile()
}

#[tauri::command]
pub fn remove_teacher_comment(
    state: State<'_, AppState>,
    comment_id: String,
) -> Result<(), AppError> {
    SubmissionService::new(state.database(), state.home_dir()).remove_teacher_comment(&comment_id)
}

#[tauri::command]
pub fn build_submission_package(
    state: State<'_, AppState>,
    homework_id: String,
) -> Result<OperationResultDto<SubmissionPackageDto>, AppError> {
    let _guard = state.lock_file_mutations()?;
    SubmissionService::new(state.database(), state.home_dir()).build(&homework_id)
}

#[tauri::command]
pub fn set_package_check(
    state: State<'_, AppState>,
    package_id: String,
    check_id: PackageCheckId,
    done: bool,
) -> Result<SubmissionPackageDto, AppError> {
    SubmissionService::new(state.database(), state.home_dir()).set_check(
        &package_id,
        check_id,
        done,
    )
}

#[tauri::command]
pub fn confirm_submission(
    state: State<'_, AppState>,
    homework_id: String,
    confirmation: SubmissionConfirmationInput,
) -> Result<OperationResultDto<SubmissionReceiptDto>, AppError> {
    let _guard = state.lock_file_mutations()?;
    let proof = confirmation
        .proof
        .as_ref()
        .map(|proof| {
            let path = proof.path.as_deref().ok_or_else(|| {
                BackendError::blocked("Файл подтверждения нужно выбрать системным диалогом")
            })?;
            let path = std::path::Path::new(path);
            if !state.grants().is_granted(path) {
                return Err(BackendError::blocked(
                    "Нет разрешения на чтение файла подтверждения; выберите его заново",
                ));
            }
            let actual_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| BackendError::blocked("Имя файла подтверждения недоступно"))?;
            if actual_name != proof.name {
                return Err(BackendError::blocked(
                    "Файл подтверждения изменился после выбора",
                ));
            }
            Ok((SourceFile::open_external(path)?, proof.name.clone()))
        })
        .transpose()?;
    SubmissionService::new(state.database(), state.home_dir()).confirm(
        &homework_id,
        &confirmation,
        proof,
    )
}

#[tauri::command]
pub fn import_teacher_transcript<R: Runtime>(
    app: AppHandle<R>,
    homework_id: String,
    input: TranscriptInput,
) -> Result<OperationResultDto<Vec<TeacherCommentDto>>, AppError> {
    if !input.confirmed {
        return Err(BackendError::new(
            crate::error::ErrorCode::ConfirmationRequired,
            "Сначала просмотрите распознанные пары и подтвердите импорт",
        ));
    }
    let comments = preview_input(&input)?;
    let meta = CommandMeta {
        operation_id: Uuid::new_v4().to_string(),
        expected_revision: None,
    };
    let result = with_workflow(&app, |service| {
        service.add_teacher_comments(&meta, &homework_id, &comments)
    });
    finish(&app, result)
}

#[tauri::command]
pub fn preview_teacher_transcript(
    input: TranscriptInput,
) -> Result<Vec<TeacherCommentInput>, AppError> {
    preview_input(&input)
}

fn preview_input(input: &TranscriptInput) -> Result<Vec<TeacherCommentInput>, AppError> {
    let text = match (
        input.text.clone(),
        input.file.clone().and_then(|f| f.source_path),
    ) {
        (Some(text), _) if !text.trim().is_empty() => text,
        (_, Some(path)) => SourceFile::open_external(std::path::Path::new(&path))?
            .read_utf8_limited(2 * 1024 * 1024)?,
        _ => {
            return Err(BackendError::blocked(
                "Вставьте текст или выберите текстовый файл",
            ))
        }
    };
    parse_transcript(&text, input.source, input.date.clone())
}

fn parse_transcript(
    text: &str,
    source: TeacherCommentSource,
    date: String,
) -> Result<Vec<TeacherCommentInput>, AppError> {
    let mut result = Vec::new();
    let mut question = String::new();
    let mut answer = String::new();
    for raw in text.lines().map(str::trim).filter(|x| !x.is_empty()) {
        let lower = raw.to_lowercase();
        let is_question = lower.starts_with("вопрос:")
            || lower.starts_with("в:")
            || lower.starts_with("я:")
            || raw.ends_with('?');
        let is_answer = lower.starts_with("ответ:")
            || lower.starts_with("о:")
            || lower.starts_with("преподаватель:");
        if is_question {
            if !question.is_empty() || !answer.is_empty() {
                result.push(TeacherCommentInput {
                    source,
                    date: date.clone(),
                    question: question.trim().to_owned(),
                    answer: answer.trim().to_owned(),
                });
                question.clear();
                answer.clear();
            }
            question = raw
                .split_once(':')
                .map(|x| x.1)
                .unwrap_or(raw)
                .trim()
                .to_owned();
        } else if is_answer {
            answer = raw
                .split_once(':')
                .map(|x| x.1)
                .unwrap_or(raw)
                .trim()
                .to_owned();
        } else if question.is_empty() {
            question = raw.to_owned();
        } else {
            if !answer.is_empty() {
                answer.push('\n');
            }
            answer.push_str(raw);
        }
    }
    if !question.is_empty() || !answer.is_empty() {
        result.push(TeacherCommentInput {
            source,
            date,
            question: question.trim().to_owned(),
            answer: answer.trim().to_owned(),
        });
    }
    if result.is_empty() {
        return Err(BackendError::blocked(
            "Не удалось выделить пары «вопрос — ответ»",
        ));
    }
    if result.len() > 50 {
        return Err(BackendError::blocked(
            "Найдено больше 50 комментариев — разделите импорт",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_d_transcript_preview_extracts_pairs_without_writing() {
        let pairs = parse_transcript(
            "Вопрос: Нужен ли шлюз?\nПреподаватель: Да, добавьте его.\nВ: Какой формат?\nО: PDF и исходник.",
            TeacherCommentSource::Chat,
            "2026-10-26".to_owned(),
        )
        .unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].question, "Нужен ли шлюз?");
        assert_eq!(pairs[0].answer, "Да, добавьте его.");
        assert_eq!(pairs[1].answer, "PDF и исходник.");
        assert!(pairs
            .iter()
            .all(|pair| pair.source == TeacherCommentSource::Chat && pair.date == "2026-10-26"));
    }
}
