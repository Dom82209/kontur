# Контракт постоянного хранения

Документ сверяет фактические типы `src/lib/bridge/types.ts`, `workflow-types.ts`, Rust domain и SQLite schema version 2. SQLite доступен только Rust backend; React получает восстановленные DTO через будущий `TauriDesktopBridge`.

## Соответствие сущностей

| TypeScript | Rust DTO/domain | Таблица/колонки | Nullable | JSON | Вычисляется | Источник истины |
|---|---|---|---|---|---|---|
| `AutomationRules`, `WorkflowSettings` | `serde_json::Value` до появления команды | `app_settings` | — | весь тип в `value_json` | — | `app_settings` |
| `Subject` | `Subject` | `subjects`: `id`, `name`, `short_name` | нет | — | `currentWeek`, `weekCount` из `weeks` | `subjects` + агрегат `weeks` |
| `Week` | `Week` | `weeks`: `id`, `subject_id`, `number`, `starts_at`, `ends_at`, `topic` | `topic` | — | `lectures`, `seminars`, `materials` из `files`; `homeworkId` из `homeworks` | `weeks`, `files`, `homeworks` |
| `FileRef` | `FileRef` | `files`: `id`, `name`, `kind`, `size_bytes`, `modified_at`, `relative_path` | legacy `modified_at` | — | — | `files` |
| `InboxFile` | `InboxFile` | `inbox_files` | `classification`, `duplicate` | `classification_json`, `duplicate_json` | — | `inbox_files` |
| `Homework` | `Homework` | `homeworks` + связи с `weeks`, `files`, `homework_versions` | `final_version_id`, `final_file_name`, `submitted_at`, `assignment_url`; legacy `deadline` | — | `subjectId`, `weekNumber`, `taskFiles`, `versions` через связи | `homeworks` и дочерние таблицы |
| `HomeworkVersion` | `HomeworkVersion` | `homework_versions` | `note`, `prompt_id`, `source_bundle_id` | `sections_json`, `files_json`, legacy `metadata_json` | — | immutable row `homework_versions` |
| `SourceBundle` | `SourceBundle`, `SourceBundleItem` | `source_bundles`, `source_bundle_items` | `version_id` | `missing_json`; FileRef элемента восстанавливается join с `files` | `items` через join | обе таблицы |
| `ReviewRun` | `ReviewRun` | `review_runs` | `version_id`, `source_bundle_id`, `finished_at`, `result_json`, `error_message`, `style_score` | `sources_used_json`, `result_json` | — | `review_runs` |
| `RevisionPrompt` | `RevisionPrompt` | `revision_prompts` | `based_on_version_id`, `review_run_id`, `sent_at` | `addresses_json`, `sources_json` | — | `revision_prompts` |
| `TeacherComment` | `TeacherComment` | `teacher_comments` | `attachment_file_id`; legacy `source`/`date` | — | `attachment` через join с `files` | `teacher_comments`, `files` |
| `TeacherRequirement` | `TeacherRequirement` | `teacher_requirements` | только legacy `comment_id`, `checked_version_id`; новые записи обязаны иметь оба | — | — | `teacher_requirements` |
| `SubmissionPackage` | `SubmissionPackage` | `submission_packages` | нет для новых записей | `items_json`, `links_json`, `warnings_json`, `checklist_json`, `upload_list_json` | — | `submission_packages` |
| `SubmissionReceipt` | `SubmissionReceipt` | `submission_receipts` | `proof_json` | `proof_json`, `files_json` | — | immutable row `submission_receipts` |
| `Operation` | `OperationRecord` | `operations` | `detail`, `finished_at`, `backup_path`, `homework_id`, `error_json`, `undo_payload_json` | `paths_json`, `error_json`, `undo_payload_json` | — | `operations` |
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

Остальные enum-поля v2 (`purpose`, provider/role/status, prompt kind, inbox kind/origin/status) используют ровно строковые значения TypeScript union и ограничены SQL `CHECK`. До Tauri-команд они не выдаются frontend напрямую.

## JSON и валидация

JSON используется только для вложенных неизменяемых payload, где отдельные связи не теряются: sections/files версии, missing sources, evidence/result проверки, addresses/sources промта, package items/links/checklist, proof/files receipt, classification/duplicate и operation payload. Все JSON-колонки защищены `json_valid`; репозитории сериализуют `serde_json::Value`, а не принимают произвольный SQL-текст.

`app_settings` разрешён для UI preferences, automation/workflow rules, путей конфигурации, шаблонов имён и безопасной provider metadata. Repository рекурсивно отклоняет ключи, похожие на API keys, passwords, cookies, access/refresh tokens и session secrets. Содержимое секретов не должно попадать ни в SQLite, ни в operation error/undo payload.

## Пути и native intake

Постоянные пути учебных файлов относительны корню хранилища. Inbox хранит только путь внутри `Предметы/00 Входящие/`. `DroppedItem.sourcePath` может быть абсолютным лишь как краткоживущий аргумент native intake до безопасного копирования и в SQLite не сохраняется. Repository проверяет `..`, абсолютные Unix/Windows-пути и drive prefix, включая path-поля вложенного JSON; SQL дополнительно ограничивает inbox/operation paths и базовые абсолютные варианты в path-колонках v1.

## Не сохраняемые представления

`Dashboard`, `Deadline`, `Readiness`, `HomeworkIteration` и `VersionComparison` являются проекциями. Они детерминированно строятся из таблиц, а не имеют собственных строк. `RoutePreview`, `DroppedItem`, `SubmissionConfirmation` — входные/предварительные DTO; подтверждение материализуется в immutable `submission_receipts`. Визуальные состояния вкладок, раскрытий, загрузки и анимаций не сохраняются.

## Legacy v1

v1 уже неизменяема. v2 добавляет недостающие колонки, перестраивает только таблицы, где нужны nullable FK или новые enum constraints, и копирует старые строки транзакционно. Поля, которых в v1 не существовало, получают безопасные детерминированные legacy-значения (`short_name = name`, даты недели из `created_at`, пустые валидные JSON-массивы). Nullable legacy-связи остаются nullable для сохранения данных; будущие create-команды обязаны применять более строгий frontend-контракт.
