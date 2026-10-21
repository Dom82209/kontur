use crate::{dto::SubjectDto, error::BackendError};

use super::ReadRepository;

impl ReadRepository<'_> {
    pub fn subjects(&self) -> Result<Vec<SubjectDto>, BackendError> {
        self.database.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.id, s.name, s.short_name, coalesce(max(w.number), 0), count(w.id), s.course_url
                 FROM subjects s
                 LEFT JOIN weeks w ON w.subject_id = s.id
                 GROUP BY s.id, s.name, s.short_name, s.course_url
                 ORDER BY s.name COLLATE NOCASE, s.id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(SubjectDto {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    short_name: row.get(2)?,
                    current_week: row.get(3)?,
                    week_count: row.get(4)?,
                    course_url: row.get(5)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }
}
