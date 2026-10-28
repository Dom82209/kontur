use std::collections::HashMap;

use rusqlite::params;

use crate::{dto::WeekDto, error::BackendError};

use super::{files::group_week_files, ReadRepository};

#[derive(Debug)]
struct WeekRow {
    id: String,
    subject_id: String,
    number: i64,
    starts_at: String,
    ends_at: String,
    topic: Option<String>,
}

impl ReadRepository<'_> {
    pub fn weeks(&self, subject_id: Option<&str>) -> Result<Vec<WeekDto>, BackendError> {
        let rows = self.week_rows(subject_id)?;
        let mut files = group_week_files(self.week_files(subject_id)?);
        let homework_ids = self.week_homework_ids(subject_id)?;
        let mut output = Vec::with_capacity(rows.len());
        for row in rows {
            let mut lectures = Vec::new();
            let mut seminars = Vec::new();
            let mut materials = Vec::new();
            for (category, file) in files.remove(&row.id).unwrap_or_default() {
                match category.as_str() {
                    "lecture" => lectures.push(file),
                    "seminar" => seminars.push(file),
                    "material" => materials.push(file),
                    _ => {}
                }
            }
            output.push(WeekDto {
                homework_id: homework_ids.get(&row.id).cloned(),
                id: row.id,
                subject_id: row.subject_id,
                number: row.number,
                starts_at: known(row.starts_at),
                ends_at: known(row.ends_at),
                topic: row.topic,
                lectures,
                seminars,
                materials,
            });
        }
        Ok(output)
    }

    pub fn week(&self, subject_id: &str, week_number: i64) -> Result<WeekDto, BackendError> {
        self.weeks(Some(subject_id))?
            .into_iter()
            .find(|week| week.number == week_number)
            .ok_or_else(|| BackendError::not_found("week not found"))
    }

    fn week_rows(&self, subject_id: Option<&str>) -> Result<Vec<WeekRow>, BackendError> {
        self.database.with_connection(|connection| {
            let sql = if subject_id.is_some() {
                "SELECT id, subject_id, number, starts_at, ends_at, topic
                 FROM weeks WHERE subject_id = ?1 ORDER BY number, id"
            } else {
                "SELECT id, subject_id, number, starts_at, ends_at, topic
                 FROM weeks ORDER BY subject_id, number, id"
            };
            let mut statement = connection.prepare(sql)?;
            let mut rows = if let Some(id) = subject_id {
                statement.query(params![id])?
            } else {
                statement.query([])?
            };
            let mut output = Vec::new();
            while let Some(row) = rows.next()? {
                output.push(WeekRow {
                    id: row.get(0)?,
                    subject_id: row.get(1)?,
                    number: row.get(2)?,
                    starts_at: row.get(3)?,
                    ends_at: row.get(4)?,
                    topic: row.get(5)?,
                });
            }
            Ok(output)
        })
    }

    fn week_homework_ids(
        &self,
        subject_id: Option<&str>,
    ) -> Result<HashMap<String, String>, BackendError> {
        self.database.with_connection(|connection| {
            let sql = if subject_id.is_some() {
                "SELECT h.week_id, h.id FROM homeworks h
                 JOIN weeks w ON w.id = h.week_id WHERE w.subject_id = ?1"
            } else {
                "SELECT week_id, id FROM homeworks"
            };
            let mut statement = connection.prepare(sql)?;
            let mut rows = if let Some(id) = subject_id {
                statement.query(params![id])?
            } else {
                statement.query([])?
            };
            let mut output = HashMap::new();
            while let Some(row) = rows.next()? {
                output.insert(row.get(0)?, row.get(1)?);
            }
            Ok(output)
        })
    }
}

/// Пустая строка в `weeks.starts_at/ends_at` означает «даты неизвестны».
fn known(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}
