# Статус реализации «Контур»

Дата аудита: 21 октября 2026 года.

## Текущий этап

Этап 4D.1 завершён: создан изолированный безопасный Rust file service с типизированными roots/paths, потоковым SHA-256, планированием дедупликации, no-clobber atomic copy, backup manifest и валидацией undo. Он не подключён к Tauri commands или UI; все filesystem-тесты используют `TempDir`. Schema version остаётся 2.

## Что уже реализовано

- Frontend на React 19, TypeScript strict, Vite, Motion, React Router и Lucide; собственная CSS-дизайн-система без Tailwind.
- Минимальная оболочка Tauri 2 для macOS с hash-router, overlay title bar, CSP и минимальными window-разрешениями.
- Экраны главной, входящих, недели, списка и карточки ДЗ, AI Workspace, автоматизации и настроек.
- Единый типизированный `DesktopBridge`; React-компоненты обращаются к системным возможностям через bridge и хуки, прямого доступа к файловой системе из компонентов нет.
- Полноценный in-memory `MockDesktopBridge` с событиями, задержками, прогрессом, ошибками, журналом операций и mock-undo.
- Девятиэтапный workflow: материалы → анализ → промт → решение → проверка → исправления → преподаватель → к сдаче → отправлено.
- Модель источников и обязательных материалов, профиль личного стиля, результаты проверки, неподтверждённые утверждения, замечания и сравнение версий.
- Цикл первичного промта и промтов исправлений, создание новых версий без перезаписи старых, история итераций.
- Ввод вопросов/ответов преподавателя, извлечение требований и отдельная проверка по ним.
- Проверка готовности, сборка комплекта, ручной чек-лист отправки и ручное подтверждение отправки.
- Mock-защита отправленной версии как неизменяемого снимка с хэшем; фактическая отправка в систему вуза не автоматизируется.
- Mock-проверки подключения Codex/Claude без хранения паролей, cookies или сессионных токенов.
- Канонические относительные пути централизованы: `Предметы/00 Входящие`, предмет, `Неделя XX`, папки 01–04 и `04 Дополнительно`.
- Раскладка ДЗ обновляет `Homework.taskFiles` или кандидатов версии; при необходимости создаёт Homework и связывает его с Week.
- Операции mock выполняются в порядке start → progress → commit → регистрация undo → completed → resolve.
- Контракт native intake содержит `sourcePath`, `pickFiles()` и подписку на native drop; HTML5 drop сохранён для browser/mock.
- Оформлены `WorkflowRail`, `SourceList`, `ReviewResultView`, `StyleScore`, `RequirementList`, `PromptBox`, `RunMonitor`, `IterationChain`, `VersionComparePanel`, `TeacherCommentsPanel`, `SubmissionPanel` и `Tabs` на существующих CSS-токенах без Tailwind и новых UI-библиотек.
- Девятиэтапный rail показывает завершённые, текущий, будущие и заблокированные этапы, цикл исправлений и компактный вариант; длинные русские подписи раскладываются сеткой без наложения.
- Вкладки поддерживают roving focus, клавиши ←/→, Home/End, видимый focus, ARIA-связь вкладки с панелью и горизонтальную прокрутку.
- Для визуальной проверки добавлен минимальный mock-комплект готовых файлов; бизнес-правила сборки и отправки не изменялись.
- В Настройках добавлена секция «Рабочий процесс AI»: независимый выбор Codex/Claude для аналитика, автора и проверяющего, пояснение ролей и порог расхождения со стилем 0–10 (по умолчанию 4). Черновик поддерживает сохранение и сброс через `updateAutomationRules`.
- Сохранение ролей синхронизирует `ProviderStatus.roles` и `ProviderCard`, включая восстановление при undo. Новые запуски AI Workspace используют актуальные роли, а новые проверки и readiness — актуальный style threshold.
- Главная и стартовая анимация переведены на девять этапов. Главная показывает агрегированные количества активных, готовых и отправленных работ, а для каждого ближайшего ДЗ — текущий этап, ближайшее действие и ссылку на логичную вкладку карточки.
- Правило `workflowStage → вкладка → ближайшее действие` централизовано в `src/lib/workflow-ui.ts`; кнопки AI в карточке блокируются, если обязательные данные отсутствуют или назначенный провайдер не подключён.
- Числа у вкладок оформлены компактными счётчиками с `aria-label`; проверены roving focus и focus-visible. Читаемость будущих этапов `WorkflowRail` в тёмной теме увеличена без изменения активного состояния.
- `FRONTEND_HANDOFF.md` полностью синхронизирован с фактическими типами, методами, событиями, intake и зонами ответственности. Устаревшие `prepareAiPackage`, `runAiPackage`, `getAiRuns` и `markHomeworkSubmitted` удалены из описания контракта.

## Что не закончено

- `TauriDesktopBridge` создан, но пока не подключён к `createBridge()`; основной UI всегда получает mock-адаптер.
- Read-only Rust-команды и две DB-only write-команды готовы. Фундамент файловых операций/backup/undo реализован изолированно, но не подключён к production-командам; watcher и автоматический классификатор отсутствуют.
- Нет реального создания структуры Obsidian, заметок, ссылок и метаданных Finder.
- Нет реальных адаптеров локальных клиентов Codex и Claude; анализ, проверка и создание версий сейчас симулируются mock-слоем.
- Нет реального выбора папок через системный диалог и открытия Finder/официальных приложений провайдеров.
- Хэши разделов, комплект на сдачу и неизменяемый снимок реализованы только как mock-данные, а не как операции над файлами.
- Автоматические frontend-тесты покрывают routing, основной mock-workflow, роли/порог и правила ближайшего действия; Rust-тесты покрывают миграции, ограничения, транзакции и базовые репозитории. Компонентных и end-to-end Tauri-тестов нет.
- Нет автоматических screenshot/regression-тестов; визуальная проверка этапа 2 выполнена вручную в браузере.

## Известные проблемы и расхождения

- На старте этапа 1 подтверждено: `MockDesktopBridge.routeFiles()` направлял `homework_task`, `homework_data` и `homework_draft` в `Week.materials`. Исправлено: task/data попадают в `Homework.taskFiles`, draft — в кандидаты версии, отсутствующий Homework создаётся и связывается с Week.
- На старте этапа 1 подтверждено: `runOp()` вызывал `finishOp`/`operation:completed` до изменения состояния вызывающим методом. Исправлено единым commit-callback и полным снимком связанных сущностей для undo раскладки.
- На старте этапа 1 подтверждено: в `tauri.conf.json` было `dragDropEnabled: false`, а `DroppedItem` не передавал надёжный `sourcePath` из Finder. Исправлена frontend/Tauri-граница: native drop включён, добавлены `sourcePath`, `pickFiles()`, native-drop subscription и явная проверка отсутствующего пути; реализация Tauri-адаптера остаётся backend-задачей.
- На старте этапа 1 подтверждено: `weekFolder()` строил путь без корня `Предметы/`. Исправлено через централизованные относительные path helpers.
- `npm ci` сообщил, что install-script необязательного пакета `fsevents@2.3.3` не был автоматически разрешён. Установка и последующие проверки при этом завершились успешно.
- Production JavaScript bundle составляет 732.49 kB (222.13 kB gzip); Vite не выдал предупреждений, но разбиение bundle пока не выполнялось.
- На ширине 1100 px подписи девяти этапов намеренно уменьшены до компактного размера; при ещё более узких окнах потребуется отдельный мобильный режим rail.
- В seeded mock собранный комплект демонстрирует предупреждение и отсутствующее обязательное приложение, поэтому действия готовности/отправки ожидаемо заблокированы; неизменяемый отправленный снимок проверен на отдельном ДЗ.

## Визуальная проверка этапа 2

- Проверены размеры 1180×740 и 1100×700, тёмная и светлая темы; горизонтального переполнения страницы не обнаружено.
- Открыты ДЗ ОБА недели 7, исходные материалы, рабочий цикл, результат проверки, сравнение версий, комментарии преподавателя, комплект на сдачу, отправленная работа и AI Workspace.
- Проверены длинные пути и имена, длинный промт, несколько итераций, пустой комплект, загрузочные skeleton-состояния, ошибки запуска в истории, заблокированные действия, submitted snapshot, клавиатурная навигация и reduced motion.
- `AiWaveform` останавливает цикл при reduced motion и скрытом окне через общие preferences/window-visibility механизмы.
- Контрольные снимки находятся в `artifacts/ui-review/`.
- Во время первой браузерной проверки обнаружено обращение `RunMonitor` к `null` при отсутствии запуска; ошибка исправлена. После исправления новых console errors не появилось.

## Проверка этапа 3

- Открыты Главная, Настройки, AI Workspace и карточка ДЗ ОБА недели 7; проверены тёмная и светлая темы, 1180×740 и 1100×700.
- Проверены сохранение каждой роли и порога, сброс несохранённого черновика, обновление ролей ProviderCard и возврат демонстрационных значений по умолчанию.
- В AI Workspace видны выбранные роли и текущий порог; в карточке ДЗ недоступна кнопка передачи промта неподключённому автору.
- Клавиши ←/→ переводят активную вкладку и фокус; счётчики версий/комментариев имеют `aria-label`. На проверенных размерах `scrollWidth` равен `clientWidth`.
- Console errors на всех проверенных экранах отсутствуют.
- Новые снимки: `dashboard-workflow-dark.png`, `settings-workflow-dark.png`, `settings-workflow-light.png`, `dashboard-workflow-light.png` в `artifacts/ui-review/`.

## Остаётся исключительно для backend

- Расширение write-контракта до полного набора, необходимого UI, и только затем отдельное решение о подключении `TauriDesktopBridge`; сейчас основной UI остаётся на mock.
- Native picker и window drag-and-drop с обязательным `sourcePath`.
- Реальные watcher, классификация, файловые операции, backup/undo, persistent storage и дедупликация на диске.
- Реальные адаптеры официальных клиентов Codex/Claude, прогресс и отмена запусков без хранения секретов.
- Хэши версий, сборка submission package и неизменяемый submitted snapshot на диске; фактическая отправка в LMS остаётся ручной.

## DB-only мутации — этап 4C.2B

- Зарегистрированы `update_automation_rules` и `update_classification`. Обе команды изменяют только SQLite и не выполняют файловых операций.
- `MutationService` отделяет транзакционные инварианты от тонких Tauri-команд. Репозиторий `writes` использует параметризованный SQL; успешный результат перечитывается через существующие read-модели.
- Каждая попытка получает UUID operation id. Успех атомарно проходит `running → commit данных → completed`; ошибка откатывает целевую транзакцию и сохраняет одну отдельную `failed`-операцию. Эти операции имеют `undoable: false`.
- `updateClassification` использует явный tri-state patch: `absent` сохраняет поле, `null` очищает subject/week либо устанавливает category `unknown`, `value` задаёт значение. Проверяются существование предмета, принадлежность недели предмету и запрет изменения уже разложенного файла.
- Ручная классификация записывает source `manual`, confidence `1`, безопасное объяснение и вычисляет `classified` только при полном наборе subject + week + category; иначе статус `ambiguous`.
- После успешного commit эмитятся реальные события: для классификации `inbox:classification-updated`, затем `operation:completed`; для правил — `operation:completed`. После записанной ошибки эмитится `operation:failed`.
- Ошибка доставки события не откатывает уже сохранённые данные и возвращается как контролируемая ошибка с указанием перечитать состояние.
- Низкоуровневый IPC и `TauriDesktopBridge` делегируют обе команды без HTTP, mock fallback и скрытой подмены backend. `createBridge()` не изменён.
- Секретоподобные и неизвестные поля AutomationRules отклоняются; пороги и инварианты подтверждений проверяются до записи.
- Всего проходит 66 Rust- и 43 frontend-теста. Frontend-дизайн, CSS, миграции и schema version не изменялись.

## Безопасный файловый сервис — этап 4D.1

- Добавлен модуль `file_service` из `paths`, `hashing`, `planning`, `atomic_copy`, `backup`, `undo`; подробная модель угроз описана в `docs/FILE_SERVICE_SECURITY.md`.
- `VaultRoot`, `InboxRoot`, `BackupRoot`, `SourceFile`, `PersistentPath` и `VerifiedDestination` не дают смешивать абсолютные runtime paths и сериализуемые относительные пути.
- Корни canonicalized и не могут быть `/`, домашним каталогом, широким системным каталогом или неявным `.`. Source обязан быть абсолютным обычным файлом и не может быть symlink.
- Component-based проверка persistent path и повторная canonical-проверка parent-каталогов блокируют traversal и symlink escape.
- SHA-256 читается потоком по 64 KiB. Planner различает `Copy`, `SameContent`, `NameConflict` и предлагает детерминированные имена с сохранением расширения.
- Atomic copy использует same-directory `.kontur-<uuid>.tmp`, `create_new`, streaming copy, flush/sync, size/hash verification и `RENAME_NOREPLACE`; overwrite отсутствует, cleanup управляется RAII.
- Backup создаётся до изменения, атомарно и без overwrite. CamelCase manifests содержат только относительные paths, hashes, размеры, operation ID и время.
- Undo foundation проверяет `undoable`, повторный undo, submitted Homework, контролируемые roots и неизменность created/backup файлов по SHA-256. Применение undo ещё не подключено.
- Добавлено 36 filesystem/security тестов; всего проходит 102 Rust- и 43 frontend-теста. Все файловые fixtures находятся в `TempDir`.
- Новых Tauri commands, filesystem capabilities, startup actions, frontend/CSS изменений и schema migration нет. UI остаётся на `MockDesktopBridge`.

## TauriDesktopBridge и events — этап 4C.2A

- `TauriDesktopBridge` типобезопасно реализует весь `DesktopBridge`. Настоящий IPC используется для `getSubjects`, `getWeeks`, `getWeek`, `getInboxFiles`, `getHomeworkList`, `getHomework`, `getAutomationRules`, `getActivityLog`.
- Только `getWeek` и `getHomework` преобразуют `not_found` в `null`; остальные коды ошибок сохраняются. Неизвестный backend-код нормализуется в `unknown` с исходным сообщением.
- Все ещё отсутствующие методы возвращают `backend_unavailable`; фиктивного успеха, mock fallback и hybrid bridge нет. Синхронный `onNativeFileDrop` выбрасывает ту же структурированную ошибку.
- Добавлена явная `createTauriBridge()`, но существующий `createBridge()` не изменён и продолжает создавать `MockDesktopBridge`.
- Runtime detection централизован в `tauri/runtime.ts` и используется IPC и events. Вне Tauri нет HTTP-попыток или fallback.
- `tauri/events.ts` лениво импортирует официальный event API. Отписка безопасна до и после асинхронной регистрации, повторный вызов идемпотентен, callback после unsubscribe блокируется, ошибки регистрации контролируемо передаются error handler.
- На этапе 4C.2A `events.rs` добавил точные константы 12 событий, camelCase payload DTO и общий emit-helper через `tauri::Emitter`; этап 4C.2B подключил его к двум DB-only мутациям.
- Добавлены 6 Rust event-тестов и 15 frontend-тестов bridge/events/error boundary. Всего проходит 49 Rust- и 40 frontend-тестов.
- Дизайн, страницы, CSS, persistence contract и schema не изменялись.

## Read-only backend и IPC — этап 4C.1

- Добавлены точные camelCase Rust DTO для Subject, Week, FileRef, InboxFile, Homework, HomeworkVersion, AutomationRules и Operation.
- Репозитории разделены на `subjects`, `weeks`, `homeworks`, `files`, `inbox`, `operations`, `settings`; прежние write-методы сохранены в `mutations`. Агрегаты читаются фиксированным числом SQL-запросов без N+1.
- Добавлен тестируемый `QueryService`, который собирает полные DTO и преобразует ошибки чтения в единый `AppError`.
- Зарегистрированы `backend_health`, `get_subjects`, `get_weeks`, `get_week`, `get_homework_list`, `get_homework`, `get_inbox_files`, `get_automation_rules`, `get_activity_log`.
- `get_week` и `get_homework` возвращают `not_found`; повреждённый JSON, неизвестный enum и небезопасный путь возвращают `database` без panic.
- Inbox DTO не раскрывает persistent `sourcePath` или внутренний relative path. Activity log отсортирован по `startedAt DESC, id DESC`.
- AutomationRules читаются типизированно; отсутствующие безопасные поля дополняются defaults, включая style threshold 4. Secrets по-прежнему запрещены.
- Низкоуровневый `src/lib/bridge/tauri/ipc.ts` лениво загружает официальный `invoke`, проверяет Tauri runtime, типизирует девять команд и нормализует ошибки в `BridgeError`. Fallback на mock отсутствует.
- Legacy nullable-связи сохраняются как null. Для пустых legacy полей версии используются документированные fallback basename, `legacy:<versionId>` и FileRef из относительного пути; обязательный отсутствующий deadline считается повреждённой записью.
- Добавлены 15 read-only Rust-тестов и 6 TypeScript IPC-тестов. Всего проходит 43 Rust- и 25 frontend-тестов.
- Документация архитектуры, persistence и frontend handoff синхронизирована с фактической границей 4C.1.

## Фундамент Rust backend — этап 4B

- Добавлены модули `app_state`, `commands`, `db`, `db/migrations`, `domain`, `error`, `events` и `repositories`; архитектура описана в `docs/BACKEND_ARCHITECTURE.md`.
- Production SQLite размещается в `<app_data_dir>/kontur.sqlite3`; включены foreign keys, busy timeout, WAL и `synchronous=NORMAL`. Тесты используют только in-memory SQLite.
- Добавлена встроенная транзакционная миграция схемы версии 1. Повторный запуск идемпотентен, а более новая несовместимая версия возвращает понятную ошибку `conflict`.
- Созданы все 15 запрошенных прикладных таблиц и служебная `schema_migrations`; добавлены внешние ключи, уникальные ограничения, индексы, timestamps и проверки статусов/этапов/относительных путей.
- Версии ДЗ создаются только отдельными frozen-записями. Репозиторий не предоставляет обновление/удаление версии и блокирует изменение отправленной карточки.
- Реализованы транзакционные операции настроек, предмета, недели, ДЗ, создания/списка версий, журнала операций и чтения версии схемы. Составная запись ДЗ и первой версии откатывается целиком при ошибке.
- Добавлена сериализуемая ошибка `{ code, message }` и безопасная Tauri-команда `backend_health`, возвращающая readiness, версии приложения/схемы, платформу и факт существования DB без раскрытия локальных путей.
- Добавлено 16 Rust unit tests: миграции, несовместимая версия, foreign keys, uniqueness, relative paths, settings, доменная цепочка, immutable versions/submitted homework, операции, rollback и отсутствие файловой test DB.
- `cargo fmt --check`, `cargo check`, строгий Clippy и `cargo test` успешны; frontend typecheck/19 тестов/build также прошли. Debug app-only сборка создана в `src-tauri/target/debug/bundle/macos/Контур.app` (31.23 MiB), DMG не запускался.
- Не реализованы `TauriDesktopBridge`, рабочие Tauri events/commands, native intake, watcher, файловые операции, AI-провайдеры и submission snapshot на диске — это граница следующих этапов.

## Синхронизация persistence — этап 4B.1

- Добавлена неизменяющая v1 миграция `v2.sql`; `CURRENT_SCHEMA_VERSION = 2`. Fresh database проходит v1→v2, заполненная v1 обновляется транзакционно с сохранением записей всех прежних таблиц.
- Добавлена карта `docs/PERSISTENCE_CONTRACT.md`: TypeScript ↔ Rust DTO ↔ SQLite, nullable/JSON/вычисляемые поля и источники истины.
- `HomeworkStage` и `HomeworkWorkflowStage` стали исчерпывающими Rust enum. Контрактное `ready` преобразуется во внутреннее `ready_to_submit`, девять workflow-стадий — в legacy DB-значения и обратно без raw strings во frontend.
- Subject, Week, Homework и HomeworkVersion дополнены недостающими контрактными полями; сложные sections/files сохраняются как JSON с `json_valid`.
- `SourceBundle.versionId`, `ReviewRun.versionId`, `RevisionPrompt.basedOnVersionId` и `reviewRunId` стали nullable. Проверен полный первичный SourceBundle и анализ до появления v01.
- Добавлена `inbox_files`: classification/duplicate сохраняются, абсолютный native `sourcePath` после импорта не записывается.
- Workflow, teacher, submission и operation таблицы дополнены полями frontend-контракта; RequirementStatus хранится как `done/not_done/manual/na`.
- SQLite triggers запрещают UPDATE/DELETE HomeworkVersion и SubmissionReceipt, изменение/удаление submitted Homework и добавление к ней новых дочерних данных.
- `app_settings` рекурсивно отклоняет очевидные ключи secrets; разрешённые категории и JSON-политика документированы.
- Rust tests покрывают fresh/populated migration, сохранность данных, enum mapping, nullable analysis, Inbox roundtrip, JSON/path validation, secret rejection и прямые нарушения immutable-инвариантов.
- Финальная проверка: Cargo fmt/check/Clippy/test, frontend typecheck/19 тестов/build и app-only Tauri debug build успешны. Собран `Контур.app` размером 31.25 MiB; DMG не запускался.
- Persistence-структуры готовы к будущим командам, но CRUD repository API для всех workflow-таблиц, Tauri commands/events и TypeScript adapter ещё не реализованы.

## Окружение Rust/Tauri — этап 4A

- Mac: Apple Silicon `arm64` (`aarch64-apple-darwin`), shell `/bin/zsh`.
- Xcode Command Line Tools: `/Library/Developer/CommandLineTools`; Apple clang 17.0.0, macOS SDK найден.
- Через официальный `rustup` для текущего пользователя без `sudo` установлен профиль `minimal`, stable toolchain `1.99.1-aarch64-apple-darwin`, target `aarch64-apple-darwin`.
- Версии: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `rustup 1.29.1 (d95a37b6a 2026-08-13)`, `rustfmt 1.10.0-stable`, `clippy 0.1.99`.
- `cargo fmt --check`, `cargo check` и Clippy с `-D warnings` завершены без замечаний. `cargo test` успешен; существующая оболочка пока содержит 0 Rust-тестов.
- `npm run tauri:dev` запустил Vite, собрал `target/debug/kontur` и открыл нативное macOS-окно «Контур» на `tauri://localhost` без ошибок в терминале. Dev-процесс завершён через `Ctrl+C`, порт и процесс после остановки отсутствуют.
- `npm run tauri:build -- --debug --bundles app` успешно собрал debug executable и `Контур.app` размером 28.94 MiB. Окно собранного приложения открыто и проверено, затем закрыто.
- Полная конфигурация bundle (`app` + `dmg`) компилирует приложение и создаёт `.app`, но DMG-этап в текущем окружении завершается сообщением: `failed to bundle project: error running bundle_dmg.sh: hdiutil: create failed - Устройство не сконфигурировано`. Для проверки кода и `.app` это не блокирует этап 4A; signing и notarization не запускались.
- `src-tauri/target/`, `dist/`, `node_modules/`, локальные базы, `.env`, ключи и credentials уже исключены через `.gitignore`; менять его не потребовалось. Сборочные `.app`/`.dmg` остаются внутри ignored `target/` и в Git не добавляются.
- Cargo создал и зафиксировал `src-tauri/Cargo.lock` для воспроизводимого набора зависимостей существующего приложения. Rust-исходники, Tauri-конфигурация и frontend на этапе 4A не изменялись.

## Результаты команд

| Команда | Результат |
|---|---|
| `npm ci` | Успешно; добавлено 62 пакета |
| `npm run typecheck` | Успешно; `tsc -b`, ошибок нет |
| `npm run test` | Успешно; 6 test files, 43 теста пройдено |
| `npm run build` | Успешно; 2426 модулей, production-сборка Vite завершена |
| `cargo fmt --check` | Успешно; форматирование соответствует rustfmt |
| `cargo check` | Успешно; существующая Tauri-оболочка компилируется |
| `cargo clippy --all-targets --all-features -- -D warnings` | Успешно; предупреждений нет |
| `cargo test` | Успешно; 102 Rust-теста пройдено на этапе 4D.1 |
| `npm run tauri:dev` | Успешно; нативное окно открыто и корректно закрыто |
| `npm run tauri:build -- --debug --bundles app` | Успешно; debug `Контур.app` собран |

## Git

В `/Users/dominika82/Documents/New project/kontur` создан самостоятельный Git-репозиторий. Baseline исходного состояния зафиксирован отдельным коммитом; проект не добавлялся в родительский репозиторий. `node_modules`, `dist`, временные файлы, логи, локальные базы и секреты исключены через `.gitignore`.

## Целостность исходника

Оригинальный архив `/Users/dominika82/Desktop/работа/kontur-fixed.zip` не изменялся. Все изменения этапа 1 выполнялись только в самостоятельной рабочей копии.
