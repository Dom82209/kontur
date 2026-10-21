# Архитектура Rust backend

Статус документа: фундамент этапа 4B.1. Persistence синхронизирован с frontend-контрактом; `TauriDesktopBridge` и бизнес-команды ещё не подключены.

## Модули

- `app_state` — безопасные долгоживущие зависимости приложения: каталог данных, соединение SQLite и реестр будущих событий.
- `commands` — минимальная команда `backend_health`. Она не раскрывает путь базы или другие локальные данные.
- `db` — соединение SQLite, параметры соединения и транзакционная граница.
- `db/migrations` — встроенные, версионированные SQL-миграции.
- `domain` — сериализуемые доменные DTO и входные структуры репозиториев.
- `error` — единый сериализуемый ответ `{ code, message }`.
- `events` — пустая типизированная граница для последующего подключения Tauri events; mock-события frontend сюда не перенесены.
- `repositories` — транзакционные операции над настройками, предметами, неделями, ДЗ, версиями и журналом операций.

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

## Health check

Tauri-команда `backend_health` проверяет доступность соединения и возвращает только:

- `ready`;
- версию приложения;
- версию схемы;
- платформу и архитектуру;
- факт существования production DB.

## Граница следующего этапа

React не обращается к SQLite напрямую: публичной границей должен остаться типизированный `DesktopBridge`, а будущий `TauriDesktopBridge` будет преобразовывать его вызовы в узкие Tauri commands и подписки на события. Это сохраняет системные полномочия в Rust и позволяет browser/mock работать без нативного доступа.

Для подключения frontend остаются `TauriDesktopBridge`, полный набор Tauri-команд и событий, native picker/drop, файловые сервисы с проверкой путей, backup/undo и watcher. Будущие provider adapters будут запускать официальные локальные клиенты Codex/Claude без получения или сохранения их cookies, токенов и других секретов. Также остаётся сборка неизменяемого submission snapshot на диске. Эти части намеренно не реализованы на этапе 4B.
