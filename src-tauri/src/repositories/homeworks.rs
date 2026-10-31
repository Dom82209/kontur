use std::{collections::HashMap, path::Path};

use rusqlite::{params, Connection};

use crate::{
    domain::{HomeworkStage, HomeworkWorkflowStage, VersionAuthor},
    dto::{FileKind, FileRefDto, HomeworkDto, HomeworkVersionDto, VersionSectionDto},
    error::BackendError,
};

use super::{files::task_files, invalid_enum, parse_json, require_relative_path, ReadRepository};

#[derive(Debug)]
struct HomeworkRow {
    id: String,
    subject_id: String,
    week_number: i64,
    ordinal: i64,
    title: String,
    deadline: Option<String>,
    stage: String,
    workflow_stage: String,
    final_version_id: Option<String>,
    final_file_name: Option<String>,
    submitted_at: Option<String>,
    folder_path: String,
    assignment_url: Option<String>,
    revision: i64,
}

#[derive(Debug)]
struct VersionRow {
    id: String,
    homework_id: String,
    number: i64,
    label: String,
    file_name: String,
    file_path: String,
    size_bytes: i64,
    created_at: String,
    source_file_id: String,
    note: Option<String>,
    sections_json: String,
    author: String,
    prompt_id: Option<String>,
    source_bundle_id: Option<String>,
    files_json: String,
    frozen: bool,
}

impl ReadRepository<'_> {
    pub fn homework_list(&self) -> Result<Vec<HomeworkDto>, BackendError> {
        self.database
            .with_connection(|connection| load_homeworks(connection, None))
    }

    pub fn homework(&self, homework_id: &str) -> Result<HomeworkDto, BackendError> {
        self.database
            .with_connection(|connection| load_homework(connection, homework_id))
    }
}

/// Один Homework: фиксированные три запроса (строка, файлы условия, версии).
pub(crate) fn load_homework(
    connection: &Connection,
    homework_id: &str,
) -> Result<HomeworkDto, BackendError> {
    load_homeworks(connection, Some(homework_id))?
        .into_iter()
        .next()
        .ok_or_else(|| BackendError::not_found("homework not found"))
}

/// Список Homework без N+1: строки, все файлы условия и все версии — по одному запросу.
pub(crate) fn load_homeworks(
    connection: &Connection,
    homework_id: Option<&str>,
) -> Result<Vec<HomeworkDto>, BackendError> {
    let rows = homework_rows(connection, homework_id)?;
    let mut task_files = task_files(connection, homework_id)?;
    let mut versions = load_versions(connection, homework_id)?;
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        // NULL — срок неизвестен (например, ДЗ подключено из существующего vault).
        let deadline = row.deadline;
        require_relative_path(&row.folder_path, "homeworks.folder_path")?;
        let stage = HomeworkStage::try_from_db(&row.stage)
            .map_err(|message| invalid_enum("homeworks.stage", message))?;
        let workflow_stage = HomeworkWorkflowStage::try_from_db(&row.workflow_stage)
            .map_err(|message| invalid_enum("homeworks.workflow_stage", message))?;
        output.push(HomeworkDto {
            task_files: task_files.remove(&row.id).unwrap_or_default(),
            versions: versions.remove(&row.id).unwrap_or_default(),
            id: row.id,
            subject_id: row.subject_id,
            week_number: row.week_number,
            ordinal: row.ordinal,
            title: row.title,
            deadline,
            stage,
            final_version_id: row.final_version_id,
            final_file_name: row.final_file_name,
            submitted_at: row.submitted_at,
            folder_path: row.folder_path,
            workflow_stage,
            assignment_url: row.assignment_url,
            revision: row.revision,
        });
    }
    Ok(output)
}

fn homework_rows(
    connection: &Connection,
    homework_id: Option<&str>,
) -> Result<Vec<HomeworkRow>, BackendError> {
    let base = "SELECT h.id, w.subject_id, w.number, h.title, h.deadline, h.stage,
                       h.workflow_stage, h.final_version_id, h.final_file_name, h.submitted_at,
                       h.folder_path, h.assignment_url, h.revision, h.ordinal
                FROM homeworks h JOIN weeks w ON w.id = h.week_id";
    let sql = if homework_id.is_some() {
        format!("{base} WHERE h.id = ?1")
    } else {
        format!("{base} ORDER BY h.deadline IS NULL, h.deadline, w.number, h.ordinal, h.id")
    };
    let mut statement = connection.prepare(&sql)?;
    let mut rows = if let Some(id) = homework_id {
        statement.query(params![id])?
    } else {
        statement.query([])?
    };
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        output.push(HomeworkRow {
            id: row.get(0)?,
            subject_id: row.get(1)?,
            week_number: row.get(2)?,
            title: row.get(3)?,
            deadline: row.get(4)?,
            stage: row.get(5)?,
            workflow_stage: row.get(6)?,
            final_version_id: row.get(7)?,
            final_file_name: row.get(8)?,
            submitted_at: row.get(9)?,
            folder_path: row.get(10)?,
            assignment_url: row.get(11)?,
            revision: row.get(12)?,
            ordinal: row.get(13)?,
        });
    }
    Ok(output)
}

/// Версии одного или всех Homework, сгруппированные по homework id и упорядоченные по номеру.
pub(crate) fn load_versions(
    connection: &Connection,
    homework_id: Option<&str>,
) -> Result<HashMap<String, Vec<HomeworkVersionDto>>, BackendError> {
    let rows = version_rows(connection, homework_id)?;
    let mut output: HashMap<String, Vec<HomeworkVersionDto>> = HashMap::new();
    for row in rows {
        let version = hydrate_version(row)?;
        output
            .entry(version.homework_id.clone())
            .or_default()
            .push(version);
    }
    Ok(output)
}

fn version_rows(
    connection: &Connection,
    homework_id: Option<&str>,
) -> Result<Vec<VersionRow>, BackendError> {
    let base = "SELECT id, homework_id, number, label, file_name, file_path, size_bytes,
                       created_at, source_file_id, note, sections_json, author, prompt_id,
                       source_bundle_id, files_json, frozen
                FROM homework_versions";
    let sql = if homework_id.is_some() {
        format!("{base} WHERE homework_id = ?1 ORDER BY number, id")
    } else {
        format!("{base} ORDER BY homework_id, number, id")
    };
    let mut statement = connection.prepare(&sql)?;
    let mut rows = if let Some(id) = homework_id {
        statement.query(params![id])?
    } else {
        statement.query([])?
    };
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        output.push(VersionRow {
            id: row.get(0)?,
            homework_id: row.get(1)?,
            number: row.get(2)?,
            label: row.get(3)?,
            file_name: row.get(4)?,
            file_path: row.get(5)?,
            size_bytes: row.get(6)?,
            created_at: row.get(7)?,
            source_file_id: row.get(8)?,
            note: row.get(9)?,
            sections_json: row.get(10)?,
            author: row.get(11)?,
            prompt_id: row.get(12)?,
            source_bundle_id: row.get(13)?,
            files_json: row.get(14)?,
            frozen: row.get::<_, i64>(15)? == 1,
        });
    }
    Ok(output)
}

fn hydrate_version(row: VersionRow) -> Result<HomeworkVersionDto, BackendError> {
    require_relative_path(&row.file_path, "homework_versions.file_path")?;
    let file_name = if row.file_name.trim().is_empty() {
        Path::new(&row.file_path)
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned)
            .ok_or_else(|| BackendError::database("legacy homework version has no file name"))?
    } else {
        row.file_name
    };
    let source_file_id = if row.source_file_id.trim().is_empty() {
        format!("legacy:{}", row.id)
    } else {
        row.source_file_id
    };
    let sections: Vec<VersionSectionDto> =
        parse_json(&row.sections_json, "homework_versions.sections_json")?;
    let mut files: Vec<FileRefDto> = parse_json(&row.files_json, "homework_versions.files_json")?;
    for file in &files {
        require_relative_path(
            &file.relative_path,
            "homework_versions.files_json.relativePath",
        )?;
    }
    if files.is_empty() {
        files.push(FileRefDto {
            id: source_file_id.clone(),
            name: file_name.clone(),
            kind: infer_file_kind(&file_name),
            size_bytes: row.size_bytes,
            modified_at: row.created_at.clone(),
            relative_path: row.file_path.clone(),
        });
    }
    let author = VersionAuthor::try_from_db(&row.author)
        .map_err(|message| invalid_enum("homework_versions.author", message))?;
    Ok(HomeworkVersionDto {
        id: row.id,
        homework_id: row.homework_id,
        number: row.number,
        label: row.label,
        file_name,
        size_bytes: row.size_bytes,
        created_at: row.created_at,
        source_file_id,
        note: row.note,
        sections,
        author,
        prompt_id: row.prompt_id,
        source_bundle_id: row.source_bundle_id,
        files,
        frozen: row.frozen,
    })
}

pub(crate) fn infer_file_kind(file_name: &str) -> FileKind {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => FileKind::Pdf,
        "doc" | "docx" => FileKind::Doc,
        "xls" | "xlsx" | "csv" => FileKind::Sheet,
        "ppt" | "pptx" => FileKind::Slides,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" => FileKind::Image,
        "ipynb" => FileKind::Notebook,
        "txt" | "md" => FileKind::Text,
        "zip" | "rar" | "7z" => FileKind::Archive,
        _ => FileKind::Other,
    }
}
