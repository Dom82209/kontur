use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::{SemesterCalendar, SemesterInput, SemesterService};
use crate::{
    db::Database,
    dto::{DeadlineSource, WeekDatesSource},
    error::ErrorCode,
    homework_registry::{HomeworkRegistry, NewHomeworkInput},
    repositories::ReadRepository,
};

fn at(value: &str) -> OffsetDateTime {
    OffsetDateTime::parse(value, &Rfc3339).unwrap()
}

fn database() -> Database {
    let database = Database::open_in_memory().unwrap();
    database.migrate().unwrap();
    database
        .with_connection(|c| {
            c.execute_batch(
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES ('stat','Статистика','Стат','Предметы/Статистика');
                 INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES
                   ('w1','stat',1,'','','Предметы/Статистика/Неделя 01'),
                   ('w8','stat',8,'2026-10-26T00:00:00Z','2026-11-01T23:59:59Z','Предметы/Статистика/Неделя 08');",
            )?;
            Ok(())
        })
        .unwrap();
    database
}

fn input(start: &str, offset: i32, weeks: i64) -> SemesterInput {
    SemesterInput {
        start_date: start.into(),
        utc_offset_minutes: offset,
        week_count: weeks,
    }
}

#[test]
fn week_boundaries_follow_local_midnight_and_cross_the_year() {
    // Москва (UTC+3): неделя 1 начинается 1 сентября 00:00 по Москве = 31 августа 21:00 UTC.
    let calendar = SemesterCalendar::new("2026-09-01", 180, 18).unwrap();
    assert_eq!(
        calendar.week_strings(1).unwrap(),
        (
            "2026-08-31T21:00:00Z".to_owned(),
            "2026-09-07T20:59:59Z".to_owned()
        )
    );
    // Граница года: неделя 18 — с 29 декабря 2026 по 4 января 2027 по Москве.
    assert_eq!(
        calendar.week_strings(18).unwrap(),
        (
            "2026-12-28T21:00:00Z".to_owned(),
            "2027-01-04T20:59:59Z".to_owned()
        )
    );
    assert_eq!(
        calendar.week_at(at("2026-08-31T20:59:59Z")),
        None,
        "до начала"
    );
    assert_eq!(calendar.week_at(at("2026-08-31T21:00:00Z")), Some(1));
    assert_eq!(calendar.week_at(at("2026-09-07T20:59:59Z")), Some(1));
    assert_eq!(calendar.week_at(at("2026-09-07T21:00:00Z")), Some(2));
    assert_eq!(calendar.week_at(at("2027-01-04T20:59:59Z")), Some(18));
    assert_eq!(
        calendar.week_at(at("2027-01-04T21:00:00Z")),
        None,
        "после окончания"
    );
    // Западное смещение: UTC−5.
    let west = SemesterCalendar::new("2027-01-04", -300, 2).unwrap();
    assert_eq!(west.week_strings(1).unwrap().0, "2027-01-04T05:00:00Z");
}

#[test]
fn invalid_semester_input_is_rejected() {
    for (start, offset, weeks) in [
        ("2026-02-30", 180, 18),
        ("01.09.2026", 180, 18),
        ("2026-09-01", 181, 18),
        ("2026-09-01", 900, 18),
        ("2026-09-01", 180, 0),
        ("2026-09-01", 180, 61),
    ] {
        assert!(
            SemesterCalendar::new(start, offset, weeks).is_err(),
            "{start} {offset} {weeks}"
        );
    }
}

#[test]
fn saving_the_semester_recomputes_week_dates_and_requires_revision() {
    let database = database();
    let service = SemesterService::new(&database);
    let now = at("2026-10-28T12:00:00Z");
    assert!(service.get(now).unwrap().is_none());
    let weeks = ReadRepository::new(&database).weeks(Some("stat")).unwrap();
    assert_eq!(weeks[0].dates_source, WeekDatesSource::Unknown);
    assert_eq!(
        weeks[1].dates_source,
        WeekDatesSource::Legacy,
        "даты до этапа 5 — приблизительные"
    );

    let saved = service
        .set(&input("2026-09-01", 180, 18), None, now)
        .unwrap();
    assert_eq!(saved.updated_weeks, 2);
    assert_eq!(saved.semester.current_week, Some(9));
    assert_eq!(saved.semester.weeks.len(), 18);
    assert!(saved
        .semester
        .warnings
        .iter()
        .any(|w| w.contains("не с понедельника")));
    let weeks = ReadRepository::new(&database).weeks(Some("stat")).unwrap();
    assert!(weeks
        .iter()
        .all(|w| w.dates_source == WeekDatesSource::Semester));
    assert_eq!(weeks[1].starts_at.as_deref(), Some("2026-10-19T21:00:00Z"));

    assert_eq!(
        service
            .set(&input("2026-08-31", 180, 18), None, now)
            .unwrap_err()
            .code,
        ErrorCode::Conflict,
        "второй раз нужна ревизия"
    );
    let updated = service
        .set(&input("2026-08-31", 180, 18), Some(0), now)
        .unwrap();
    assert_eq!(updated.semester.revision, 1);
    assert!(updated.semester.warnings.is_empty());
}

#[test]
fn deadline_is_exact_manual_or_unknown_and_submitted_is_immutable() {
    let database = database();
    let homework = HomeworkRegistry::new(&database)
        .create(&NewHomeworkInput {
            subject_id: "stat".into(),
            week_number: 8,
            title: "ДЗ 8".into(),
            deadline: None,
        })
        .unwrap();
    assert_eq!(homework.deadline, None);
    assert_eq!(homework.deadline_source, None);
    let service = SemesterService::new(&database);
    let set = service
        .set_homework_deadline(
            &homework.id,
            homework.revision,
            Some("2026-11-01T23:59:00+03:00"),
        )
        .unwrap();
    assert_eq!(set.deadline.as_deref(), Some("2026-11-01T20:59:00Z"));
    assert_eq!(set.deadline_source, Some(DeadlineSource::Manual));
    assert_eq!(
        service
            .set_homework_deadline(&homework.id, homework.revision, None)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let cleared = service
        .set_homework_deadline(&homework.id, set.revision, None)
        .unwrap();
    assert_eq!(cleared.deadline, None);
    assert_eq!(cleared.deadline_source, None);
    assert_eq!(
        service
            .set_homework_deadline(&homework.id, cleared.revision, Some("31.12.2026"))
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    database
        .with_connection(|c| {
            c.execute(
                "UPDATE homeworks SET stage='submitted', submitted_at='2026-11-01T10:00:00Z', workflow_stage='submitted' WHERE id=?1",
                [&homework.id],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        service
            .set_homework_deadline(&homework.id, cleared.revision, None)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn legacy_deadline_is_reported_as_legacy() {
    let database = database();
    database
        .with_connection(|c| {
            c.execute(
                "INSERT INTO homeworks(id,week_id,ordinal,title,deadline,stage,workflow_stage,folder_path)
                 VALUES ('old','w8',1,'ДЗ','2026-11-02T20:59:59Z','task','materials','Предметы/Статистика/Неделя 08/03 ДЗ')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let homework = ReadRepository::new(&database).homework("old").unwrap();
    assert_eq!(homework.deadline_source, Some(DeadlineSource::Legacy));
}

#[test]
fn new_weeks_get_semester_dates_and_dashboard_week_is_honest() {
    let database = database();
    let now = at("2026-10-28T12:00:00Z");
    let dashboard = |database: &Database| {
        crate::workflow::projections::academic_week(
            database
                .with_connection(crate::semester::calendar)
                .unwrap()
                .as_ref(),
            now,
        )
        .unwrap()
    };
    assert_eq!(
        dashboard(&database),
        None,
        "без семестра номер недели не выдумывается"
    );
    SemesterService::new(&database)
        .set(&input("2026-09-01", 180, 18), None, now)
        .unwrap();
    assert_eq!(dashboard(&database).unwrap().number, 9);
    let (starts, ends, source) = database
        .with_connection(|c| crate::semester::new_week_dates(c, 10))
        .unwrap();
    assert_eq!(
        (starts.as_str(), ends.as_str(), source),
        ("2026-11-02T21:00:00Z", "2026-11-09T20:59:59Z", "semester")
    );
}
