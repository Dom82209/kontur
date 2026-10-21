# Контракт постоянного хранения

Документ сверяет фактические типы `src/lib/bridge/types.ts`, `workflow-types.ts`, Rust DTO и SQLite schema version 2. SQLite доступен только Rust backend; read-only DTO уже доступны через Tauri IPC, но React пока работает через `MockDesktopBridge`.

## Соответствие сущностей

| TypeScript | Rust DTO/domain | Таблица/колонки | Nullable | JSON | Вычисляется | Источник истины |
|---|---|---|---|---|---|---|
| `AutomationRules`, `WorkflowSettings` | `AutomationRulesDto` и вложенные typed DTO | `app_settings` | отсутствующие безопасные поля дополняются defaults | весь тип в `value_json` | merge с defaults | `app_settings` |
| `Subject` | `SubjectDto` | `subjects`: `id`, `name`, `short_name` | нет | — | `currentWeek`, `weekCount` из `weeks` | `subjects` + агрегат `weeks` |
| `Week` | `WeekDto` | `weeks`: `id`, `subject_id`, `number`, `starts_at`, `ends_at`, `topic` | `topic`, `homeworkId` | — | `lectures`, `seminars`, `materials` из `files`; `homeworkId` из `homeworks` | `weeks`, `files`, `homeworks` |
| `FileRef` | `FileRefDto` | `files`: `id`, `name`, `kind`, `size_bytes`, `modified_at`, `relative_path` | legacy `modified_at` | — | timestamp fallback из `updated_at`/`created_at` | `files` |
| `InboxFile` | `InboxFileDto` | `inbox_files` | `classification`, `duplicate` | typed `classification_json`, `duplicate_json` | persistent `relative_path` не выдаётся | `inbox_files` |
| `Homework` | `HomeworkDto` | `homeworks` + связи с `weeks`, `files`, `homework_versions` | `final_version_id`, `final_file_name`, `submitted_at`, `assignment_url`; legacy `deadline` | — | `subjectId`, `weekNumber`, `taskFiles`, `versions` через связи | `homeworks` и дочерние таблицы |
| `HomeworkVersion` | `HomeworkVersionDto` | `homework_versions` | `note`, `prompt_id`, `source_bundle_id` | typed `sections_json`, `files_json`, legacy `metadata_json` | legacy file fallback | immutable row `homework_versions` |
| `SourceBundle` | `SourceBundle`, `SourceBundleItem` | `source_bundles`, `source_bundle_items` | `version_id` | `missing_json`; FileRef элемента восстанавливается join с `files` | `items` через join | обе таблицы |
| `ReviewRun` | `ReviewRun` | `review_runs` | `version_id`, `source_bundle_id`, `finished_at`, `result_json`, `error_message`, `style_score` | `sources_used_json`, `result_json` | — | `review_runs` |
| `RevisionPrompt` | `RevisionPrompt` | `revision_prompts` | `based_on_version_id`, `review_run_id`, `sent_at` | `addresses_json`, `sources_json` | — | `revision_prompts` |
| `TeacherComment` | `TeacherComment` | `teacher_comments` | `attachment_file_id`; legacy `source`/`date` | — | `attachment` через join с `files` | `teacher_comments`, `files` |
| `TeacherRequirement` | `TeacherRequirement` | `teacher_requirements` | только legacy `comment_id`, `checked_version_id`; новые записи обязаны иметь оба | — | — | `teacher_requirements` |
| `SubmissionPackage` | `SubmissionPackage` | `submission_packages` | нет для новых записей | `items_json`, `links_json`, `warnings_json`, `checklist_json`, `upload_list_json` | — | `submission_packages` |
| `SubmissionReceipt` | `SubmissionReceipt` | `submission_receipts` | `proof_json` | `proof_json`, `files_json` | — | immutable row `submission_receipts` |
| `Operation` | `OperationDto` | `operations` | `detail`, `finished_at`, `backup_path`, `homework_id`, `error_json`, `undo_payload_json` | typed `paths_json`; internal error/undo payload не выдаётся | status normalization | `operations` |
| `StyleProfile`, `ProviderStatus` | пока JSON-настройки/будущий service DTO | `app_settings` хранит только безопасные preference/metadata | контрактные nullable-поля | профиль и безопасная metadata | runtime connection state не восстанавливается как подключённый | settings + будущий provider service |

## Enum-преобразования

`HomeworkStage` сериализуется во frontend исчерпывающим Rust enum:

| Frontend/Rust | Внутреннее значение v1/v2 DB |
|---|---|
| `task` | `task` |
| `in_progress` | `in_progress` |
| `ready` | `ready_to_submit` |
| `submitted` | `submitted` |

`HomeworkWorkflowStage` также не передаётся raw string:

| Frontend/Rust | Внутреннее значение DB |
|---|---|
| `materials_collected` | `materials` |
| `codex_analysis` | `analysis` |
| `prompt_ready` | `prompt` |
| `claude_solving` | `solution` |
| `codex_review` | `review` |
| `needs_fixes` | `revision` |
| `teacher_review` | `teacher` |
| `ready_to_submit` | `ready_to_submit` |
| `submitted` | `submitted` |

Оба направления реализованы через `as_db_str`/`try_from_db` и покрыты всеми вариантами. `RequirementStatus` хранится контрактно: `done`, `not_done`, `manual`, `na`; миграция преобразует v1 `met`, `unmet`, `manual_check`, `not_applicable`.

`OperationStatus` также типизирован: legacy `started` и `running` восстанавливаются как frontend `running`, `failed` и legacy `cancelled` — как `failed`, `completed` и `undone` совпадают напрямую.

Остальные enum-поля read-only DTO (`FileKind`, inbox origin/status/classification, operation kind, provider roles, week detection) преобразуются typed Rust enum и сериализуются в строковые значения TypeScript union.

## JSON и валидация

JSON используется только для вложенных неизменяемых payload, где отдельные связи не теряются: sections/files версии, missing sources, evidence/result проверки, addresses/sources промта, package items/links/checklist, proof/files receipt, classification/duplicate и operation payload. Все JSON-колонки защищены `json_valid`; репозитории сериализуют `serde_json::Value`, а не принимают произвольный SQL-текст.

`app_settings` разрешён для UI preferences, automation/workflow rules, путей конфигурации, шаблонов имён и безопасной provider metadata. Repository рекурсивно отклоняет ключи, похожие на API keys, passwords, cookies, access/refresh tokens и session secrets. Содержимое секретов не должно попадать ни в SQLite, ни в operation error/undo payload.

## Пути и native intake

Постоянные пути учебных файлов относительны корню хранилища. Inbox хранит только путь внутри `Предметы/00 Входящие/`. `DroppedItem.sourcePath` может быть абсолютным лишь как краткоживущий аргумент native intake до безопасного копирования и в SQLite не сохраняется. Repository проверяет `..`, абсолютные Unix/Windows-пути и drive prefix, включая path-поля вложенного JSON; SQL дополнительно ограничивает inbox/operation paths и базовые абсолютные варианты в path-колонках v1.

## Не сохраняемые представления

`Dashboard`, `Deadline`, `Readiness`, `HomeworkIteration` и `VersionComparison` являются проекциями. Они детерминированно строятся из таблиц, а не имеют собственных строк. `RoutePreview`, `DroppedItem`, `SubmissionConfirmation` — входные/предварительные DTO; подтверждение материализуется в immutable `submission_receipts`. Визуальные состояния вкладок, раскрытий, загрузки и анимаций не сохраняются.

## AutomationRules defaults

Если `automation_rules`/legacy `automation` отсутствует либо содержит только часть безопасных полей, чтение дополняет значения: inbox `Предметы/00 Входящие`, downloads `Загрузки`, vault `.`, имя `Не задано`/`Не задано`, шаблоны `{Фамилия}_{Имя}_Неделя{Неделя} ДЗ` и `Работа_Неделя{Неделя}_v{Версия}`, определение недели `filename_then_calendar`, semester start `1970-01-05T00:00:00Z`, пустые subject rules, порог классификации `0.7`, backup включён, auto-route выключен. Confirm defaults: low-confidence route, final и submitted требуют подтверждения; overwrite запрещён. Workflow defaults: роли `codex/claude/codex`, style threshold `4`, профиль `ПРОФИЛЬ_СТИЛЯ_ДЗ.md`, teacher review включён. Ключи, похожие на password/cookie/API/access/refresh/session secret, отклоняются.

## Legacy v1

v1 уже неизменяема. v2 добавляет недостающие колонки, перестраивает только таблицы, где нужны nullable FK или новые enum constraints, и копирует старые строки транзакционно. Поля, которых в v1 не существовало, получают безопасные детерминированные legacy-значения (`short_name = name`, даты недели из `created_at`, пустые валидные JSON-массивы).

При read hydration nullable `topic`, `finalVersionId`, `finalFileName`, `submittedAt`, `assignmentUrl`, `promptId` и `sourceBundleId` сохраняются как `null`. Пустые legacy `HomeworkVersion.fileName` и `sourceFileId` восстанавливаются из basename относительного `file_path` и стабильного `legacy:<versionId>`; пустой `files_json` даёт один FileRef той же frozen-версии. Отсутствующий обязательный deadline, неизвестный enum, повреждённый JSON или небезопасный путь не маскируются пустой строкой: возвращается структурированная ошибка `database` без panic.
