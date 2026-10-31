use std::collections::HashMap;

use rusqlite::Connection;

use crate::{dto::SubjectDto, error::BackendError};

use super::ReadRepository;

impl ReadRepository<'_> {
    /// Все предметы, включая архивные (поле `archived`). Потребители, предлагающие
    /// предмет для классификации, обязаны пропускать архивные.
    pub fn subjects(&self) -> Result<Vec<SubjectDto>, BackendError> {
        self.database.with_connection(load_subjects)
    }
}

pub(crate) fn load_subjects(connection: &Connection) -> Result<Vec<SubjectDto>, BackendError> {
    let mut keywords = subject_keywords(connection)?;
    let mut statement = connection.prepare(
        "SELECT s.id, s.name, s.short_name, coalesce(max(w.number), 0), count(w.id), s.course_url,
                s.folder_path, s.archived_at IS NOT NULL, s.merged_into_id, s.revision
         FROM subjects s
         LEFT JOIN weeks w ON w.subject_id = s.id
         GROUP BY s.id
         ORDER BY s.archived_at IS NOT NULL, s.name COLLATE NOCASE, s.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SubjectDto {
            id: row.get(0)?,
            name: row.get(1)?,
            short_name: row.get(2)?,
            current_week: row.get(3)?,
            week_count: row.get(4)?,
            course_url: row.get(5)?,
            folder_path: row.get(6)?,
            keywords: Vec::new(),
            archived: row.get(7)?,
            merged_into_id: row.get(8)?,
            revision: row.get(9)?,
        })
    })?;
    let mut output = rows.collect::<Result<Vec<_>, _>>()?;
    for subject in &mut output {
        subject.keywords = keywords.remove(&subject.id).unwrap_or_default();
    }
    Ok(output)
}

/// Ключевые слова по предметам, отсортированные для стабильного вывода.
pub(crate) fn subject_keywords(
    connection: &Connection,
) -> Result<HashMap<String, Vec<String>>, BackendError> {
    let mut statement = connection
        .prepare("SELECT subject_id, keyword FROM subject_keywords ORDER BY subject_id, keyword")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut output: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (subject, keyword) = row?;
        output.entry(subject).or_default().push(keyword);
    }
    Ok(output)
}
