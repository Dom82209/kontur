//! Production workflow, этап 3: управление предметами.
//!
//! Предмет — управляемая сущность: создание, переименование, ключевые слова,
//! архив и слияние. Папка предмета в vault (`folder_path`) задаётся при создании и
//! не меняется: переименование меняет только отображаемое имя, файлы vault не
//! перемещаются. Архивный предмет не предлагается в классификации, но его недели,
//! ДЗ и файлы сохраняются. Слияние — два шага: preview (ничего не пишет) и apply с
//! fingerprint, совпадающим с предпросмотром.

use std::collections::BTreeSet;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::Database,
    dto::SubjectDto,
    error::{AppError, BackendError},
    file_service::PersistentPath,
    repositories::load_subjects,
};

pub const SUBJECTS_ROOT: &str = "Предметы";
const MAX_NAME_CHARS: usize = 80;
const MAX_SHORT_NAME_CHARS: usize = 12;
const MAX_KEYWORDS: usize = 30;
const MAX_KEYWORD_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubjectInput {
    pub name: String,
    pub short_name: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedWeekDto {
    pub number: i64,
    pub has_homework: bool,
    pub files: i64,
}

/// Что произойдёт при слиянии. Ничего не записано.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectMergePreviewDto {
    pub source: SubjectDto,
    pub target: SubjectDto,
    pub moved_weeks: Vec<MergedWeekDto>,
    /// Номера недель, которые есть у обоих предметов: слияние заблокировано.
    pub conflicting_weeks: Vec<i64>,
    pub retargeted_inbox: i64,
    pub added_keywords: Vec<String>,
    pub can_merge: bool,
    pub effects: Vec<String>,
    pub warnings: Vec<String>,
    /// Подтверждение действительно только для этого состояния базы.
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectMergeResultDto {
    pub target: SubjectDto,
    pub source: SubjectDto,
    pub moved_weeks: i64,
    pub retargeted_inbox: i64,
}

pub struct SubjectService<'a> {
    database: &'a Database,
}

impl<'a> SubjectService<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn create(&self, input: &SubjectInput) -> Result<SubjectDto, AppError> {
        let name = normalize_name(&input.name)?;
        let short_name = normalize_short_name(&input.short_name)?;
        let keywords = normalize_keywords(&input.keywords)?;
        let folder_path = format!("{SUBJECTS_ROOT}/{name}");
        PersistentPath::new(&folder_path)
            .map_err(|_| BackendError::blocked("Название нельзя использовать как имя папки"))?;
        let id = Uuid::new_v4().to_string();
        self.database.with_transaction(|transaction| {
            ensure_name_free(transaction, &name, None)?;
            ensure_folder_free(transaction, &folder_path)?;
            transaction.execute(
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES (?1,?2,?3,?4)",
                params![id, name, short_name, folder_path],
            )?;
            replace_keywords(transaction, &id, &keywords)?;
            Ok(())
        })?;
        self.subject(&id)
    }

    /// Переименование не трогает папку в vault: меняется только отображаемое имя.
    pub fn update(
        &self,
        id: &str,
        expected_revision: i64,
        input: &SubjectInput,
    ) -> Result<SubjectDto, AppError> {
        let name = normalize_name(&input.name)?;
        let short_name = normalize_short_name(&input.short_name)?;
        let keywords = normalize_keywords(&input.keywords)?;
        self.database.with_transaction(|transaction| {
            let current = require_revision(transaction, id, expected_revision)?;
            if current.merged {
                return Err(BackendError::blocked(
                    "Предмет слит с другим и не редактируется",
                ));
            }
            ensure_name_free(transaction, &name, Some(id))?;
            transaction.execute(
                "UPDATE subjects SET name=?2, short_name=?3, revision=revision+1,
                        updated_at=CURRENT_TIMESTAMP
                 WHERE id=?1",
                params![id, name, short_name],
            )?;
            replace_keywords(transaction, id, &keywords)?;
            Ok(())
        })?;
        self.subject(id)
    }

    pub fn set_archived(
        &self,
        id: &str,
        expected_revision: i64,
        archived: bool,
    ) -> Result<SubjectDto, AppError> {
        self.database.with_transaction(|transaction| {
            let current = require_revision(transaction, id, expected_revision)?;
            if current.merged && !archived {
                return Err(BackendError::blocked(
                    "Слитый предмет нельзя вернуть из архива: его недели принадлежат другому предмету",
                ));
            }
            if !archived {
                // Возвращённый предмет снова участвует в классификации: ключевые слова
                // не должны совпадать с другим активным предметом.
                let keywords = keywords_of(transaction, id)?;
                ensure_keywords_free(transaction, id, &keywords)?;
            }
            transaction.execute(
                "UPDATE subjects SET archived_at = CASE WHEN ?2 THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') END,
                        revision=revision+1, updated_at=CURRENT_TIMESTAMP
                 WHERE id=?1",
                params![id, archived],
            )?;
            Ok(())
        })?;
        self.subject(id)
    }

    /// Только чтение: показывает, какие недели переедут и что заблокирует слияние.
    pub fn preview_merge(
        &self,
        source_id: &str,
        target_id: &str,
    ) -> Result<SubjectMergePreviewDto, AppError> {
        self.database
            .with_connection(|connection| merge_plan(connection, source_id, target_id))
    }

    pub fn merge(
        &self,
        source_id: &str,
        target_id: &str,
        fingerprint: &str,
    ) -> Result<SubjectMergeResultDto, AppError> {
        let (moved, retargeted) = self.database.with_transaction(|transaction| {
            let plan = merge_plan(transaction, source_id, target_id)?;
            if plan.fingerprint != fingerprint {
                return Err(BackendError::conflict(
                    "Предметы изменились после предпросмотра. Выполните предпросмотр слияния заново.",
                ));
            }
            if !plan.can_merge {
                return Err(BackendError::blocked(
                    "Слияние заблокировано: у предметов есть недели с одинаковыми номерами",
                ));
            }
            let moved = transaction.execute(
                "UPDATE weeks SET subject_id=?2, updated_at=CURRENT_TIMESTAMP WHERE subject_id=?1",
                params![source_id, target_id],
            )?;
            let mut keywords = keywords_of(transaction, target_id)?;
            keywords.extend(plan.added_keywords.iter().cloned());
            transaction.execute(
                "DELETE FROM subject_keywords WHERE subject_id=?1",
                [source_id],
            )?;
            replace_keywords(transaction, target_id, &keywords)?;
            let retargeted = transaction.execute(
                "UPDATE inbox_files
                 SET classification_json = json_set(classification_json, '$.subjectId', ?2),
                     updated_at = CURRENT_TIMESTAMP
                 WHERE status <> 'routed'
                   AND classification_json IS NOT NULL
                   AND json_extract(classification_json, '$.subjectId') = ?1",
                params![source_id, target_id],
            )?;
            transaction.execute(
                "UPDATE subjects SET archived_at = coalesce(archived_at, strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                        merged_into_id=?2, revision=revision+1, updated_at=CURRENT_TIMESTAMP
                 WHERE id=?1",
                params![source_id, target_id],
            )?;
            transaction.execute(
                "UPDATE subjects SET revision=revision+1, updated_at=CURRENT_TIMESTAMP WHERE id=?1",
                [target_id],
            )?;
            transaction.execute(
                "INSERT INTO subject_merges(id,source_id,target_id,fingerprint,moved_weeks,retargeted_inbox,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![
                    Uuid::new_v4().to_string(),
                    source_id,
                    target_id,
                    fingerprint,
                    moved as i64,
                    retargeted as i64
                ],
            )?;
            Ok((moved as i64, retargeted as i64))
        })?;
        Ok(SubjectMergeResultDto {
            target: self.subject(target_id)?,
            source: self.subject(source_id)?,
            moved_weeks: moved,
            retargeted_inbox: retargeted,
        })
    }

    pub fn subject(&self, id: &str) -> Result<SubjectDto, AppError> {
        self.database
            .with_connection(|connection| find_subject(connection, id))
    }
}

fn find_subject(connection: &Connection, id: &str) -> Result<SubjectDto, AppError> {
    load_subjects(connection)?
        .into_iter()
        .find(|subject| subject.id == id)
        .ok_or_else(|| BackendError::not_found("Предмет не найден"))
}

struct CurrentSubject {
    merged: bool,
}

fn require_revision(
    transaction: &Transaction<'_>,
    id: &str,
    expected: i64,
) -> Result<CurrentSubject, AppError> {
    let row: Option<(i64, bool)> = transaction
        .query_row(
            "SELECT revision, merged_into_id IS NOT NULL FROM subjects WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (revision, merged) = row.ok_or_else(|| BackendError::not_found("Предмет не найден"))?;
    if revision != expected {
        return Err(BackendError::conflict(
            "Предмет изменился в другом окне. Обновите данные и повторите.",
        ));
    }
    Ok(CurrentSubject { merged })
}

fn merge_plan(
    connection: &Connection,
    source_id: &str,
    target_id: &str,
) -> Result<SubjectMergePreviewDto, AppError> {
    if source_id == target_id {
        return Err(BackendError::blocked("Предмет нельзя слить с самим собой"));
    }
    let source = find_subject(connection, source_id)?;
    let target = find_subject(connection, target_id)?;
    if source.merged_into_id.is_some() {
        return Err(BackendError::blocked("Этот предмет уже слит с другим"));
    }
    if target.archived {
        return Err(BackendError::blocked(
            "Сливать можно только в активный предмет: сначала верните его из архива",
        ));
    }
    let mut statement = connection.prepare(
        "SELECT w.id, w.number,
                EXISTS(SELECT 1 FROM homeworks h WHERE h.week_id = w.id),
                (SELECT count(*) FROM files f WHERE f.week_id = w.id
                   OR f.homework_id IN (SELECT h.id FROM homeworks h WHERE h.week_id = w.id))
         FROM weeks w WHERE w.subject_id = ?1 ORDER BY w.number, w.id",
    )?;
    let mut weeks = |subject: &str| -> Result<Vec<(String, i64, bool, i64)>, AppError> {
        let rows = statement.query_map([subject], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    };
    let source_weeks = weeks(source_id)?;
    let target_weeks = weeks(target_id)?;
    let target_numbers: BTreeSet<i64> = target_weeks.iter().map(|week| week.1).collect();
    let conflicting_weeks: Vec<i64> = source_weeks
        .iter()
        .map(|week| week.1)
        .filter(|number| target_numbers.contains(number))
        .collect();
    let moved_weeks: Vec<MergedWeekDto> = source_weeks
        .iter()
        .map(|(_, number, has_homework, files)| MergedWeekDto {
            number: *number,
            has_homework: *has_homework,
            files: *files,
        })
        .collect();
    let mut inbox_statement = connection.prepare(
        "SELECT id FROM inbox_files
         WHERE status <> 'routed' AND classification_json IS NOT NULL
           AND json_extract(classification_json, '$.subjectId') = ?1
         ORDER BY id",
    )?;
    let inbox: Vec<String> = inbox_statement
        .query_map([source_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let target_keywords: BTreeSet<&String> = target.keywords.iter().collect();
    let added_keywords: Vec<String> = source
        .keywords
        .iter()
        .filter(|keyword| !target_keywords.contains(keyword))
        .cloned()
        .collect();

    let mut hasher = Sha256::new();
    for part in [
        source.id.as_str(),
        &source.revision.to_string(),
        target.id.as_str(),
        &target.revision.to_string(),
    ] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    for (id, number, has_homework, files) in source_weeks.iter().chain(target_weeks.iter()) {
        hasher.update(format!("{id}:{number}:{has_homework}:{files}").as_bytes());
        hasher.update([0]);
    }
    for id in &inbox {
        hasher.update(id.as_bytes());
        hasher.update([0]);
    }
    let fingerprint = format!("{:x}", hasher.finalize());

    let can_merge = conflicting_weeks.is_empty();
    let mut effects = vec![
        format!(
            "Недели «{}» ({}) станут неделями «{}».",
            source.name,
            moved_weeks.len(),
            target.name
        ),
        format!("«{}» уйдёт в архив с пометкой «слит».", source.name),
        "Файлы в vault не перемещаются и не переименовываются: недели сохраняют свои папки."
            .to_owned(),
    ];
    if !inbox.is_empty() {
        effects.push(format!(
            "Файлы во «Входящих», отнесённые к «{}» ({}), будут отнесены к «{}».",
            source.name,
            inbox.len(),
            target.name
        ));
    }
    if !added_keywords.is_empty() {
        effects.push(format!(
            "Ключевые слова перейдут к «{}»: {}.",
            target.name,
            added_keywords.join(", ")
        ));
    }
    let mut warnings = Vec::new();
    if !can_merge {
        warnings.push(format!(
            "У обоих предметов есть недели {}. Слияние таких недель не поддерживается: сначала разберите их вручную.",
            conflicting_weeks
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if source.folder_path != target.folder_path && !moved_weeks.is_empty() {
        warnings.push(format!(
            "Материалы останутся в папке «{}» в vault.",
            source.folder_path
        ));
    }
    Ok(SubjectMergePreviewDto {
        source,
        target,
        moved_weeks,
        conflicting_weeks,
        retargeted_inbox: inbox.len() as i64,
        added_keywords,
        can_merge,
        effects,
        warnings,
        fingerprint,
    })
}

fn normalize_name(raw: &str) -> Result<String, AppError> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(BackendError::blocked("Укажите название предмета"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(BackendError::blocked(format!(
            "Название длиннее {MAX_NAME_CHARS} символов"
        )));
    }
    if name.starts_with('.')
        || name.starts_with("00 ")
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(BackendError::blocked(
            "Название не должно начинаться с точки или «00 » и содержать / \\ : или управляющие символы",
        ));
    }
    Ok(name)
}

fn normalize_short_name(raw: &str) -> Result<String, AppError> {
    let short = raw.trim().to_owned();
    if short.is_empty() || short.chars().count() > MAX_SHORT_NAME_CHARS {
        return Err(BackendError::blocked(format!(
            "Короткое название: от 1 до {MAX_SHORT_NAME_CHARS} символов"
        )));
    }
    if short.chars().any(char::is_control) {
        return Err(BackendError::blocked(
            "Короткое название содержит управляющие символы",
        ));
    }
    Ok(short)
}

/// Нижний регистр, без пробелов по краям, без повторов; от 2 до 60 символов.
pub fn normalize_keywords(raw: &[String]) -> Result<Vec<String>, AppError> {
    let mut output: Vec<String> = Vec::new();
    for keyword in raw {
        let keyword = keyword
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if keyword.is_empty() {
            continue;
        }
        let length = keyword.chars().count();
        if !(2..=MAX_KEYWORD_CHARS).contains(&length) || keyword.chars().any(char::is_control) {
            return Err(BackendError::blocked(format!(
                "Ключевое слово «{keyword}»: от 2 до {MAX_KEYWORD_CHARS} символов"
            )));
        }
        if !output.contains(&keyword) {
            output.push(keyword);
        }
    }
    if output.len() > MAX_KEYWORDS {
        return Err(BackendError::blocked(format!(
            "Не больше {MAX_KEYWORDS} ключевых слов на предмет"
        )));
    }
    output.sort();
    Ok(output)
}

/// Заменяет ключевые слова предмета. Слово не может принадлежать двум активным
/// предметам: иначе автоматическая классификация стала бы неоднозначной.
pub(crate) fn replace_keywords(
    transaction: &Connection,
    subject_id: &str,
    keywords: &[String],
) -> Result<(), BackendError> {
    let keywords = normalize_keywords(keywords)?;
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM subjects WHERE id=?1)",
        [subject_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(BackendError::not_found(
            "Предмет для ключевых слов не найден",
        ));
    }
    ensure_keywords_free(transaction, subject_id, &keywords)?;
    transaction.execute(
        "DELETE FROM subject_keywords WHERE subject_id=?1",
        [subject_id],
    )?;
    for keyword in &keywords {
        transaction.execute(
            "INSERT INTO subject_keywords(subject_id, keyword) VALUES (?1, ?2)",
            params![subject_id, keyword],
        )?;
    }
    Ok(())
}

fn keywords_of(connection: &Connection, subject_id: &str) -> Result<Vec<String>, BackendError> {
    let mut statement = connection
        .prepare("SELECT keyword FROM subject_keywords WHERE subject_id=?1 ORDER BY keyword")?;
    let rows = statement.query_map([subject_id], |row| row.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn ensure_keywords_free(
    connection: &Connection,
    subject_id: &str,
    keywords: &[String],
) -> Result<(), BackendError> {
    let mut statement = connection.prepare(
        "SELECT k.keyword, s.name FROM subject_keywords k JOIN subjects s ON s.id = k.subject_id
         WHERE k.subject_id <> ?1 AND s.archived_at IS NULL",
    )?;
    let rows = statement.query_map([subject_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (keyword, owner) = row?;
        if keywords
            .iter()
            .any(|candidate| candidate.to_lowercase() == keyword.to_lowercase())
        {
            return Err(BackendError::conflict(format!(
                "Ключевое слово «{keyword}» уже относится к предмету «{owner}»"
            )));
        }
    }
    Ok(())
}

/// Имена сравниваются без учёта регистра (Unicode): на macOS папки
/// «Статистика» и «статистика» — одна и та же папка.
fn ensure_name_free(
    connection: &Connection,
    name: &str,
    except: Option<&str>,
) -> Result<(), BackendError> {
    let mut statement = connection.prepare("SELECT id, name, folder_path FROM subjects")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let wanted = name.to_lowercase();
    for row in rows {
        let (id, existing, _) = row?;
        if Some(id.as_str()) != except && existing.to_lowercase() == wanted {
            return Err(BackendError::conflict(format!(
                "Предмет «{existing}» уже существует"
            )));
        }
    }
    Ok(())
}

fn ensure_folder_free(connection: &Connection, folder_path: &str) -> Result<(), BackendError> {
    let mut statement = connection.prepare("SELECT folder_path FROM subjects")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let wanted = folder_path.to_lowercase();
    for row in rows {
        if row?.to_lowercase() == wanted {
            return Err(BackendError::conflict(
                "Папка предмета уже занята другим предметом",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "subjects_tests.rs"]
mod tests;
