# Архитектура Rust backend

Статус документа: этап 4D.1. Read-only путь и две DB-only мутации реализованы; изолированный file-service foundation пока не имеет production entry point. UI по-прежнему использует `MockDesktopBridge`.

## Модули

- `app_state` — безопасные долгоживущие зависимости приложения: каталог данных, соединение SQLite и реестр будущих событий.
- `commands` — тонкие Tauri-команды; они получают managed `AppState`, делегируют чтение query-слою и DB-only записи mutation-слою, а после commit доставляют события.
- `db` — соединение SQLite, параметры соединения и транзакционная граница.
- `db/migrations` — встроенные, версионированные SQL-миграции.
- `domain` — доменные enum и входные структуры persistence.
- `dto` — точные сериализуемые read-модели frontend-контракта с `camelCase`.
- `error` — единый сериализуемый ответ `{ code, message }`.
- `events` — константы 12 bridge-событий, camelCase payload DTO и общий helper эмита через `tauri::Emitter`; DB-only команды используют его после commit.
- `file_service` — типизированная граница безопасных roots/paths, hashing, read-only planning, atomic copy, backup и undo validation; не зависит от GUI и не зарегистрирована в IPC.
- `mutation_service` — транзакционные сценарии сохранения AutomationRules и ручной классификации, журнал операций и намерения событий без зависимости от GUI.
- `query_service` — прикладной read-only слой сборки DTO и общей обработки ошибок без зависимости от GUI.
- `repositories` — каталог специализированных модулей: `subjects`, `weeks`, `homeworks`, `files`, `inbox`, `operations`, `settings`, DB-only `writes`; прежние транзакционные методы этапа 4B.1 сохранены в `mutations`.

Списковые агрегаты собираются фиксированной серией запросов: Subject — одним агрегатным запросом, Week — базовые строки + файлы + связи Homework, Homework — базовые строки + task files + versions. N+1-запросов по элементам списков нет.

## База данных

Production-файл открывается как `<app_data_dir>/kontur.sqlite3`. Для него включены foreign keys, busy timeout 5 секунд, WAL и `synchronous=NORMAL`. Тесты используют только отдельные in-memory базы.

Текущая версия схемы — `2`. Неизменяемая v1 и добавочная v2 встроены через `include_str!`, выполняются последовательно в транзакциях и повторно безопасны. Версия хранится одновременно в `PRAGMA user_version` и журнале `schema_migrations`. База с версией выше поддерживаемой отклоняется ошибкой `conflict`, чтобы старый бинарник не изменял новую схему. Тест миграции заполненной v1 проверяет сохранение строк всех старых таблиц.

Схема содержит таблицы:

- `app_settings`;
- `subjects`, `weeks`, `homeworks`, `files`, `inbox_files`;
- `homework_versions`;
- `source_bundles`, `source_bundle_items`;
- `review_runs`, `revision_prompts`;
- `teacher_comments`, `teacher_requirements`;
- `submission_packages`, `submission_receipts`;
- `operations`.

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

## File service foundation этапа 4D.1

Публичная Rust-граница использует `VaultRoot`, `InboxRoot`, `BackupRoot`, `SourceFile`, `PersistentPath` и `VerifiedDestination`. Root canonicalized, существует и не является `/`, домашним или широким системным каталогом. Persistent path валидируется по компонентам и сериализуется только с `/`; absolute source остаётся runtime-only.

Planner сравнивает streaming SHA-256 и возвращает `Copy`, `SameContent` либо `NameConflict`. Atomic copy создаёт same-directory temp через `create_new`, потоково копирует, синхронизирует, проверяет size/hash и публикует через `rustix` `RENAME_NOREPLACE`. Backup использует тот же primitive. Undo пока только валидируется по operation state, submitted guard, roots и hashes.

Модуль не вызывается из `run()`, commands, AppState или bridge. Capabilities остаются минимальными и не открывают frontend доступ к filesystem. Все тестовые операции выполняются в `tempfile::TempDir`. Полная политика находится в `FILE_SERVICE_SECURITY.md`.

## Health check

Tauri-команда `backend_health` проверяет доступность соединения и возвращает только:

- `ready`;
- версию приложения;
- версию схемы;
- платформу и архитектуру;
- факт существования production DB.

## Граница следующего этапа

React не обращается к SQLite напрямую: публичной границей остаётся типизированный `DesktopBridge`. `createBridge()` намеренно продолжает создавать `MockDesktopBridge`; наличие двух DB-only write-команд ещё не означает готовность полного production bridge. Hybrid bridge запрещён, чтобы реальные чтения не смешивались с несохраняемыми mock-мутациями.

Следующий этап 4D.2 — native picker/drop и реальный импорт во «Входящие» через принятый file service. Watcher, routing по vault, полное применение backup/undo, provider adapters, AI/LMS-интеграции и immutable submission snapshot остаются отдельными этапами.
