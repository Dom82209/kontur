use uuid::Uuid;

use crate::{
    db::Database,
    dto::{
        AutomationRulesDto, ClassificationDto, ClassificationPatchDto, ClassificationSource,
        InboxFileDto, InboxStatus, MaterialCategory, OperationDto, OperationKind, PatchField,
    },
    error::{AppError, BackendError},
    events::{InboxClassificationUpdatedPayload, OperationCompletedPayload},
    query_service::QueryService,
    repositories::{writes, ReadRepository},
};

#[derive(Debug, Clone)]
pub enum MutationEvent {
    InboxClassificationUpdated(InboxClassificationUpdatedPayload),
    OperationCompleted(OperationCompletedPayload),
}

#[derive(Debug, Clone)]
pub struct MutationSuccess<T> {
    pub value: T,
    pub operation: OperationDto,
    pub events: Vec<MutationEvent>,
}

#[derive(Debug, Clone)]
pub struct MutationFailure {
    pub error: AppError,
    pub operation: Option<Box<OperationDto>>,
}

pub struct MutationService<'a> {
    database: &'a Database,
}

impl<'a> MutationService<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn update_automation_rules(
        &self,
        rules: AutomationRulesDto,
    ) -> Result<MutationSuccess<AutomationRulesDto>, MutationFailure> {
        let operation_id = Uuid::new_v4().to_string();
        let kind = OperationKind::UpdateRules;
        let title = "Обновление правил автоматизации";
        let result = self.database.with_transaction(|transaction| {
            writes::insert_running_operation(transaction, &operation_id, kind, title)?;
            writes::upsert_automation_rules(transaction, &rules)?;
            writes::complete_operation(transaction, &operation_id)?;
            Ok(())
        });
        if let Err(error) = result {
            return Err(self.record_failure(operation_id, kind, title, error));
        }

        let value = QueryService::new(self.database)
            .automation_rules()
            .map_err(post_commit_failure)?;
        let operation = ReadRepository::new(self.database)
            .operation(&operation_id)
            .map_err(post_commit_failure)?;
        Ok(MutationSuccess {
            value,
            operation: operation.clone(),
            events: vec![MutationEvent::OperationCompleted(
                OperationCompletedPayload { operation },
            )],
        })
    }

    pub fn update_classification(
        &self,
        file_id: &str,
        patch: ClassificationPatchDto,
    ) -> Result<MutationSuccess<InboxFileDto>, MutationFailure> {
        let operation_id = Uuid::new_v4().to_string();
        let kind = OperationKind::Classify;
        let title = "Изменение классификации";
        let result = self.database.with_transaction(|transaction| {
            writes::insert_running_operation(transaction, &operation_id, kind, title)?;
            let stored = writes::read_inbox_classification(transaction, file_id)?;
            if stored.status == InboxStatus::Routed {
                return Err(BackendError::conflict(
                    "routed inbox file classification cannot be changed",
                ));
            }
            let mut classification = stored.classification.unwrap_or(ClassificationDto {
                homework_id: None,
                subject_id: None,
                week_number: None,
                category: MaterialCategory::Unknown,
                confidence: 0.0,
                reasons: Vec::new(),
                source: ClassificationSource::Manual,
            });
            if !(0.0..=1.0).contains(&classification.confidence) {
                return Err(BackendError::database(
                    "stored classification confidence is outside 0..1",
                ));
            }

            apply_optional_patch(&mut classification.subject_id, patch.subject_id);
            apply_optional_patch(&mut classification.week_number, patch.week_number);
            match patch.category {
                PatchField::Absent => {}
                PatchField::Null => classification.category = MaterialCategory::Unknown,
                PatchField::Value(category) => classification.category = category,
            }
            let homework_explicit = matches!(patch.homework_id, PatchField::Value(_));
            apply_optional_patch(&mut classification.homework_id, patch.homework_id);

            if let Some(subject_id) = classification.subject_id.as_deref() {
                if !writes::subject_exists(transaction, subject_id)? {
                    return Err(BackendError::not_found("subject not found"));
                }
                if writes::subject_archived(transaction, subject_id)? {
                    return Err(BackendError::blocked(
                        "Предмет в архиве: верните его из архива или выберите другой",
                    ));
                }
                if let Some(week_number) = classification.week_number {
                    if !writes::week_exists_for_subject(transaction, subject_id, week_number)? {
                        return Err(BackendError::conflict(
                            "week does not belong to selected subject",
                        ));
                    }
                }
            }
            // Этап 4: выбранное ДЗ должно принадлежать неделе и подходить к категории.
            if let Some(homework_id) = classification.homework_id.clone() {
                let fits = matches!(
                    classification.category,
                    MaterialCategory::HomeworkTask
                        | MaterialCategory::HomeworkData
                        | MaterialCategory::HomeworkDraft
                ) && match (
                    classification.subject_id.as_deref(),
                    classification.week_number,
                ) {
                    (Some(subject_id), Some(week_number)) => writes::homework_in_week(
                        transaction,
                        &homework_id,
                        subject_id,
                        week_number,
                    )?,
                    _ => false,
                };
                if !fits {
                    if homework_explicit {
                        return Err(BackendError::conflict(
                            "Выбранное ДЗ не относится к неделе или категории файла",
                        ));
                    }
                    // Неделя или категория изменились: прежний выбор ДЗ больше не действует.
                    classification.homework_id = None;
                }
            }

            classification.confidence = 1.0;
            classification.source = ClassificationSource::Manual;
            classification.reasons = vec!["Классификация изменена вручную".to_owned()];
            let status = if classification.subject_id.is_some()
                && classification.week_number.is_some()
                && classification.category != MaterialCategory::Unknown
            {
                InboxStatus::Classified
            } else {
                InboxStatus::Ambiguous
            };
            writes::update_inbox_classification(transaction, file_id, &classification, status)?;
            writes::complete_operation(transaction, &operation_id)?;
            Ok(())
        });
        if let Err(error) = result {
            return Err(self.record_failure(operation_id, kind, title, error));
        }

        let value = ReadRepository::new(self.database)
            .inbox_file(file_id)
            .map_err(post_commit_failure)?;
        let operation = ReadRepository::new(self.database)
            .operation(&operation_id)
            .map_err(post_commit_failure)?;
        let classification = value
            .classification
            .clone()
            .ok_or_else(|| MutationFailure {
                error: BackendError::database("updated inbox file has no classification"),
                operation: None,
            })?;
        Ok(MutationSuccess {
            events: vec![
                MutationEvent::InboxClassificationUpdated(InboxClassificationUpdatedPayload {
                    file_id: value.id.clone(),
                    classification,
                    status: value.status,
                }),
                MutationEvent::OperationCompleted(OperationCompletedPayload {
                    operation: operation.clone(),
                }),
            ],
            value,
            operation,
        })
    }

    fn record_failure(
        &self,
        operation_id: String,
        kind: OperationKind,
        title: &str,
        error: AppError,
    ) -> MutationFailure {
        let recorded = self.database.with_transaction(|transaction| {
            writes::insert_failed_operation(transaction, &operation_id, kind, title, &error)
        });
        let operation = recorded.ok().and_then(|()| {
            ReadRepository::new(self.database)
                .operation(&operation_id)
                .ok()
                .map(Box::new)
        });
        MutationFailure { error, operation }
    }
}

fn apply_optional_patch<T>(target: &mut Option<T>, patch: PatchField<T>) {
    match patch {
        PatchField::Absent => {}
        PatchField::Null => *target = None,
        PatchField::Value(value) => *target = Some(value),
    }
}

fn post_commit_failure(error: AppError) -> MutationFailure {
    MutationFailure {
        error: AppError::new(
            error.code,
            "Данные сохранены, но результат операции не удалось прочитать",
        ),
        operation: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::{
        db::{migrations::CURRENT_SCHEMA_VERSION, Database},
        domain::OperationStatus,
        dto::{
            AutomationRulesDto, ClassificationPatchDto, InboxStatus, MaterialCategory,
            OperationKind, PatchField,
        },
        error::ErrorCode,
        events::InboxClassificationUpdatedPayload,
        query_service::QueryService,
    };

    use super::{MutationEvent, MutationService};

    fn database() -> Database {
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .with_connection(|connection| {
                connection.execute_batch(
                    r#"
                    INSERT INTO subjects(id, name, short_name, folder_path)
                    VALUES
                      ('subject-1', 'ОБА', 'ОБА', 'Предметы/ОБА'),
                      ('subject-2', 'Статистика', 'Стат', 'Предметы/Статистика');
                    INSERT INTO weeks(id, subject_id, number, starts_at, ends_at, folder_path)
                    VALUES
                      ('week-1', 'subject-1', 7, '2026-10-19T00:00:00Z', '2026-10-25T23:59:59Z', 'Предметы/ОБА/Неделя 07'),
                      ('week-2', 'subject-2', 8, '2026-10-26T00:00:00Z', '2026-11-01T23:59:59Z', 'Предметы/Статистика/Неделя 08');
                    INSERT INTO inbox_files(
                      id, name, kind, size_bytes, added_at, origin, status, relative_path,
                      classification_json
                    ) VALUES
                      ('inbox-1', 'task.pdf', 'pdf', 42, '2026-10-21T10:00:00Z', 'drop', 'classified',
                       'Предметы/00 Входящие/task.pdf',
                       '{"subjectId":"subject-1","weekNumber":7,"category":"homework_task","confidence":0.8,"reasons":["авто"],"source":"auto"}'),
                      ('routed-1', 'routed.pdf', 'pdf', 24, '2026-10-21T09:00:00Z', 'drop', 'routed',
                       'Предметы/00 Входящие/routed.pdf',
                       '{"subjectId":"subject-1","weekNumber":7,"category":"lecture","confidence":1.0,"reasons":["вручную"],"source":"manual"}');
                    "#,
                )?;
                Ok(())
            })
            .unwrap();
        database
    }

    fn patch(
        subject_id: PatchField<String>,
        week_number: PatchField<i64>,
        category: PatchField<MaterialCategory>,
    ) -> ClassificationPatchDto {
        ClassificationPatchDto {
            subject_id,
            week_number,
            category,
            homework_id: PatchField::Absent,
        }
    }

    fn absent_patch() -> ClassificationPatchDto {
        patch(PatchField::Absent, PatchField::Absent, PatchField::Absent)
    }

    #[test]
    fn saves_and_rehydrates_automation_rules_with_completed_operation() {
        let database = database();
        let mut rules = AutomationRulesDto::default();
        rules.workflow.style_threshold = 6;
        let success = MutationService::new(&database)
            .update_automation_rules(rules.clone())
            .unwrap();

        assert_eq!(success.value, rules);
        assert_eq!(
            QueryService::new(&database)
                .automation_rules()
                .unwrap()
                .workflow
                .style_threshold,
            6
        );
        assert_eq!(success.operation.kind, OperationKind::UpdateRules);
        assert_eq!(success.operation.status, OperationStatus::Completed);
        assert!(!success.operation.undoable);
        assert!(success.operation.paths.is_empty());
        assert!(matches!(
            success.events.as_slice(),
            [MutationEvent::OperationCompleted(_)]
        ));
    }

    #[test]
    fn rejects_confirmation_invariants_without_partial_change() {
        let database = database();
        let service = MutationService::new(&database);
        let mut saved = AutomationRulesDto::default();
        saved.workflow.style_threshold = 5;
        service.update_automation_rules(saved.clone()).unwrap();

        let mut invalid = saved.clone();
        invalid.confirm.mark_submitted = false;
        let failure = service.update_automation_rules(invalid).unwrap_err();
        assert_eq!(failure.error.code, ErrorCode::Database);
        assert_eq!(
            QueryService::new(&database).automation_rules().unwrap(),
            saved
        );
        assert_eq!(failure.operation.unwrap().status, OperationStatus::Failed);

        let mut overwrite = saved.clone();
        overwrite.confirm.allow_overwrite = true;
        assert!(service.update_automation_rules(overwrite).is_err());
    }

    #[test]
    fn rejects_thresholds_outside_contract_ranges() {
        let database = database();
        let service = MutationService::new(&database);
        let mut style = AutomationRulesDto::default();
        style.workflow.style_threshold = 11;
        assert!(service.update_automation_rules(style).is_err());

        let confidence = AutomationRulesDto {
            low_confidence_threshold: 1.1,
            ..AutomationRulesDto::default()
        };
        assert!(service.update_automation_rules(confidence).is_err());
    }

    #[test]
    fn typed_rules_reject_unknown_secret_like_fields() {
        let mut value = serde_json::to_value(AutomationRulesDto::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("password".to_owned(), json!("secret"));
        let error = serde_json::from_value::<AutomationRulesDto>(value).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn classification_patch_distinguishes_subject_absent_null_and_value() {
        let database = database();
        let service = MutationService::new(&database);
        let unchanged = service
            .update_classification("inbox-1", absent_patch())
            .unwrap()
            .value;
        assert_eq!(
            unchanged.classification.unwrap().subject_id.as_deref(),
            Some("subject-1")
        );

        let cleared = service
            .update_classification(
                "inbox-1",
                patch(PatchField::Null, PatchField::Absent, PatchField::Absent),
            )
            .unwrap()
            .value;
        assert_eq!(cleared.classification.unwrap().subject_id, None);

        let selected = service
            .update_classification(
                "inbox-1",
                patch(
                    PatchField::Value("subject-1".to_owned()),
                    PatchField::Absent,
                    PatchField::Absent,
                ),
            )
            .unwrap()
            .value;
        assert_eq!(
            selected.classification.unwrap().subject_id.as_deref(),
            Some("subject-1")
        );
    }

    #[test]
    fn classification_patch_distinguishes_week_absent_null_and_value() {
        let database = database();
        let service = MutationService::new(&database);
        let unchanged = service
            .update_classification("inbox-1", absent_patch())
            .unwrap()
            .value;
        assert_eq!(unchanged.classification.unwrap().week_number, Some(7));

        let cleared = service
            .update_classification(
                "inbox-1",
                patch(PatchField::Absent, PatchField::Null, PatchField::Absent),
            )
            .unwrap()
            .value;
        assert_eq!(cleared.classification.unwrap().week_number, None);

        let selected = service
            .update_classification(
                "inbox-1",
                patch(PatchField::Absent, PatchField::Value(7), PatchField::Absent),
            )
            .unwrap()
            .value;
        assert_eq!(selected.classification.unwrap().week_number, Some(7));
    }

    #[test]
    fn updates_category_or_preserves_it_when_absent() {
        let database = database();
        let service = MutationService::new(&database);
        let preserved = service
            .update_classification("inbox-1", absent_patch())
            .unwrap()
            .value;
        assert_eq!(
            preserved.classification.unwrap().category,
            MaterialCategory::HomeworkTask
        );

        let changed = service
            .update_classification(
                "inbox-1",
                patch(
                    PatchField::Absent,
                    PatchField::Absent,
                    PatchField::Value(MaterialCategory::Lecture),
                ),
            )
            .unwrap()
            .value;
        assert_eq!(
            changed.classification.unwrap().category,
            MaterialCategory::Lecture
        );
    }

    #[test]
    fn manual_update_recalculates_source_confidence_and_status() {
        let database = database();
        let service = MutationService::new(&database);
        let classified = service
            .update_classification("inbox-1", absent_patch())
            .unwrap()
            .value;
        let classification = classified.classification.unwrap();
        assert_eq!(classified.status, InboxStatus::Classified);
        assert_eq!(
            serde_json::to_value(classification.source).unwrap(),
            "manual"
        );
        assert_eq!(classification.confidence, 1.0);
        assert_eq!(classification.reasons, ["Классификация изменена вручную"]);

        let ambiguous = service
            .update_classification(
                "inbox-1",
                patch(PatchField::Absent, PatchField::Null, PatchField::Absent),
            )
            .unwrap()
            .value;
        assert_eq!(ambiguous.status, InboxStatus::Ambiguous);
    }

    #[test]
    fn rejects_unknown_subject_and_week_from_another_subject() {
        let database = database();
        let service = MutationService::new(&database);
        let unknown = service.update_classification(
            "inbox-1",
            patch(
                PatchField::Value("missing".to_owned()),
                PatchField::Null,
                PatchField::Absent,
            ),
        );
        assert_eq!(unknown.unwrap_err().error.code, ErrorCode::NotFound);

        let wrong_week = service.update_classification(
            "inbox-1",
            patch(
                PatchField::Value("subject-1".to_owned()),
                PatchField::Value(8),
                PatchField::Absent,
            ),
        );
        assert_eq!(wrong_week.unwrap_err().error.code, ErrorCode::Conflict);
    }

    #[test]
    fn routed_inbox_file_is_immutable_for_classification_update() {
        let database = database();
        let before = QueryService::new(&database).inbox_files().unwrap();
        let failure = MutationService::new(&database)
            .update_classification("routed-1", absent_patch())
            .unwrap_err();
        assert_eq!(failure.error.code, ErrorCode::Conflict);
        let after = QueryService::new(&database).inbox_files().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn successful_classification_creates_completed_operation_and_ordered_events() {
        let database = database();
        let success = MutationService::new(&database)
            .update_classification("inbox-1", absent_patch())
            .unwrap();
        assert_eq!(success.operation.kind, OperationKind::Classify);
        assert_eq!(success.operation.status, OperationStatus::Completed);
        assert!(!success.operation.undoable);
        assert!(success.operation.paths.is_empty());
        assert!(matches!(
            success.events.as_slice(),
            [
                MutationEvent::InboxClassificationUpdated(_),
                MutationEvent::OperationCompleted(_)
            ]
        ));
    }

    #[test]
    fn failed_mutation_creates_one_failed_operation_without_partial_update() {
        let database = database();
        let before = QueryService::new(&database).inbox_files().unwrap();
        let failure = MutationService::new(&database)
            .update_classification(
                "inbox-1",
                patch(
                    PatchField::Value("missing".to_owned()),
                    PatchField::Absent,
                    PatchField::Absent,
                ),
            )
            .unwrap_err();
        let failed = failure.operation.unwrap();
        assert_eq!(failed.status, OperationStatus::Failed);
        assert_eq!(failed.kind, OperationKind::Classify);
        assert_eq!(before, QueryService::new(&database).inbox_files().unwrap());
        let matching = QueryService::new(&database)
            .activity_log()
            .unwrap()
            .into_iter()
            .filter(|operation| operation.id == failed.id)
            .count();
        assert_eq!(matching, 1);
    }

    #[test]
    fn event_payload_is_camel_case_and_exists_only_after_committed_state() {
        let database = database();
        let success = MutationService::new(&database)
            .update_classification("inbox-1", absent_patch())
            .unwrap();
        let stored = QueryService::new(&database).inbox_files().unwrap();
        assert_eq!(
            stored[0].classification.as_ref().unwrap().source,
            crate::dto::ClassificationSource::Manual
        );
        let payload = match &success.events[0] {
            MutationEvent::InboxClassificationUpdated(payload) => payload,
            MutationEvent::OperationCompleted(_) => panic!("classification event must be first"),
        };
        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value["fileId"], "inbox-1");
        assert!(value.get("file_id").is_none());

        let failed = MutationService::new(&database).update_classification(
            "inbox-1",
            patch(
                PatchField::Value("missing".to_owned()),
                PatchField::Absent,
                PatchField::Absent,
            ),
        );
        assert!(failed.is_err());
    }

    #[test]
    fn activity_log_order_is_deterministic_and_payloads_have_no_absolute_paths() {
        let database = database();
        let service = MutationService::new(&database);
        service
            .update_automation_rules(AutomationRulesDto::default())
            .unwrap();
        let success = service
            .update_classification("inbox-1", absent_patch())
            .unwrap();
        let log = QueryService::new(&database).activity_log().unwrap();
        let mut expected = log.clone();
        expected.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        assert_eq!(log, expected);
        let event_values: Vec<Value> = success
            .events
            .into_iter()
            .map(|event| match event {
                MutationEvent::InboxClassificationUpdated(payload) => {
                    serde_json::to_value(payload).unwrap()
                }
                MutationEvent::OperationCompleted(payload) => {
                    serde_json::to_value(payload).unwrap()
                }
            })
            .collect();
        let serialized = serde_json::to_string(&(success.value, event_values)).unwrap();
        assert!(!serialized.contains("sourcePath"));
        assert!(!serialized.contains("/Users/"));
        assert!(!serialized.contains("file://"));
    }

    #[test]
    fn schema_version_includes_file_workflow_tables() {
        let database = database();
        const { assert!(CURRENT_SCHEMA_VERSION >= 10) };
        assert_eq!(database.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn tri_state_wire_format_is_explicit() {
        let value: Value = serde_json::to_value(patch(
            PatchField::Absent,
            PatchField::Null,
            PatchField::Value(MaterialCategory::Seminar),
        ))
        .unwrap();
        assert_eq!(value["subjectId"], json!({ "state": "absent" }));
        assert_eq!(value["weekNumber"], json!({ "state": "null" }));
        assert_eq!(
            value["category"],
            json!({ "state": "value", "value": "seminar" })
        );
    }

    #[test]
    fn classification_event_payload_reuses_exact_dto() {
        let payload = InboxClassificationUpdatedPayload {
            file_id: "inbox-1".to_owned(),
            classification: crate::dto::ClassificationDto {
                homework_id: None,
                subject_id: Some("subject-1".to_owned()),
                week_number: Some(7),
                category: MaterialCategory::Lecture,
                confidence: 1.0,
                reasons: vec!["Классификация изменена вручную".to_owned()],
                source: crate::dto::ClassificationSource::Manual,
            },
            status: InboxStatus::Classified,
        };
        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value["classification"]["subjectId"], "subject-1");
        assert_eq!(value["status"], "classified");
    }
}
