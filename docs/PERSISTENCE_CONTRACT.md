# Контракт постоянного хранения

## AI isolation consent (этап 9.2)

Схема SQLite остаётся v7. В `app_settings.ai_providers` хранятся только флаги, `consentProfileVersion` и безопасный `IsolationReport` без путей и секретов. Отсутствующая версия (включая старое `{cliEnabled:true}` этапа 9) означает отсутствие согласия и не активирует CLI. Изменение версии профиля или failed self-test аннулирует consent. Абсолютный job path, credential path и содержимое credentials не сохраняются.

Документ сверяет фактические типы `src/lib/bridge/types.ts`, `workflow-types.ts`, `ai-types.ts`, Rust DTO и SQLite schema version 8. SQLite доступен только Rust backend; основной native UI работает через реальный `TauriDesktopBridge`. Mock не участвует в production persistence.

## Соответствие сущностей

| TypeScript | Rust DTO/domain | Таблица/колонки | Nullable | JSON | Вычисляется | Источник истины |
|---|---|---|---|---|---|---|
| `AutomationRules`, `WorkflowSettings` | `AutomationRulesDto` и вложенные typed DTO | `app_settings` | отсутствующие безопасные поля дополняются defaults | весь тип в `value_json` | merge с defaults | `app_settings` |
| `Subject` | `SubjectDto` | `subjects`: `id`, `name`, `short_name`, v4 `course_url` | `courseUrl` | — | `currentWeek`, `weekCount` из `weeks` | `subjects` + агрегат `weeks` |
| `Week` | `WeekDto` | `weeks`: `id`, `subject_id`, `number`, `starts_at`, `ends_at`, `topic` | `topic`, `homeworkId` | — | `lectures`, `seminars`, `materials` из `files`; `homeworkId` из `homeworks` | `weeks`, `files`, `homeworks` |
| `FileRef` | `FileRefDto` | `files`: `id`, `name`, `kind`, `size_bytes`, `modified_at`, `relative_path` | legacy `modified_at` | — | timestamp fallback из `updated_at`/`created_at` | `files` |
| `InboxFile` | `InboxFileDto` | `inbox_files` (+ v3 `sha256`, `mime_type`, `import_operation_id`; v4 `provenance_json`) | `classification`, `duplicate`, `provenance`; v3/v4-колонки NULL у legacy | typed `classification_json`, `duplicate_json`, `provenance_json` | persistent `relative_path`, SHA-256 и MIME не выдаются | `inbox_files` |
| `ManagedRoots` | `ManagedRootsDto` (вычисляется) | `app_settings.managed_roots`: `vaultPath`, `backupPath`, `downloadsPath` | все три nullable | весь тип в `value_json` | состояние и `displayPath` вычисляются при каждом запросе | `app_settings` + файловая система |
| `InboxImportReport`, `OperationRecord` | `InboxImportReport` | `operation_results`: `operation_id`, `kind = inbox_import`, `result_json` | `importReport` null для операций без итога | весь отчёт в `result_json` | `replayed` выставляется при повторе | immutable row `operation_results` |
| `Homework` | `HomeworkDto` | `homeworks` + связи с `weeks`, `files`, `homework_versions`; v6 `revision` | `final_version_id`, `final_file_name`, `submitted_at`, `assignment_url`; legacy `deadline` | — | `subjectId`, `weekNumber`, `taskFiles`, `versions` через связи | `homeworks` и дочерние таблицы |
| `HomeworkVersion` | `HomeworkVersionDto` | `homework_versions` | `note`, `prompt_id`, `source_bundle_id` | typed `sections_json`, `files_json`, legacy `metadata_json` | legacy file fallback | immutable row `homework_versions` |
| `SourceBundle` | `workflow::SourceBundleDto`, `SourceItemDto` | `source_bundles` (+ v6 `origin`, `selection_id`, `extractor_version`, `warnings_json`, `total_chars`, `limit_chars`), `source_bundle_items` (v6: снимок материала) | `version_id`; у legacy — `selection_id`, `extractor_version`, `sha256` | `missing_json`, `warnings_json`, `warnings_json` элемента | признак «материал изменился после снимка» | обе таблицы (снимок неизменяем) |
| `ReviewRun` | `workflow::ReviewRunDto` (+ `ReviewAssessmentDto`) | `review_runs` (v6: `provenance`, `verdict`, `correctness`, `task_fit`, `course_grounding`, `teacher_status`, `missing_json`, `critical_json`, `unsupported_json`) | `version_id`, legacy `source_bundle_id`, `finished_at`, `result_json`, `error_message`, `style_score` | typed `sources_used_json`, `result_json` (`StoredReviewResult`) | `result.prompt` из `revision_prompts` | `review_runs` |
| `RevisionPrompt` | `workflow::RevisionPromptDto` | `revision_prompts` (v6: `status` draft/committed, `committed_at`, `provenance`) | `based_on_version_id`, `review_run_id`, `sent_at`, `committed_at` | typed `addresses_json`, `sources_json` | — | `revision_prompts` |
| `TeacherComment` | `workflow::TeacherCommentDto` | `teacher_comments` | `attachment_file_id`; legacy `source`/`date` | — | `attachment` через join с `files` | `teacher_comments`, `files` |
| `TeacherRequirement` | `workflow::TeacherRequirementDto` | `teacher_requirements` | только legacy `comment_id`, `checked_version_id` (TS `string \| null`); новые записи обязаны иметь оба | — | — | `teacher_requirements` |
| `SubmissionPackage` | `workflow::SubmissionPackageDto` | `submission_packages` | нет для новых записей | typed `items_json`, `links_json`, `warnings_json`, `checklist_json`, `upload_list_json` | — | `submission_packages` (этап 8 — только чтение) |
| `StyleConfirmation` | `workflow::StyleConfirmationDto` | v6 `style_confirmations` | — | — | — | immutable row `style_confirmations` |
| `SubmissionReceipt` | `workflow::SubmissionReceiptDto` | `submission_receipts` | `proof_json` | typed `proof_json`, `files_json` | — | immutable row `submission_receipts` |
| `Operation` | `OperationDto` | `operations` | `detail`, `finished_at`, `backup_path`, `homework_id`, `error_json`, `undo_payload_json` | typed `paths_json`; internal error/undo payload не выдаётся | status normalization | `operations` |
| `MaterialExtraction`, `ExtractionSummary` | `MaterialExtractionDto`, `ExtractionSummaryDto` | v5 `extractions` + `material_extractions` | `statusMessage`, все поля `metadata` | `metadata_json`, `structure_json` (блоки), `warnings_json` | `stale`, `cached`, `context` | `extractions` (cache по SHA-256) |
| `ExtractionBatchReport` | `ExtractionBatchReport` | v5 `extraction_batches.report_json` | — | весь отчёт | `replayed` при повторе | immutable row `extraction_batches` |
| `CorpusSelection`, `CorpusPreview` | `CorpusSelectionDto`, `CorpusPreviewDto` | v5 `corpus_selections`, `corpus_selection_items` | `homeworkId` (общая подборка), `updatedAt` | — | `ready`, `problem`, корпус | обе таблицы + `extractions` |
| `ExtractionSettings` | `ExtractionSettingsDto` | `app_settings.extraction = { ocrEnabled }` | — | весь тип | `ocr` и `limits` вычисляются | `app_settings` |
| `StyleProfile` | пока JSON-настройки | `app_settings` хранит только безопасные preference/metadata | контрактные nullable-поля | профиль | — | settings |
| `ProviderStatus` | `workflow::dto::ProviderStatusDto` (+ `ai::model::ProviderCliDto`, `ProviderHandoffDto`) | v7: `app_settings.ai_providers = { codex: { cliEnabled }, claude: { cliEnabled } }` | — | только флаг | `availability`, `state`, `cli`, `handoff`, `tasks`, `roles` вычисляются при каждом запросе из флага, поиска CLI и кэша проверки в памяти | `app_settings` + `ProviderRuntime` (память процесса) |
| `AiJob` | `ai::model::AiJobDto` | v7 `ai_jobs` | ответ, разбор, черновик, ошибка, времена, `edited`, `prompt_id` до соответствующего шага | `context_json`, `draft_json`, `issues_json`, `log_json` | `responseChars`; тексты не выдаются в списках | `ai_jobs` |
| `WorkflowSettings.tasks` | `AiTaskProvidersDto` | `app_settings.automation_rules.workflow.tasks` | — | часть правил | у настроек до этапа 9 выводятся из ролей | `app_settings` |

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

`OperationKind` в v3 дополнен значением `import` (колонка `operations.kind` без CHECK, миграция не требуется). Остальные enum-поля read-only DTO (`FileKind`, inbox origin/status/classification, operation kind, provider roles, week detection) преобразуются typed Rust enum и сериализуются в строковые значения TypeScript union.

## JSON и валидация

JSON используется только для вложенных неизменяемых payload, где отдельные связи не теряются: sections/files версии, missing sources, evidence/result проверки, addresses/sources промта, package items/links/checklist, proof/files receipt, classification/duplicate и operation payload. Все JSON-колонки защищены `json_valid`; репозитории сериализуют `serde_json::Value`, а не принимают произвольный SQL-текст.

`app_settings` разрешён для UI preferences, automation/workflow rules, путей конфигурации, шаблонов имён и безопасной provider metadata. Repository рекурсивно отклоняет ключи, похожие на API keys, passwords, cookies, access/refresh tokens и session secrets. Содержимое секретов не должно попадать ни в SQLite, ни в operation error/undo payload.

## Пути и native intake

Постоянные пути учебных файлов относительны корню хранилища. Inbox хранит только путь внутри `Предметы/00 Входящие/`. `DroppedItem.sourcePath` может быть абсолютным лишь как краткоживущий аргумент native intake до безопасного копирования и в SQLite не сохраняется. Repository проверяет `..`, абсолютные Unix/Windows-пути и drive prefix, включая path-поля вложенного JSON; SQL дополнительно ограничивает inbox/operation paths и базовые абсолютные варианты в path-колонках v1.

File service 4D.1 уточняет будущий typed undo payload: `UndoManifest { operationId, createdFiles, backups }`. `createdFiles` содержит только относительный `path` и `expectedSha256`; `backups` — относительные `originalPath`, `backupPath` и `expectedBackupSha256`. `BackupManifest` содержит operation ID, относительные source/backup paths, SHA-256, размер и UTC timestamp. Абсолютные source/root paths никогда не сериализуются. Формат пока не записывается production-кодом (undo импорта не реализован).

## Импорт во «Входящие» (schema v3)

- v3 добавлена отдельной транзакционной миграцией; v1 и v2 не изменялись. Legacy-строки `inbox_files` получают `sha256 = NULL`.
- `sha256` — 64 символа `[0-9a-f]`; частичный уникальный индекс запрещает две неразложенные (`status <> 'routed'`) записи с одинаковым содержимым. `mime_type` — MIME по расширению. `import_operation_id` ссылается на операцию импорта (`ON DELETE SET NULL`).
- Строка `inbox_files` вставляется только после проверенной публикации файла; `origin = 'drop'` и для picker, и для drag-and-drop, `status = 'new'`, `classification = NULL`. Если имя было занято, `duplicate = { existingPath, sameContent: false, suggestedName }`.
- `operation_results` хранит отчёт `InboxImportReport` для чтения командой и идемпотентного повтора. Триггеры запрещают UPDATE и текстовые значения JSON, начинающиеся с `/`, `\`, `~/`, `file:` или содержащие `/../`. В отчёте только безопасные имена, `InboxFile` DTO и коды/сообщения ошибок без путей.
- `app_settings.managed_roots` — единственное место с абсолютными путями: канонические пути трёх выбранных пользователем корней. Пути исходных файлов, абсолютный путь «Входящих», security-scoped bookmarks и секреты не сохраняются. Разрешения picker/drop (`IntakeGrants`) живут только в памяти процесса.
- `AutomationRules.inboxPath`, `downloadsWatchPath`, `obsidianVaultPath` остаются legacy-предпочтениями и file service не используются; источником истины для корней является `managed_roots`.

## Экспорт ЦУ и watcher (schema v4)

- v4 добавлена отдельной транзакционной миграцией; v1–v3 не изменялись.
- `inbox_files.provenance_json` (`ProvenanceDto`, `deny_unknown_fields`): `source: manual | watcher | cu_export`, `via: picker | drop | watcher`, `archiveName` (имя файла экспорта без каталога), `entryPath` (относительный путь внутри архива), `courseUrl`, `assignmentUrl`. Триггеры INSERT/UPDATE отклоняют текстовые значения, начинающиеся с `/`, `\`, `~/`, `file:`, `../`, содержащие `/../` или с drive prefix. Legacy-строки — `NULL` → `provenance: null`.
- Новые строки этапа 6: `origin = lms` (экспорт), `downloads` (обычная загрузка из watcher), `drop` (ручной импорт); `status` — `classified`/`ambiguous` с `classification_json` для экспорта, `new` для ручного импорта. `routed` этап 6 не ставит.
- `subjects.course_url` — NULL или `https://…` ≤ 2048 символов (CHECK); allowlist хоста `my.centraluniversity.ru` проверяет backend при записи и перед открытием. `homeworks.assignment_url` (v2) использует тот же allowlist для новых записей через `set_homework_assignment_url`.
- `download_seen`: `sha256` (PK), `name` (без `/` и `\`), `size_bytes`, `outcome: imported | dismissed`, `operation_id` (FK, `ON DELETE SET NULL`), `first_seen_at`, `updated_at`. Используется только для того, чтобы watcher не предлагал известное содержимое; пути папки загрузок не хранятся.
- `app_settings.downloads_watcher = { enabled }` (`deny_unknown_fields`), отсутствие ключа = выключено. Абсолютный путь папки загрузок по-прежнему хранится только в `managed_roots.downloadsPath`.
- `InboxImportReport` получил поле `skipped` (`#[serde(default)]` — старые сохранённые отчёты читаются как 0). Кандидаты watcher и разрешения выбора экспорта живут только в памяти процесса.

## Извлечение текста (schema v5)

- v5 добавлена отдельной транзакционной миграцией; v1–v4 не изменялись.
- `extractions`: `id`, `sha256` (64 hex), `extractor_version` (> 0), `options_key` (1–32 символа), `format` и `status` (CHECK по спискам), `status_message` (≤ 2000), `metadata_json`, `structure_json`, `warnings_json` (`json_valid`), `text`, `text_chars`, `truncated`, `blocks_truncated`, `ocr_used`, `source_size_bytes`, `duration_ms`, `created_at`; `UNIQUE(sha256, extractor_version, options_key)`. Перестроение (`force`) обновляет строку под тем же ключом. Результаты привязаны к содержимому, а не к пути: путей файловой системы в таблице нет; `text` — извлечённое содержимое учебного материала, хранится только в локальной SQLite.
- `material_extractions`: PK (`material_kind` ∈ `inbox`/`file`, `material_id`), `sha256` содержимого на момент анализа, `extraction_id` (FK, `ON DELETE SET NULL`), `analyzed_at`. Внешнего ключа на материал нет (два вида материалов) — триггеры `AFTER DELETE` на `inbox_files` и `files` снимают связи и выбор в подборках.
- `extraction_batches`: `operation_id` (PK, FK на `operations`), `report_json`; триггеры запрещают абсолютные/home/`file:`/`..`-пути в тексте отчёта и любые UPDATE.
- `corpus_selections`: `homework_id` (nullable, FK `ON DELETE CASCADE`); частичные уникальные индексы — одна подборка на ДЗ и одна общая. `corpus_selection_items`: PK (`selection_id`, `material_kind`, `material_id`), закреплённый `sha256`, `position`, `added_at`. Триггеры запрещают создавать подборку для отправленной работы и добавлять/удалять её элементы.
- `operations.kind = 'extract'` (колонка без CHECK, `OperationKind::Extract`), `undoable = 0`.
- `app_settings.extraction = { ocrEnabled }` (`deny_unknown_fields`); отсутствие ключа — OCR выключен. Путь к Tesseract не сохраняется.

## Не сохраняемые представления

`Dashboard`, `Deadline`, `Readiness`, `HomeworkIteration` и `VersionComparison` являются проекциями (`workflow::projections`). Они детерминированно строятся из таблиц, а не имеют собственных строк. `RoutePreview`, `DroppedItem`, `SubmissionConfirmation`, `ExportPreview`, `DownloadCandidate`, `DownloadsWatcher`, `MaterialExtractionItem`, `CorpusPreview` — входные/предварительные/вычисляемые DTO; подтверждение материализуется в immutable `submission_receipts`. Визуальные состояния вкладок, раскрытий, загрузки и анимаций не сохраняются.

## AutomationRules defaults

Если `automation_rules`/legacy `automation` отсутствует либо содержит только часть безопасных полей, чтение дополняет значения: inbox `Предметы/00 Входящие`, downloads `Загрузки`, vault `.`, имя `Не задано`/`Не задано`, шаблоны `{Фамилия}_{Имя}_Неделя{Неделя} ДЗ` и `Работа_Неделя{Неделя}_v{Версия}`, определение недели `filename_then_calendar`, semester start `1970-01-05T00:00:00Z`, пустые subject rules, порог классификации `0.7`, backup включён, auto-route выключен. Confirm defaults: low-confidence route, final и submitted требуют подтверждения; overwrite запрещён. Workflow defaults: роли `codex/claude/codex`, style threshold `4`, профиль `ПРОФИЛЬ_СТИЛЯ_ДЗ.md`, teacher review включён. Ключи, похожие на password/cookie/API/access/refresh/session secret, отклоняются.

`update_automation_rules` принимает только точный typed DTO (`deny_unknown_fields`), повторно проверяет secret-like ключи и доменные диапазоны, затем параметризованно заменяет JSON под ключом `automation_rules`. Запись и завершение operation выполняются одной транзакцией; сохранённое значение возвращается через обычную read hydration.

## Ручная классификация и tri-state patch

IPC-поле каждого из `subjectId`, `weekNumber`, `category` передаётся как tagged state: `{ state: "absent" }`, `{ state: "null" }` или `{ state: "value", value }`. `absent` сохраняет текущее значение. `null` очищает subject/week; для category это каноническое значение `unknown`. `value` записывает выбранное значение. Такое кодирование не смешивает «поле не передано» с явной очисткой.

Если subject задан, он обязан существовать. Если одновременно заданы subject и week, неделя обязана принадлежать этому предмету. Routed-запись неизменяема этой командой. После ручного изменения `classification.source = manual`, `confidence = 1`, reason не содержит локальных путей. Статус `classified` устанавливается только при subject + week + category, отличной от `unknown`; иначе сохраняется `ambiguous`.

Обе DB-only мутации создают журнал с UUID, ISO 8601 UTC timestamps, пустым безопасным списком paths и `undoable = false`. Успех фиксирует целевую запись и `completed` одной транзакцией. Ошибка откатывает её и сохраняет одну `failed`-операцию отдельной транзакцией; error payload не содержит секретов и абсолютных путей.

## Legacy v1

v1 уже неизменяема. v2 добавляет недостающие колонки, перестраивает только таблицы, где нужны nullable FK или новые enum constraints, и копирует старые строки транзакционно. Поля, которых в v1 не существовало, получают безопасные детерминированные legacy-значения (`short_name = name`, даты недели из `created_at`, пустые валидные JSON-массивы).

При read hydration nullable `topic`, `finalVersionId`, `finalFileName`, `submittedAt`, `assignmentUrl`, `promptId` и `sourceBundleId` сохраняются как `null`. Пустые legacy `HomeworkVersion.fileName` и `sourceFileId` восстанавливаются из basename относительного `file_path` и стабильного `legacy:<versionId>`; пустой `files_json` даёт один FileRef той же frozen-версии. Отсутствующий обязательный deadline, неизвестный enum, повреждённый JSON или небезопасный путь не маскируются пустой строкой: возвращается структурированная ошибка `database` без panic.

## Рабочий цикл ДЗ (schema v6, этап 8A)

- v6 — одна транзакционная миграция; v1–v5 не изменялись. Новых таблиц с назначением существующих нет: `source_bundles` дополнена, `source_bundle_items`, `review_runs`, `revision_prompts` и `operation_results` перестроены с переносом всех строк. Единственная новая таблица — `style_confirmations` (ручное подтверждение расхождения со стилем), аналога в старой схеме нет.
- `homeworks.revision` — optimistic revision агрегата (legacy-строки получают `0`).
- **SourceBundle = неизменяемый снимок подборки этапа 7.** Новый набор обязан иметь `origin = corpus_selection`, `selection_id`, `extractor_version`; версия, если указана, принадлежит тому же ДЗ (триггер). Элемент снимка хранит материал (`material_kind`/`material_id` без FK — материал можно удалить, снимок остаётся), вид источника, порядок (`position`, уникален в наборе), имя, тип, размер, относительный путь, SHA-256, id результата извлечения, версию правил, статус, число символов и коды предупреждений. `UPDATE` наборов и элементов запрещён; добавлять элементы в набор, уже использованный запуском, нельзя. Legacy-элементы v2 переносятся как `material_kind = file` с данными из `files`; SHA-256 берётся только из корректного `files.content_hash`, иначе `NULL` (не выдумывается).
- Создание снимка: план из подборки → сверка содержимого с диском (`ContentVerifier`, production — fd-relative чтение хранилища) → в транзакции план перечитывается и обязан совпасть, иначе `conflict`. Блокируют набор: удалённый материал, изменённый файл «Входящих», ссылка на результат другой версии файла, удалённый из cache результат, статус без текста, устаревшая версия правил извлечения, повреждённый JSON предупреждений, расхождение SHA-256 на диске, недоступное хранилище, пустая подборка. Предупреждения корпуса (`partial_extraction`, `text_truncated`, `corpus_truncated`) и пропуски (`missing`: нет условия — обязательно, нет лекций/семинаров — рекомендовано) сохраняются в наборе.
- **ReviewRun**: статусы `queued`, `waiting_for_provider`, `manual_handoff`, `running`, `completed`, `failed`, `cancelled`; `provenance` — `legacy` (строки v1–v5), `manual`, `provider` (этап 9). Отдельными колонками хранятся итоговое решение, корректность, соответствие условию, опора на материалы, требования преподавателя, расхождение со стилем 0–10, применённый порог, отсутствующие файлы/ссылки, критические ошибки и неподтверждённые утверждения; полный `StoredReviewResult` — в `result_json`. Новый запуск ссылается на набор и версию того же ДЗ. Завершённый запуск неизменяем (разрешено только закрепить итерацию, открепить нельзя); порог и ссылки не меняются никогда. Legacy-оценки не выдумываются: у legacy-запуска `assessment = null`.
- **RevisionPrompt**: новый промт создаётся только как `draft` с `provenance = manual`; черновик один на ДЗ (частичный уникальный индекс). Промт исправлений обязан ссылаться на завершённую проверку той же версии. После `committed` текст, ссылки и адресаты неизменяемы, удаление запрещено; передача (`sent_at`) отмечается один раз. Legacy-промты переносятся как `committed` (`committed_at = created_at`).
- **style_confirmations**: `confirmed_by = user`, время, версия, проверка, исходный балл и применённый порог; строка обязана совпадать с завершённой проверкой той же версии и существует только если балл выше порога. Неизменяема.
- **operation_results**: `kind ∈ {inbox_import, workflow}`, для `workflow` обязателен `request_hash` (SHA-256 канонического запроса) — основа идемпотентного повтора команд этапа 8B.
- Индексы реальных запросов: запуски по ДЗ/версии/набору, промты по ДЗ и запуску, элементы набора по позиции, операции по ДЗ, требования по комментарию, файлы ДЗ по категории.
- Hydration агрегата одного ДЗ — фиксированная серия из 13 запросов (`workflow::repository::load_snapshot`), независимая от числа версий, наборов и запусков. Все JSON-колонки читаются в typed DTO; повреждённый JSON, неизвестный enum, небезопасный путь — ошибка `database` с именем поля, без panic. Записи повторно проверяют, что ни одна строка вложенного JSON не является абсолютным, home, `file:` или выходящим за корень путём.

### Соответствие TypeScript → Rust → SQLite → repository

| TypeScript | Rust DTO | Таблица/колонки | Repository (`workflow::repository`) |
|---|---|---|---|
| `Homework` (+`revision`) | `HomeworkDto` | `homeworks`, `homework_versions`, `files` | `homework_dto`, `homework_core`, `bump_revision`, `set_workflow_stage`, `set_current_version` |
| `HomeworkVersion` | `HomeworkVersionDto` | `homework_versions` (append-only) | `versions`, `insert_version`, `version_candidates`, `homework_file` |
| `SourceBundle`, `SourceItem`, `BundleWarning` | `SourceBundleDto`, `SourceItemDto`, `BundleWarningDto` | `source_bundles`, `source_bundle_items` | `bundles`, `bundle`, `insert_bundle`; план — `workflow::bundle` |
| `ReviewRun`, `ReviewResult`, `ReviewAssessment` | `ReviewRunDto`, `StoredReviewResult`/`ReviewResultDto`, `ReviewAssessmentDto` | `review_runs` | `runs`, `run`, `insert_run`, `complete_run_manually`, `cancel_run`, `pin_run` |
| `RevisionPrompt` | `RevisionPromptDto` | `revision_prompts` | `prompts`, `prompt`, `insert_draft_prompt`, `update_draft_text`, `commit_prompt`, `mark_prompt_handed_off` |
| `TeacherComment` | `TeacherCommentDto` | `teacher_comments` + `files` | `teacher_comments`, `insert_teacher_comment` |
| `TeacherRequirement` | `TeacherRequirementDto` | `teacher_requirements` | `teacher_requirements`, `insert_requirement`, `resolve_requirement` |
| `StyleConfirmation` | `StyleConfirmationDto` | `style_confirmations` | `style_confirmations`, `insert_style_confirmation` |
| `SubmissionPackage` | `SubmissionPackageDto` | `submission_packages` | `submission_package` (чтение) |
| `SubmissionReceipt` | `SubmissionReceiptDto` | `submission_receipts` | `submission_receipt` (чтение) |
| `Readiness`, `HomeworkIteration`, `VersionComparison`, `Dashboard` | `ReadinessDto`, `HomeworkIterationDto`, `VersionComparisonDto`, `DashboardDto` | проекции | `workflow::projections` |

### Команды (этап 8B)

Мутации каждой сущности выполняются только командами `workflow_commands` поверх `WorkflowService`: SourceBundle — `create_source_bundle`; ReviewRun — `start_workflow_run`, `save_manual_analysis`, `save_manual_review`, `cancel_workflow_run`, `pin_workflow_run`; RevisionPrompt — `create_revision_prompt`, `update_prompt_draft`, `commit_prompt`, `hand_off_prompt` (и черновик первичного промта из `save_manual_analysis`); HomeworkVersion — `create_homework_version`, `select_homework_version`; TeacherComment/TeacherRequirement — `add_teacher_comments`, `add_teacher_requirement`, `resolve_teacher_requirement`; StyleConfirmation — `confirm_style_divergence`; Homework.workflowStage — только через state machine (`mark_ready_to_submit`, `transition_workflow` и переходы внутри команд). Чтение — `get_workflow`, `get_iterations`, `compare_versions`, `get_homework_candidates`, `get_dashboard`. SubmissionPackage и SubmissionReceipt на этапе 8 только читаются в составе агрегата.

`operations.kind` дополнен `workflow_transition` и `confirm_style` (колонка без CHECK, миграция не нужна).

## Задания передачи провайдерам (schema v7, этап 9)

- v7 — одна транзакционная миграция; v1–v6 не изменялись. Единственная новая таблица — `ai_jobs`: аналога в старой схеме нет.
- Колонки: `id`, `homework_id` (FK, `ON DELETE CASCADE`), `run_id` (FK на `review_runs`; триггер — запуск того же ДЗ), `task` (5 значений), `provider` (`codex`/`claude`), `transport` (`cli`/`manual_handoff`), `adapter_label` (≤ 120), `status` (7 значений), `include_prompt`, `prompt_text` (ровно показанный пользователю текст), `prompt_sha256` (64 hex), `prompt_chars`, `context_json` (`ContextSourceDto[]`: метка, id источника, имя, вид, SHA-256, объём, обрезка — без путей), `context_truncated`, `limit_chars`, `response_text`, `response_source` (`cli`/`paste`/`file`), `response_file_name` (только имя, без `/` и `\`), `response_sha256`, `response_truncated`, `parse_status`, `draft_json`, `issues_json`, `log_json`, `error_code`, `error_message` (≤ 2000), `exit_code`, `duration_ms`, `sent_at`, `finished_at`, `confirmed_at`, `edited`, `prompt_id` (FK на `revision_prompts`, `ON DELETE SET NULL`), `created_at`, `updated_at`.
- CHECK: `confirmed_at` задан ровно у `confirmed`; у `response_ready` есть ответ; `response_source = cli` только у `transport = cli`.
- Индексы: по ДЗ и времени, по запуску и статусу; частичный уникальный индекс — не больше одного `running` на запуск.
- Триггеры: переданный текст, состав и привязки неизменяемы; закрытое задание (`confirmed`, `discarded`, `cancelled`, `failed`) неизменяемо; абсолютные пути (`/…`, `\…`, `~/`, `file:`, `/../`) запрещены в составе, пути `/Users/…`, `/home/…`, `file:` — в журнале, замечаниях и сообщении об ошибке; новые задания у отправленной работы запрещены.
- Ответ провайдера и текст передачи — учебное содержание; хранятся только в локальной SQLite (как извлечённый текст этапа 7), не в Git и не в хранилище. Учётных данных провайдеров нет нигде.
- `review_runs.provenance = provider` и `revision_prompts.provenance = provider` (значения существовали с v6) ставятся только командой `confirm_ai_result`; промт получает `provider`, только если его текст вернул провайдер.

## Schema v8 — этап 10

- `route_operation_items` связывает undoable-операцию с точными `inbox_file_id`, относительным target и решением `copy|skip|rename`; восстановление не ищет файл по имени.
- `managed_notes` резервирует безопасные относительные пути и hash управляемого блока для диагностики.
- `style_profile_sources` принимает только явно выбранные (`explicitly_selected = 1`) локальные источники трёх видов; хранит ссылку и вычисленные признаки, не содержимое документа.
- `submission_packages.manifest_json` и `submission_receipts.manifest_json` содержат только относительный путь, SHA-256 и размер. Триггер запрещает изменение manifest/snapshot receipt.
- Все новые пути относительны; абсолютные исходные пути, passwords, cookies и tokens в таблицы не записываются.
