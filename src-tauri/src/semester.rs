//! Production workflow, этап 5: семестр, календарь недель и точные сроки.
//!
//! Семестр — дата начала (местная дата недели 1), смещение часового пояса и число
//! недель. Неделя N начинается в 00:00 местного времени `start + 7·(N−1)` дней и
//! заканчивается через 7 дней минус секунда; в базе хранится UTC (RFC 3339).
//! Без настроенного семестра даты недель неизвестны и не выдумываются. Сроки ДЗ
//! задаются только явно (или «неизвестен»); `deadline_source` хранит происхождение.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use time::{
    format_description::well_known::Rfc3339, Date, Duration, OffsetDateTime, Time, UtcOffset,
};

use crate::{
    db::Database,
    dto::HomeworkDto,
    error::{AppError, BackendError},
    homework_registry::normalize_deadline,
    repositories::ReadRepository,
};

const MIN_OFFSET_MINUTES: i32 = -720;
const MAX_OFFSET_MINUTES: i32 = 840;
const MAX_WEEKS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemesterInput {
    /// Местная дата начала недели 1, `YYYY-MM-DD`.
    pub start_date: String,
    /// Смещение местного времени от UTC в минутах (Москва: 180).
    pub utc_offset_minutes: i32,
    pub week_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemesterWeekDto {
    pub number: i64,
    pub starts_at: String,
    pub ends_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemesterDto {
    pub start_date: String,
    pub utc_offset_minutes: i32,
    pub week_count: i64,
    pub revision: i64,
    pub weeks: Vec<SemesterWeekDto>,
    /// `None` — сейчас вне семестра (до начала или после окончания).
    pub current_week: Option<i64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemesterSaveResultDto {
    pub semester: SemesterDto,
    /// Сколько недель получили даты из семестра.
    pub updated_weeks: i64,
}

/// Вычисленный календарь семестра.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemesterCalendar {
    start: Date,
    offset: UtcOffset,
    week_count: i64,
}

impl SemesterCalendar {
    pub fn new(
        start_date: &str,
        utc_offset_minutes: i32,
        week_count: i64,
    ) -> Result<Self, AppError> {
        let start = parse_date(start_date.trim()).ok_or_else(|| {
            BackendError::blocked("Дата начала семестра должна быть в формате ГГГГ-ММ-ДД")
        })?;
        if !(MIN_OFFSET_MINUTES..=MAX_OFFSET_MINUTES).contains(&utc_offset_minutes)
            || utc_offset_minutes % 15 != 0
        {
            return Err(BackendError::blocked(
                "Часовой пояс: смещение от UTC−12:00 до UTC+14:00 с шагом 15 минут",
            ));
        }
        if !(1..=MAX_WEEKS).contains(&week_count) {
            return Err(BackendError::blocked(format!(
                "Число недель семестра: от 1 до {MAX_WEEKS}"
            )));
        }
        let offset = UtcOffset::from_whole_seconds(utc_offset_minutes * 60)
            .map_err(|_| BackendError::blocked("Некорректное смещение часового пояса"))?;
        Ok(Self {
            start,
            offset,
            week_count,
        })
    }

    /// Начало и конец недели `number` в UTC. Работает и за пределами `week_count`.
    pub fn week_range(&self, number: i64) -> Result<(OffsetDateTime, OffsetDateTime), AppError> {
        if number < 1 {
            return Err(BackendError::blocked("Номер недели должен быть от 1"));
        }
        let day = self
            .start
            .checked_add(Duration::days(7 * (number - 1)))
            .ok_or_else(|| BackendError::blocked("Неделя вне допустимого диапазона дат"))?;
        let starts = day.with_time(Time::MIDNIGHT).assume_offset(self.offset);
        let ends = starts + Duration::days(7) - Duration::seconds(1);
        Ok((
            starts.to_offset(UtcOffset::UTC),
            ends.to_offset(UtcOffset::UTC),
        ))
    }

    pub fn week_strings(&self, number: i64) -> Result<(String, String), AppError> {
        let (starts, ends) = self.week_range(number)?;
        Ok((format_utc(starts)?, format_utc(ends)?))
    }

    /// Номер текущей недели или `None`, если момент вне семестра.
    pub fn week_at(&self, now: OffsetDateTime) -> Option<i64> {
        let (start, _) = self.week_range(1).ok()?;
        let elapsed = (now - start).whole_seconds();
        if elapsed < 0 {
            return None;
        }
        let index = elapsed / (7 * 24 * 60 * 60) + 1;
        (index <= self.week_count).then_some(index)
    }

    fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.start.weekday() != time::Weekday::Monday {
            warnings.push(
                "Семестр начинается не с понедельника: недели будут идти от этого дня недели."
                    .to_owned(),
            );
        }
        warnings
    }
}

fn parse_date(value: &str) -> Option<Date> {
    let mut parts = value.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let month = time::Month::try_from(month.parse::<u8>().ok()?).ok()?;
    Date::from_calendar_date(year.parse().ok()?, month, day.parse().ok()?).ok()
}

fn format_utc(value: OffsetDateTime) -> Result<String, AppError> {
    value
        .format(&Rfc3339)
        .map_err(|error| BackendError::database(error.to_string()))
}

struct StoredSemester {
    calendar: SemesterCalendar,
    start_date: String,
    utc_offset_minutes: i32,
    revision: i64,
}

fn load(connection: &Connection) -> Result<Option<StoredSemester>, AppError> {
    let row: Option<(String, i32, i64, i64)> = connection
        .query_row(
            "SELECT start_date, utc_offset_minutes, week_count, revision FROM semester_settings WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    row.map(|(start_date, offset, weeks, revision)| {
        Ok(StoredSemester {
            calendar: SemesterCalendar::new(&start_date, offset, weeks)?,
            start_date,
            utc_offset_minutes: offset,
            revision,
        })
    })
    .transpose()
}

/// Календарь семестра активного пространства, если он настроен.
pub fn calendar(connection: &Connection) -> Result<Option<SemesterCalendar>, AppError> {
    Ok(load(connection)?.map(|stored| stored.calendar))
}

/// Даты новой недели: из семестра или неизвестны (`''`, `unknown`).
pub fn new_week_dates(
    connection: &Connection,
    number: i64,
) -> Result<(String, String, &'static str), AppError> {
    match calendar(connection)? {
        Some(calendar) => {
            let (starts, ends) = calendar.week_strings(number)?;
            Ok((starts, ends, "semester"))
        }
        None => Ok((String::new(), String::new(), "unknown")),
    }
}

fn dto(stored: &StoredSemester, now: OffsetDateTime) -> Result<SemesterDto, AppError> {
    let weeks = (1..=stored.calendar.week_count)
        .map(|number| {
            let (starts_at, ends_at) = stored.calendar.week_strings(number)?;
            Ok(SemesterWeekDto {
                number,
                starts_at,
                ends_at,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(SemesterDto {
        start_date: stored.start_date.clone(),
        utc_offset_minutes: stored.utc_offset_minutes,
        week_count: stored.calendar.week_count,
        revision: stored.revision,
        weeks,
        current_week: stored.calendar.week_at(now),
        warnings: stored.calendar.warnings(),
    })
}

pub struct SemesterService<'a> {
    database: &'a Database,
}

impl<'a> SemesterService<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn get(&self, now: OffsetDateTime) -> Result<Option<SemesterDto>, AppError> {
        self.database.with_connection(|connection| {
            load(connection)?
                .map(|stored| dto(&stored, now))
                .transpose()
        })
    }

    /// Сохраняет семестр и пересчитывает даты всех недель (кроме ничего — даты
    /// недель всегда производные). `expected_revision = None` — семестр ещё не задан.
    pub fn set(
        &self,
        input: &SemesterInput,
        expected_revision: Option<i64>,
        now: OffsetDateTime,
    ) -> Result<SemesterSaveResultDto, AppError> {
        let calendar = SemesterCalendar::new(
            &input.start_date,
            input.utc_offset_minutes,
            input.week_count,
        )?;
        let start_date = input.start_date.trim().to_owned();
        let updated = self.database.with_transaction(|transaction| {
            let current = load(transaction)?.map(|stored| stored.revision);
            if current != expected_revision {
                return Err(BackendError::conflict(
                    "Семестр изменился в другом окне. Обновите данные и повторите.",
                ));
            }
            transaction.execute(
                "INSERT INTO semester_settings(singleton,start_date,utc_offset_minutes,week_count,revision,updated_at)
                 VALUES (1,?1,?2,?3,0,strftime('%Y-%m-%dT%H:%M:%fZ','now'))
                 ON CONFLICT(singleton) DO UPDATE SET start_date=excluded.start_date,
                   utc_offset_minutes=excluded.utc_offset_minutes, week_count=excluded.week_count,
                   revision=semester_settings.revision+1, updated_at=excluded.updated_at",
                params![start_date, input.utc_offset_minutes, input.week_count],
            )?;
            let mut statement = transaction.prepare("SELECT DISTINCT number FROM weeks ORDER BY number")?;
            let numbers = statement
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let mut updated = 0_i64;
            for number in numbers {
                let (starts, ends) = calendar.week_strings(number)?;
                updated += transaction.execute(
                    "UPDATE weeks SET starts_at=?2, ends_at=?3, dates_source='semester',
                            updated_at=CURRENT_TIMESTAMP
                     WHERE number=?1",
                    params![number, starts, ends],
                )? as i64;
            }
            Ok(updated)
        })?;
        let semester = self
            .get(now)?
            .ok_or_else(|| BackendError::database("Семестр не сохранён"))?;
        Ok(SemesterSaveResultDto {
            semester,
            updated_weeks: updated,
        })
    }

    /// Точный срок ДЗ (RFC 3339 с часовым поясом, хранится в UTC) или «неизвестен».
    pub fn set_homework_deadline(
        &self,
        homework_id: &str,
        expected_revision: i64,
        deadline: Option<&str>,
    ) -> Result<HomeworkDto, AppError> {
        let deadline = normalize_deadline(deadline)?;
        self.database.with_transaction(|transaction| {
            let row: Option<(i64, String)> = transaction
                .query_row(
                    "SELECT revision, stage FROM homeworks WHERE id=?1",
                    [homework_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (revision, stage) = row.ok_or_else(|| BackendError::not_found("ДЗ не найдено"))?;
            if stage == "submitted" {
                return Err(BackendError::blocked(
                    "Сданное ДЗ неизменяемо: срок нельзя поменять",
                ));
            }
            if revision != expected_revision {
                return Err(BackendError::conflict(
                    "ДЗ изменилось в другом окне. Обновите данные и повторите.",
                ));
            }
            transaction.execute(
                "UPDATE homeworks SET deadline=?2,
                        deadline_source=CASE WHEN ?2 IS NULL THEN NULL ELSE 'manual' END,
                        revision=revision+1, updated_at=CURRENT_TIMESTAMP
                 WHERE id=?1",
                params![homework_id, deadline],
            )?;
            Ok(())
        })?;
        ReadRepository::new(self.database).homework(homework_id)
    }
}

#[cfg(test)]
#[path = "semester_tests.rs"]
mod tests;
