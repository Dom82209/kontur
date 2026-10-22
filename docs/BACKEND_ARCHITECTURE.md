# Архитектура Rust backend

Статус документа: этап 8B. Read-only путь, две DB-only мутации, реальный импорт во «Входящие» через fd-relative file service, watcher загрузок, импорт экспорта CU LMS Enhancer и локальное извлечение текста с подборкой материалов подключены к Tauri IPC. Основной UI по умолчанию использует `MockDesktopBridge`.

## Модули

- `app_state` — безопасные долгоживущие зависимости приложения: каталог данных, домашний каталог (для политики корней), соединение SQLite, in-memory `IntakeGrants`, mutex последовательных импортов, in-memory `CandidateRegistry` и `WatcherService` (этап 6), флаги отмены `ExtractionJobs`, лимиты извлечения и лениво найденный OCR-движок (этап 7).
- `commands` — тонкие Tauri-команды; они получают managed `AppState`, делегируют чтение query-слою и DB-only записи mutation-слою, а после commit доставляют события.
- `db` — соединение SQLite, параметры соединения и транзакционная граница.
- `db/migrations` — встроенные, версионированные SQL-миграции.
- `domain` — доменные enum и входные структуры persistence.
- `dto` — точные сериализуемые read-модели frontend-контракта с `camelCase`.
- `error` — единый сериализуемый ответ `{ code, message }`.
- `events` — константы 17 bridge-событий (12 исходных, 3 этапов 5–6, 2 этапа 7), camelCase payload DTO и общий helper эмита через `tauri::Emitter`; DB-only команды используют его после commit.
- `file_service` — типизированная граница безопасных roots/paths, hashing, read-only planning, atomic copy (в том числе компенсируемый `atomic_copy_committed`), read-only `inspect_stored_file`, backup и undo validation; все операции внутри корней fd-relative (`fd.rs`), test-only race hooks в `hooks.rs`; не зависит от GUI.
- `intake` — этап 5 без зависимости от GUI: управляемые каталоги (`RootsService`), безопасные имена и MIME, `IntakeGrants`, `InboxImporter` с дедупликацией, компенсацией и идемпотентностью, абстракция `FilePicker`, SQL-граница `store`.
- `lms` — этап 6 без зависимости от GUI: `zip_safe` (строгий парсер ZIP и ограниченная распаковка), `classify` (классификация по путям), `export` (предпросмотр и импорт экспорта через `InboxImporter`), `watcher` (чистый `DownloadScanner`, `WatcherCore`, фоновый `WatcherService`), `candidates`, `links` (allowlist LMS), `store` (SQL этапа 6).
- `extraction` — этап 7 без зависимости от GUI: `model` (формат, статус, предупреждения, блоки, лимиты, бюджет времени, `Collector`), `engine` (определение формата по содержимому, изолированный поток с `catch_unwind` и сторожевым таймером), `pdf`, `ooxml` (XML без DTD, пакет OOXML поверх `zip_safe` в памяти, relationships только внутрь пакета), `docx`, `xlsx`, `pptx`, `text` (TXT/MD/CSV/TSV/IPYNB), `image`, `ocr` (необязательный локальный Tesseract), `store` (SQL v5), `service` (cache по SHA-256, пакет, подборка, корпус), `jobs` (отмена).
- `extraction_commands` — тонкие Tauri-команды этапа 7 и доставка событий `extraction:progress`/`extraction:updated`.
- `lms_commands` — тонкие Tauri-команды этапа 6, диалог выбора экспорта, хост watcher-потока (`AppWatcherHost`), открытие ссылок во внешнем браузере через `tauri-plugin-opener`.
- `intake_commands` — тонкие Tauri-команды этапа 5, `DialogPicker` поверх официального `tauri-plugin-dialog`, действия Finder через `tauri-plugin-opener`, обработчик native drag-and-drop окна.
- `ipc_commands` — единый список IPC-команд, который `build.rs` передаёт в `AppManifest`; тест сверяет его с capability.
- `mutation_service` — транзакционные сценарии сохранения AutomationRules и ручной классификации, журнал операций и намерения событий без зависимости от GUI.
- `query_service` — прикладной read-only слой сборки DTO и общей обработки ошибок без зависимости от GUI.
- `workflow_commands` — тонкие Tauri-команды этапа 8, `EventSink` и доставка событий после commit.
- `workflow` — этап 8: `dto` (точные DTO рабочего цикла и исчерпывающие enum), `repository` (SQL поверх `&Connection` — одинаково в чтении и в транзакции), `bundle` (неизменяемый SourceBundle из подборки этапа 7, `ContentVerifier`), `projections` (Readiness, IterationChain, сравнение версий, Главная), `state_machine` (единственная таблица переходов девяти этапов), `service` (транзакционные команды: идемпотентность, optimistic revision, журнал операций, события после commit).
- `repositories` — каталог специализированных модулей: `subjects`, `weeks`, `homeworks`, `files`, `inbox`, `operations`, `settings`, DB-only `writes`; прежние транзакционные методы этапа 4B.1 сохранены в `mutations`.

Списковые агрегаты собираются фиксированной серией запросов: Subject — одним агрегатным запросом, Week — базовые строки + файлы + связи Homework, Homework — базовые строки + task files + versions. N+1-запросов по элементам списков нет.

## База данных

Production-файл открывается как `<app_data_dir>/kontur.sqlite3`. Для него включены foreign keys, busy timeout 5 секунд, WAL и `synchronous=NORMAL`. Тесты используют только отдельные in-memory базы.

Текущая версия схемы — `6`. Неизменяемые v1–v5 и добавочная v6 встроены через `include_str!`, выполняются последовательно в транзакциях и повторно безопасны. Версия хранится одновременно в `PRAGMA user_version` и журнале `schema_migrations`. База с версией выше поддерживаемой отклоняется ошибкой `conflict`, чтобы старый бинарник не изменял новую схему. Тест миграции заполненной v1 проверяет сохранение строк всех старых таблиц.

Схема содержит таблицы:

- `app_settings`;
- `subjects`, `weeks`, `homeworks`, `files`, `inbox_files`;
- `homework_versions`;
- `source_bundles`, `source_bundle_items`;
- `review_runs`, `revision_prompts`;
- `teacher_comments`, `teacher_requirements`;
- `submission_packages`, `submission_receipts`;
- `operations`, `operation_results` (v3);
- `download_seen` (v4);
- `extractions`, `material_extractions`, `extraction_batches`, `corpus_selections`, `corpus_selection_items` (v5);
- `style_confirmations` (v6).

Foreign keys задают владение и политику удаления, уникальные ограничения защищают номера недель, одну карточку ДЗ на неделю, номера/метки версий и единственный receipt. Индексы добавлены для основных связей и выборок по статусу. Статусы и этапы ограничены `CHECK`. Постоянно хранятся только относительные пути; `sourcePath` native intake является транзитным значением и не сохраняется как абсолютный пользовательский путь.

`source_bundles` и `review_runs` сохраняют Homework, а ссылка на версию nullable для первичного анализа до v01. `revision_prompts` допускают отсутствие исходной версии и review run. Полное соответствие frontend DTO описано в `PERSISTENCE_CONTRACT.md`.

Версии ДЗ вставляются как новые записи с `frozen = 1`; SQLite triggers запрещают их UPDATE/DELETE. Receipt также защищён от UPDATE/DELETE. Submitted Homework нельзя обновить или удалить, а новые версии, файлы и workflow-данные к ней добавить нельзя. Переход в submitted намеренно не предоставлен обычным repository API: будущая специализированная транзакция должна сначала создать package/receipt и только затем закрыть Homework.

## Транзакции и ошибки

Все записи репозиториев проходят через `Database::with_transaction`. Составная операция создания ДЗ с начальной версией демонстрирует атомарный rollback. Ограничения SQLite преобразуются в `conflict`, отсутствие записи — в `not_found`, изменение отправленной работы — в `immutable`.

`app_settings` принимает безопасные automation/workflow/UI preferences. Repository рекурсивно отклоняет очевидные secret-ключи: passwords, cookies, API keys, access/refresh tokens и session secrets.

Зарезервированы коды: `not_found`, `conflict`, `duplicate`, `confirmation_required`, `io_error`, `not_undoable`, `immutable`, `blocked`, `cancelled`, `database`, `unknown`.

## Read-only IPC этапа 4C.1

Зарегистрированы команды `backend_health`, `get_subjects`, `get_weeks`, `get_week`, `get_homework_list`, `get_homework`, `get_inbox_files`, `get_automation_rules`, `get_activity_log`. Запросы по идентификатору возвращают `{ code: "not_found", message }`, если запись отсутствует. Повреждённые JSON, неизвестные enum и небезопасные persistent paths дают структурированную ошибку `database`, без panic.

Низкоуровневый TypeScript-клиент находится в `src/lib/bridge/tauri/ipc.ts`. Он лениво загружает официальный `@tauri-apps/api/core`, проверяет Tauri runtime, преобразует Rust-ошибки в `BridgeError` и вне native runtime возвращает `backend_unavailable`.

`TauriDesktopBridge` использует эти функции для восьми методов чтения. `getWeek` и `getHomework` преобразуют только `not_found` в `null`; остальные ошибки сохраняются. Все отсутствующие возможности возвращают `backend_unavailable`, без mock fallback. Отдельная `createTauriBridge()` не используется основным UI.

`tauri/runtime.ts` является общей runtime boundary для invoke/listen. `tauri/events.ts` согласует асинхронный Tauri `listen()` с синхронным `DesktopBridge.on()`: ранняя отписка запоминается и вызывает настоящий unlisten сразу после регистрации; callbacks после отмены блокируются, ошибки регистрации контролируемо нормализуются.

## DB-only IPC этапа 4C.2B

Зарегистрированы `update_automation_rules` и `update_classification`. Они используют `MutationService`, меняют только SQLite и возвращают `OperationResult<T>` с перечитанным frontend DTO и завершённой операцией. TypeScript IPC и `TauriDesktopBridge` вызывают команды напрямую; fallback на mock отсутствует.

Операция создаётся с UUID и статусом `running` внутри той же транзакции, что и целевая запись. Успех атомарно фиксирует данные и статус `completed`; ошибка откатывает целевую транзакцию и записывает ровно одну отдельную `failed`-операцию. DB-only операции необратимы через текущий undo API (`undoable = false`). Время хранится как ISO 8601 UTC.

Классификационный patch имеет явные состояния `absent`, `null`, `value`: отсутствие сохраняет колонку, null очищает subject/week либо переводит category в `unknown`, value устанавливает значение. Сервис проверяет ссылки subject/week, принадлежность недели предмету и запрещает изменение routed-файла. Ручное изменение ставит source `manual`, confidence `1`; полный набор полей даёт `classified`, неполный — `ambiguous`.

События эмитятся только после успешного commit: классификация — `inbox:classification-updated`, затем `operation:completed`; правила — `operation:completed`. После сохранённой failed-операции отправляется `operation:failed`. Если доставка события не удалась, БД не откатывается; команда возвращает контролируемую ошибку и предлагает перечитать состояние.

## File service foundation этапов 4D.1 и 4D.1.1

Публичная Rust-граница использует `VaultRoot`, `InboxRoot`, `BackupRoot`, `SourceFile`, `PersistentPath` и `VerifiedDestination`. Root проходит проверку политики (абсолютный, не `/`, не home, не широкий системный каталог), после чего один раз открывается как directory fd с `O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`. Дальше корни, inbox и destination представлены только открытыми fd; абсолютный `PathBuf` наружу не выдаётся.

Обход относительного пути выполняется покомпонентно через `openat(O_NOFOLLOW)`, создание — через `mkdirat`, проверка имён — `statat(AT_SYMLINK_NOFOLLOW)`, публикация — `renameat_with(RENAME_NOREPLACE)` в одном dir fd, cleanup — `unlinkat` только при совпадении identity созданного inode. Source открывается один раз; SHA-256, размер и копирование читают один inode через `pread`. Перед публикацией, после неё и в конце planning/undo повторный обход сравнивает inode родителя с открытым fd, подмена каталога даёт `blocked`. Undo пока только валидируется.

С этапа 5 модуль используется импортом во «Входящие» (`intake`); прямого IPC к file service нет, frontend получает только команды этапа 5. Все тестовые операции выполняются в `tempfile::TempDir`; race-тесты используют синхронные test hooks и `Barrier`, без sleep. Полная политика находится в `FILE_SERVICE_SECURITY.md`.

## Импорт во «Входящие» — этап 5

Команды (все возвращают структурированный `AppError { code, message }`, без panic):

| Команда | Назначение |
|---|---|
| `check_managed_roots` | Проверить корни: повторно открыть fd, вернуть `ManagedRoots` |
| `choose_managed_root { kind }` | Системный выбор папки (`vault`/`backup`/`downloads`); отмена → `cancelled: true` |
| `prepare_inbox_root` | Создать `Предметы/00 Входящие` внутри хранилища fd-relative |
| `pick_import_files` | Системный выбор нескольких файлов; пути регистрируются в `IntakeGrants` |
| `import_inbox_files { operationId, items }` | Импорт; `OperationResult<InboxImportReport>` |
| `get_operation_result { operationId }` | Операция и сохранённый отчёт импорта |
| `open_inbox_file { fileId }` | Открыть файл приложением по умолчанию (не для исполняемых файлов) |
| `reveal_inbox_file { fileId }`, `reveal_in_finder { relativePath }` | Показать в Finder |

Долгие команды — `async` и выполняют работу в `spawn_blocking`, поэтому главный поток и доставка `operation:progress` не блокируются. Диалоги и Finder вызываются Rust-кодом через официальные плагины; frontend не получает их permissions.

`InboxImporter::run(operationId, items, progress)`:

1. UUID v4 operation id; повтор с сохранённым итогом → replay без файловых действий; незавершённая операция с тем же id → `conflict`.
2. Операция `running` (kind `import`) в отдельной транзакции, `progress(0)`.
3. Для каждого файла: разрешение picker/drop → `SourceFile::open_external` → размер ≤ 4 ГБ → безопасное имя → SHA-256 → дубль по SHA-256 → свободное имя (диск и БД) → `atomic_copy_committed` → `insert_inbox_file`; при ошибке БД — компенсация. Ошибка файла фиксируется в отчёте, остальные продолжаются; после каждого файла — progress.
4. Одна транзакция завершает операцию (`completed`, или `failed`, если не удались все) и пишет `operation_results`.

Команда после возврата importer эмитит `inbox:file-added` (только новые файлы), `inbox:updated`, затем `operation:completed`/`operation:failed`. Replay событий не эмитит. Ошибка доставки события не откатывает данные и возвращает `unknown` с просьбой перечитать «Входящие».

Native drag-and-drop: `Builder::on_window_event` → `WindowEvent::DragDrop` → `register_native_paths` (только абсолютные regular files) → `intake:native-drop { items }`; Enter/Leave → `intake:drag-state`.

Capabilities: `build.rs` → `tauri_build::AppManifest::commands(COMMANDS)` генерирует `allow-<command>` (каталог `permissions/autogenerated/` создаётся при сборке и не хранится в Git). `capabilities/default.json` содержит только `core:event:default`, `core:window:allow-start-dragging` и `allow-*` для 41 команды (20 этапов 4–5, 10 этапа 6 и 11 этапа 7).

## Экспорт ЦУ и watcher загрузок — этап 6

| Команда | Назначение |
|---|---|
| `get_downloads_watcher` | Состояние watcher и кандидаты (`DownloadsWatcherDto`) |
| `set_downloads_watcher { enabled }` | Явно включить/выключить; включение требует папку загрузок |
| `dismiss_download_candidate { candidateId }` | Скрыть кандидата (`download_seen.outcome = dismissed`) |
| `pick_cu_export` | Системный выбор ZIP/PDF; путь → `IntakeGrants`, кандидат `manual` |
| `preview_cu_export { candidateId }` | Индекс без распаковки, классификация, предупреждения, SHA-256 |
| `import_cu_export { operationId, request }` | Проверка, распаковка в `TempDir`, импорт через `InboxImporter` |
| `set_subject_course_url`, `set_homework_assignment_url` | Ссылки LMS после allowlist |
| `open_course`, `open_assignment` | Открыть сохранённую ссылку во внешнем браузере после повторной проверки |

- Watcher: фоновый поток `kontur-downloads-watcher`, шаг раз в 1,5 с; каждый шаг заново открывает `DownloadsRoot` по сохранённому пути (политика корня), читает верхний уровень через fd, стабилизирует наблюдения (`DownloadScanner`), хэширует готовый файл из открытого inode, отбрасывает известное содержимое и регистрирует кандидата. Изменения доставляются `downloads:watcher-updated`. Остановка — флаг + `join`. Поток стартует при запуске приложения, только если пользователь включил watcher ранее.
- Импорт экспорта: сериализуется тем же mutex, что и импорт этапа 5; повтор с тем же `operationId` возвращает сохранённый итог без событий. Ошибки запроса и отпечатка возвращаются до создания операции. Порядок событий: `cu-import:progress` + `operation:progress` → `inbox:file-added` → `inbox:updated` → `operation:completed`/`operation:failed` → `downloads:watcher-updated`.
- Подробная модель угроз ZIP и watcher — `FILE_SERVICE_SECURITY.md`.

## Извлечение текста — этап 7

| Команда | Назначение |
|---|---|
| `list_material_extractions` | Материалы «Входящих» и недель с кратким итогом извлечения |
| `get_material_extraction { material }` | Сохранённый результат (`not_found`, если анализа не было) |
| `analyze_material { material, force }` | Анализ одного материала; cache по SHA-256, `force` — перестроение |
| `analyze_materials { operationId, materials, force }` | Пакет с прогрессом и отменой; `OperationResult<ExtractionBatchReport>` |
| `cancel_extraction { operationId }` | Остановить пакет (`false` — уже завершён) |
| `clear_extraction_cache` | Удалить результаты и связи (оригиналы и подборки не трогаются) |
| `get_extraction_settings`, `set_extraction_settings { ocrEnabled }` | OCR (по умолчанию выключен), состояние движка, лимиты |
| `get_corpus_selection`, `set_corpus_selection`, `preview_corpus` | Подборка материалов (`homeworkId` или общая) и корпус для будущего анализа |

`ExtractionService::analyze`:

1. Материал по ссылке `{ kind, id }` → имя, относительный путь, SHA-256 импорта (для «Входящих»).
2. Ключ cache: (SHA-256, `EXTRACTOR_VERSION`, `options_key`). Если SHA-256 известен и результат переиспользуемый (не `timeout`/`failed`) — связь с материалом и ответ без чтения файла.
3. Иначе `RootsService::open_vault` → `file_service::read_vault_file` (fd-relative, только чтение, один inode, ≤ 200 МБ в память; больше — потоковый SHA-256 и статус `too_large`). Несовпадение с SHA-256 импорта → `conflict`.
4. `engine::run_isolated`: поток `kontur-extract` (стек 32 МБ), `catch_unwind`, кооперативный бюджет и сторожевой таймер (срок + 2 с), опрос отмены каждые 50 мс.
5. `store::save_extraction` одной транзакцией: upsert результата + связь материала.

Пакет: UUID `operationId` (повтор — сохранённый итог из `extraction_batches` без событий; незавершённый — `conflict`), дедупликация, ≤ 500 материалов, операция kind `extract`, ошибка одного материала попадает в отчёт, отмена помечает оставшиеся `cancelled`. Порядок событий: `extraction:progress` + `operation:progress` → `extraction:updated` → `operation:completed`/`operation:failed`. Ошибка доставки события не откатывает сохранённое и возвращает `unknown` с просьбой обновить список.

Подборка: только материалы с текстом (`complete`/`partial`), неизменённым содержимым и закреплённым SHA-256; до 200 материалов; подборка отправленной работы неизменяема; корпус — до 600 000 символов в порядке выбора.

## Рабочий цикл ДЗ — этап 8

### State machine (`workflow/state_machine.rs`)

Этап меняет только backend; записать `workflowStage` напрямую нельзя. Разрешены ровно 18 пар (всё остальное — `conflict`, из «Отправлено» — `immutable`); невыполненное условие — `blocked`:

| Откуда → куда | Условие |
|---|---|
| materials_collected → codex_analysis | есть полный набор источников анализа из подборки |
| codex_analysis → materials_collected, prompt_ready → codex_analysis, claude_solving → prompt_ready, codex_review → claude_solving, teacher_review → codex_review | возврат, без условий |
| codex_analysis → prompt_ready | анализ сохранён и первичный промт зафиксирован |
| prompt_ready → claude_solving | зафиксированный промт передан автору |
| claude_solving → codex_review | выбрана версия решения |
| codex_review → needs_fixes | проверка текущей версии требует исправлений |
| codex_review → teacher_review | проверка текущей версии пройдена, критических ошибок нет |
| codex_review → ready_to_submit | проверка преподавателем не обязательна и готовность пройдена |
| needs_fixes → claude_solving | промт исправлений по текущей версии зафиксирован и передан |
| teacher_review → needs_fixes, ready_to_submit → needs_fixes, ready_to_submit → teacher_review | решение пользователя |
| teacher_review → ready_to_submit | готовность пройдена; без комментариев преподавателя — явное подтверждение (`confirmation_required`) |
| ready_to_submit → submitted | только подтверждением отправки этапа 10 — сейчас всегда `blocked` |

Названия этапов исторические: провайдер каждой роли берётся из `AutomationRules.workflow.roles`.

### Application service (`workflow/service.rs`)

Каждая мутация: UUID v4 `operationId` и SHA-256 отпечаток запроса → повтор с тем же id и запросом возвращает сохранённый итог (`operation_results.kind = workflow`) без действий и событий, тот же id с другим запросом — `conflict`, id завершившейся ошибкой операции — `conflict` → транзакция: операция `running` с `homework_id` → проверка «не отправлено» и `expectedRevision` (устаревшая — `conflict` с просьбой перечитать) → проверки состояния и state machine → запись → `revision + 1` → итог → `completed`. Любая ошибка откатывает всю транзакцию и записывает одну отдельную `failed`-операцию (без секретов и путей). Одновременные команды на одной ревизии дают одного победителя. События строятся только после commit из перечитанного состояния.

Реальных AI-адаптеров нет. Запуск получает `waiting_for_provider` (режим `provider`) или `manual_handoff` (режим `manual`); результат сохраняет пользователь с provenance `manual`. Одновременно у ДЗ может быть только один незавершённый запуск. Набор источников запуска должен быть того же вида, полным и неизменившимся; проверяется только текущая выбранная версия. Применённый порог стиля берётся из текущих настроек и сохраняется в запуске.

### IPC-команды этапа 8

| Команда | DesktopBridge | Результат |
|---|---|---|
| `get_workflow { homeworkId }` | `getWorkflow` (+ `getReviewRuns`, `getPrompts`, `getTeacherComments`, `getTeacherRequirements`, `getReadiness`, `getSubmissionPackage`, `getSubmissionReceipt`) | агрегат `HomeworkWorkflowDto` |
| `get_dashboard` | `getDashboard` | `DashboardDto` |
| `get_iterations { homeworkId }` | `getIterations` | `HomeworkIterationDto[]` |
| `compare_versions { fromVersionId, toVersionId }` | `compareVersions` | `VersionComparisonDto` |
| `get_homework_candidates { homeworkId }` | `getHomeworkCandidates` | `FileRefDto[]` |
| `create_source_bundle { operationId, homeworkId, purpose, versionId, expectedRevision }` | `prepareSourceBundle` | `OperationResult<SourceBundle>` |
| `start_workflow_run { operationId, request: { homeworkId, kind, versionId, bundleId, mode }, expectedRevision }` | `runMaterialAnalysis`, `runVersionReview`, `runRevisionPrompt`, `runTeacherReview` | `OperationResult<ReviewRun>` |
| `save_manual_analysis { operationId, runId, input, expectedRevision }` | `saveManualAnalysis` | `OperationResult<ReviewRun>` + черновик первичного промта |
| `save_manual_review { operationId, runId, input, expectedRevision }` | `saveManualReviewResult` | `OperationResult<ReviewRun>` |
| `cancel_workflow_run`, `pin_workflow_run { operationId, runId, expectedRevision }` | `cancelRun`, `saveRunAsIteration` | `OperationResult<ReviewRun>` |
| `create_revision_prompt { operationId, reviewRunId, text, expectedRevision }` | `createRevisionPrompt` | `OperationResult<RevisionPrompt>` (черновик) |
| `update_prompt_draft { operationId, promptId, text, expectedRevision }` | `updatePromptText` | только черновик |
| `commit_prompt { operationId, promptId, expectedRevision }` | `commitPrompt` | фиксация |
| `hand_off_prompt { operationId, promptId, expectedRevision }` | `sendPromptToAuthor` | ручная передача автору |
| `create_homework_version { operationId, homeworkId, input, expectedRevision }` | `createHomeworkVersion` | append-only версия, номер назначает backend |
| `select_homework_version { operationId, versionId, expectedRevision }` | `markVersionFinal` | текущая проверяемая версия |
| `add_teacher_comments`, `add_teacher_requirement`, `resolve_teacher_requirement` | `saveTeacherComments`, `addTeacherRequirement`, `resolveRequirement` | ручное добавление и проверка |
| `confirm_style_divergence { operationId, reviewRunId, expectedRevision }` | `confirmStyleDivergence` | `StyleConfirmation` |
| `mark_ready_to_submit { operationId, homeworkId, noTeacherComments, expectedRevision }` | `markReadyToSubmit` | переход через state machine |
| `transition_workflow { operationId, homeworkId, to, reason, expectedRevision }` | `transitionWorkflow` | допустимый переход (кроме «Готово к сдаче» и «Отправлено») |

Capability содержит ровно 64 команды; тесты сверяют `ipc_commands::COMMANDS` с capability и с `generate_handler!`.

### События

После commit, в порядке: доменные (`homework:version-created`, `review:completed`, `homework:revision-prompt-created`), `workflow:stage-changed` для каждого перехода, `homework:updated { homeworkId, revision, stage, reason, operationId }`, `homework:readiness-updated { homeworkId, ready, blockers, revision }`, `operation:progress`, `operation:completed`. Ошибка — только `operation:failed`. Повтор — без событий. Имена `workflow:stage-changed`, `review:completed` и `homework:version-created` существовали в контракте и сохранены (вместо `homework:workflow-stage-changed` и `homework:review-completed`). Payload — camelCase, только ID и короткие поля, без абсолютных путей и текста извлечения. Ошибка доставки события не откатывает сохранённое: команда возвращает `unknown` с просьбой перечитать состояние.

## Health check

Tauri-команда `backend_health` проверяет доступность соединения и возвращает только:

- `ready`;
- версию приложения;
- версию схемы;
- платформу и архитектуру;
- факт существования production DB.

## Граница следующего этапа

React не обращается к SQLite и файловой системе напрямую: публичной границей остаётся типизированный `DesktopBridge`. `createBridge()` по умолчанию создаёт `MockDesktopBridge`; `TauriDesktopBridge` целиком включается только флагом разработчика `VITE_KONTUR_BRIDGE=tauri`. Hybrid bridge запрещён.

Дальше — классификация обычных файлов и раскладка по предметам, применение backup/undo, provider adapters и построение `SourceBundle` из подборки этапа 7, AI-интеграции и immutable submission snapshot. Отправка работ в LMS остаётся ручной.
