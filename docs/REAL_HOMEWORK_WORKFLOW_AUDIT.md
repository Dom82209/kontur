# Аудит реального цикла домашней работы

Дата аудита: 27 октября 2026 года

Проверяемая база: `d51564f5df03ac66ed19593f2d391c289d35616e`

Ветка аудита: `codex/audit-real-homework-workflow`

Схема SQLite: `8`

## 1. Краткий вердикт

**NO-GO для настоящих домашних работ как для системы, которой можно доверить решение о готовности и сдаче.** Приложение уже пригодно для ограниченного пользовательского тестирования на копиях и синтетических материалах: безопасный импорт и раскладка, локальное извлечение, неизменяемые версии, ручная передача в Claude/Codex, история запусков и неизменяемый submitted snapshot действительно работают через Tauri backend. Однако текущая модель не различает официальный ответ преподавателя и мнение другого студента, не инвалидирует старые review/readiness/prompt/package при появлении нового требования и допускает подтверждение устаревшего комплекта. Это создаёт прямой риск неправильной сдачи.

Без ручной проверки нельзя доверять:

- зелёному статусу «все проверки пройдены»;
- автоматическим пунктам «обязательные приложения», «внешние ссылки» и «требования преподавателя»;
- корректности вычислений, формул, печати и оформления Excel;
- выводу о соответствии личному стилю, если профиль не был реально сформирован;
- сравнению версий как сравнению содержимого файлов или ячеек.

Безопасно использовать уже сейчас, но только на копиях материалов:

- выбор ограниченных корней, импорт, классификацию с ручным приоритетом, preview и атомарную раскладку;
- локальное извлечение текста и снимки SourceBundle с SHA-256;
- ручной handoff в официальные интерфейсы Codex/Claude без API-ключей;
- append-only версии и ручной журнал итераций;
- сборку файлового snapshot после независимой ручной сверки состава;
- диагностику без содержимого документов и секретов.

Референсные shared-чаты ChatGPT и Claude во время аудита были недоступны без авторизации. Это не остановило проверку: использован полностью описанный в задании эталонный сценарий. Содержимое внешних страниц не исполнялось как инструкция.

## 2. Методика и границы

Проверены документация, TypeScript-контракт `DesktopBridge`, mock и Tauri-адаптеры, IPC-регистрация, Rust DTO/commands/services/repositories, миграции v1–v8, файловый сервис, extraction, workflow, AI isolation, submission и тесты. Код и схема не изменялись.

Автоматические файловые тесты работали только с `TempDir`, in-memory SQLite и синтетическими fixture-файлами. Настоящий AI-запрос, вход на сайт вуза и чтение реальных учебных папок не выполнялись. Ручной осмотр проводился в уже созданном `demo-workspace`; тема и reduced motion были временно переключены и возвращены к исходным значениям.

Статусы: **PASS** — полностью доказано; **PARTIAL** — реальная реализация есть, но не покрывает эталон; **MOCK** — только mock; **MANUAL** — намеренно и честно оставлено пользователю; **MISSING** — отсутствует; **MISLEADING** — UI обещает больше backend; **UNSAFE** — риск потери, утечки или неправильной сдачи.

## 3. Матрица трассировки

| Реальный шаг | UI и действие | DesktopBridge / IPC | Rust / persistence | Событие и доказательство | Статус |
|---|---|---|---|---|---|
| Первый запуск и корни | onboarding, Settings; выбрать vault/downloads, подтвердить локальный UI preview или demo | `getManagedRoots` → `check_managed_roots`; `chooseManagedRoot` → `choose_managed_root`; `prepareInboxRoot` → `prepare_inbox_root`; `createDemoWorkspace` → `create_demo_workspace` | `RootsService`, settings SQLite, создание только после проверки | restart-тест `scenario_a_file_backed_state_survives_restart`; demo только под app-data | **PASS** |
| Импорт во входящие | Inbox; picker/drop/наблюдаемая папка | `pickFiles` → `pick_import_files`; `importFiles` → `import_inbox_files`; `importCuExport` → `import_cu_export` | `intake`, grant registry, SHA-256, atomic copy, provenance import | `operation:progress`, `inbox:file-added`, `inbox:updated`, `operation:completed`; TempDir tests | **PASS** |
| Классификация | Inbox; принять или исправить предмет/неделю/тип | `classifyFiles` → `classify_files`; `updateClassification` → `update_classification` | `lms::classify`, `MutationService`, SQLite | `inbox:classification-updated`; ambiguous не маршрутизируется, manual имеет приоритет | **PASS** |
| Preview раскладки | Inbox; просмотреть exact plan | `previewRoute` → `preview_route` | `routing`, fd-relative file service | source/target/action/hash/conflict/folders; тест preview без записи | **PASS** |
| Подтверждённая раскладка/undo | Inbox/журнал; подтвердить или отменить | `routeFiles` → `route_files`; `undoOperation` → `undo_operation` | atomic copy, manifests, compensation, SQLite transaction | `operation:progress`, `operation:completed`, `operation:failed`; symlink/race/corruption/repeat-undo tests | **PASS** |
| Obsidian | route/package/submission; открыть заметку | `openObsidianNote` → `open_obsidian_note` | managed block writer, `managed_notes` | пользовательский текст вне маркеров сохраняется | **PASS** |
| Подборка материалов | Materials/карточка ДЗ; выбрать файлы и анализ | `setCorpusSelection` → `set_corpus_selection`; `analyzeMaterials` → `analyze_materials`; `prepareSourceBundle` → `create_source_bundle` | extraction cache + immutable `source_bundles`, `source_bundle_items` | `extraction:progress`, `extraction:updated`, `homework:updated`; SHA/extractor version/freshness tests | **PARTIAL**: style MD не имеет рабочего пути в профиль; authority отсутствует |
| Первый анализ | AI Workspace; preview, ручной ввод или provider | `runMaterialAnalysis` → `start_workflow_run`; `prepareAiJob` → `prepare_ai_job`; `saveManualAnalysis` → `save_manual_analysis`; `confirmAiResult` → `confirm_ai_result` | `WorkflowService`, `AiService`, `review_runs`, `ai_jobs` | `review:progress`, `review:completed`, `review:failed`, `ai-job:progress`, `ai-job:updated`; exact prompt/context видимы; результат не применяется без confirm | **PARTIAL**: семантическая истинность — утверждение пользователя/AI |
| Промт для Claude | AI Workspace; просмотреть/копировать, открыть официальный Claude | `createRevisionPrompt` → `create_revision_prompt`; `sendPromptToAuthor` → `hand_off_prompt`; `openProviderApp` → `open_provider_app` | immutable committed prompt, provenance | `homework:revision-prompt-created`, `homework:updated` | **MANUAL** и корректно: Claude automatic заблокирован |
| Новая версия | карточка ДЗ; выбрать новый файл | `createHomeworkVersion` → `create_homework_version`; `markVersionFinal` → `select_homework_version` | append-only `homework_versions`, frozen triggers | `homework:version-created`, `homework:updated`, restart tests | **PARTIAL**: нет parent/provider/changed cells/rejection reason |
| Повторная проверка | Result/AI Workspace | `runVersionReview` → `start_workflow_run`; manual/provider confirmation | отдельный review по version + bundle | `review:progress`, `review:completed`, `review:failed`, `ai-job:updated`; version-bound tests | **PARTIAL**: нет детерминированной проверки вычислений/оформления |
| Импорт переписки | Comments; paste/TXT/MD preview, явное подтверждение | `previewTeacherTranscript` → `preview_teacher_transcript`; `importTeacherTranscript` → `import_teacher_transcript`; `saveTeacherComments` → `add_teacher_comments` | parser + `teacher_comments` | preview без записи; IPC tests | **PARTIAL**: пары есть, но авторитетности нет |
| Требование преподавателя | Comments; вручную создать/закрыть требование | `addTeacherRequirement` → `add_teacher_requirement`; `resolveRequirement` → `resolve_teacher_requirement` | `teacher_requirements` | `homework:updated`, `homework:readiness-updated` | **UNSAFE**: новое требование не делает старые артефакты stale |
| Style profile | Settings/result | `getStyleProfile` → `get_style_profile` | только чтение `style_profile_sources` | нет production-команды вставки/выбора | **MISSING** для реального формирования; mock-профиль не доказательство |
| Readiness | Package; «Отметить готово» | `getWorkflow` → `get_workflow`; `markReadyToSubmit` → `mark_ready_to_submit` | `projections::readiness`, state machine | `homework:readiness-updated`, revision + checks | **UNSAFE**: `styleScore=null` проходит; нет dependency revision для комментариев |
| Сборка комплекта | Package; «Собрать/Пересобрать» | `buildSubmissionPackage` → `build_submission_package`; `setPackageCheck` → `set_package_check` | `SubmissionService`, files + manifest | `operation:completed` | **MISLEADING/UNSAFE**: rebuild возвращает старый package; часть auto-checks фиктивна |
| Ручная сдача | Package; checklist, явное подтверждение | `confirmSubmission` → `confirm_submission` | receipt + immutable snapshot + SHA-256 manifest | `submission:confirmed`; direct update/delete blocked | **PARTIAL**: snapshot надёжен, но может фиксировать устаревший package |
| Перезапуск | закрыть/открыть | обычные query methods | file-backed SQLite | restart tests и ручное открытие demo | **PASS** для сохранённых сущностей |
| Диагностика | Settings; экспорт | `exportDiagnosticReport` → `export_diagnostic_report` | агрегированные статусы без document content/paths/secrets | dedicated regression test | **PASS** |

В `FirstRunWizard` preview структуры — локальный boolean-state UI (`src/components/domain/FirstRunWizard.tsx:14`, `src/components/domain/FirstRunWizard.tsx:54-64`, `src/components/domain/FirstRunWizard.tsx:87-93`). Он не вызывает отдельную backend-команду: после локального предпросмотра подтверждение вызывает `prepareInboxRoot` (`src/components/domain/FirstRunWizard.tsx:58`); текущее состояние корней читается через `getManagedRoots`, выбор выполняется через `chooseManagedRoot`, а demo создаётся через `createDemoWorkspace` (`src/components/domain/FirstRunWizard.tsx:10`, `src/components/domain/FirstRunWizard.tsx:33`, `src/components/domain/FirstRunWizard.tsx:51`).

## 4. Обязательный сквозной сценарий

Сценарий 1–8 технически можно собрать из существующих команд на синтетических данных: создать сущности, выбрать материалы, сохранить ручной review, prompt, v02, повторный review и package. Шаг 9 уже невыразим: DTO комментария имеет только канал (`chat/email/lms/in_person/other`), дату, вопрос и ответ; роли «другой студент» и «официальный преподаватель» отсутствуют. Поэтому мнение студента невозможно сохранить так, чтобы оно принципиально не стало «комментарием преподавателя».

Шаги 10–11 тоже не проходят: после нового подтверждённого требования старые review, readiness, final selection, prompt и package не помечаются stale. `buildSubmissionPackage` не пересобирает существующий пакет. Следовательно, корректно продолжить шаги 12–20 без изменения продуктового кода невозможно. В соответствии с ограничением аудита новый тест и обходная реализация не добавлялись.

Минимальный будущий regression-тест должен:

1. В TempDir создать v01 → review/pass → prompt → v02 → review/pass → ready → package.
2. Добавить комментарий с `role=student`, `official=false` и доказать отсутствие нового требования/инвалидации.
3. Добавить `role=teacher`, `official=true`, новый conflicting requirement.
4. Проверить: stage откатился, review/prompt/final/package stale, confirm старого package запрещён.
5. Создать correction prompt → v03 → новый bundle/review → новый package → receipt.
6. Перезапустить backend и проверить полную цепочку и неизменяемость receipt.

## 5. Проблемы

### P0-1. Устаревший комплект можно подтвердить после нового требования

- **Доказательство:** `add_teacher_comments` и `add_teacher_requirement` только вставляют строки (`src-tauri/src/workflow/service.rs:1518`, `src-tauri/src/workflow/service.rs:1569`); readiness выбирает последний completed review текущей версии без сравнения с ревизией комментариев (`src-tauri/src/workflow/projections.rs:76`); package build возвращает существующую запись без revalidation (`src-tauri/src/submission.rs:105`); `confirm` проверяет только сохранённый checklist (`src-tauri/src/submission.rs:349`).
- **Сценарий:** v02 прошла review, стала ready, package собран; затем добавлено официальное требование с `not_done`; старый package остаётся, а после ранее отмеченного manual checklist может быть превращён в receipt.
- **Ожидалось:** атомарная инвалидация review/readiness/prompt/final/package и запрет confirm.
- **Фактически:** увеличивается revision агрегата, но dependency state не меняется; confirm заново readiness не вычисляет.
- **Минимальное направление:** dependency revision/generation для authoritative inputs; `stale_at/stale_reason`; transactional invalidation; confirm обязан заново проверить stage, current version, review generation и package generation.
- **Тесты:** описанный обязательный E2E; race между requirement и confirm; restart после invalidation; old operation replay.

### P1-1. Нет модели авторитетности источников и официальности ответа

- **Доказательство:** `TeacherCommentInput` не содержит автора, роли, цитаты/контекста, authority и official confirmation (`src/lib/bridge/workflow-types.ts:281-286`); таблица хранит только question/answer/source/date (`src-tauri/src/db/migrations/v1.sql:125`, `src-tauri/src/db/migrations/v2.sql:125`). `SourceItem` также не содержит trust/authority (`src/lib/bridge/workflow-types.ts:55`).
- **Сценарий:** вставить ответ одногруппника из чата и ответ преподавателя из того же чата — backend видит одинаковый тип `chat`.
- **Ожидалось:** приоритет teacher > task > course > confirmed calculation > AI > student и явное подтверждение official.
- **Фактически:** все импортированные пары объявляются комментариями преподавателя; UI сразу предлагает создать из них требования.
- **Минимальное направление:** отдельные `actorRole`, `authority`, `officialConfirmedByUser`, quote/context и conflict relation; требования разрешать только из official source.
- **Тесты:** student opinion не создаёт requirement; conflict resolution; downgrade/revoke official confirmation; audit trail.

### P1-2. Реальный профиль стиля не формируется, а отсутствие оценки считается успехом

- **Доказательство:** migration создаёт `style_profile_sources` (`src-tauri/src/db/migrations/v8.sql:16`), а production-код только читает последнюю строку (`src-tauri/src/submission.rs:47`). Поиск по репозиторию не обнаружил INSERT/UPDATE или bridge-команды выбора источников. При `style_score=None` readiness возвращает `true` (`src-tauri/src/workflow/projections.rs:192-203`).
- **Сценарий:** выполнить ручной review без style score. В debug UI одновременно отображаются «все проверки пройдены» и «Расхождение со стилем не оценивалось».
- **Ожидалось:** выбранные style MD/previous/final источники формируют профиль; отсутствие оценки требует ручного подтверждения.
- **Фактически:** real backend обычно возвращает `null`, но style gate зелёный.
- **Минимальное направление:** explicit selection command + local trait extraction/provenance; `null => manual/block`; confirmation привязать к review и профилю.
- **Тесты:** no profile, deleted/changed profile, explicit selection only, threshold boundary, restart.

### P1-3. Комплект и его автоматические проверки завышают гарантии

- **Доказательство:** существующий package возвращается как «уже собран» (`src-tauri/src/submission.rs:105-123`), хотя UI предлагает «Пересобрать комплект» (`src/components/domain/SubmissionPanel.tsx:140-147`). Assignment URL сразу получает `verified=true` без сетевой проверки (`src-tauri/src/submission.rs:171-180`). Attachments/name выставляются `true` без отдельной модели обязательности (`src-tauri/src/submission.rs:183-213`). Для teacher check SQL исключает только `not_done`, поэтому `manual` ошибочно проходит (`src-tauri/src/submission.rs:128-135`).
- **Сценарий:** собрать package, сменить final/requirement; нажать «Пересобрать» — backend возвращает старый. Добавить requirement=`manual` — package считает teacher check выполненным.
- **Ожидалось:** новый immutable package generation или безопасная замена draft после revalidation; manual остаётся blocker.
- **Фактически:** один UNIQUE package на ДЗ и no-op rebuild; UI показывает auto success.
- **Минимальное направление:** versioned packages, stale flag, explicit required attachments/links, реальная или ручная link verification, единая семантика `manual`.
- **Тесты:** rebuild after final change, missing required attachment, unreachable link/manual check, requirement manual/not_done, old package confirm.

### P1-4. Цепочка версий не хранит требуемую родословную и фактический diff

- **Доказательство:** при создании version `source_bundle_id=None`, parent отсутствует (`src-tauri/src/workflow/service.rs:1450-1461`); repository всегда пишет `sections_json='[]'` (`src-tauri/src/workflow/repository.rs:313-318`). Сравнение сопоставляет строки замечаний и sections (`src-tauri/src/workflow/projections.rs:303-395`, `src-tauri/src/workflow/projections.rs:401-447`), а не файлы/формулы/ячейки.
- **Сценарий:** импортировать v02 с изменённой формулой — версия append-only сохранится, но compare не покажет изменённую ячейку и не докажет parent v01.
- **Ожидалось:** parent, provider/import origin, changed files/cells, acceptance/rejection reason.
- **Фактически:** есть номер, файлы, prompt link и note; родство выводится только порядком.
- **Минимальное направление:** immutable lineage entity и format-aware diff artifact, привязанный к двум SHA-256.
- **Тесты:** branch/parent, same content, formula-only change, reject reason, restart.

### P2-1. Review pipeline сохраняет структурированное мнение, но не доказывает корректность

- **Доказательство:** `ReviewAssessmentDto` хранит статусы и счётчики (`src-tauri/src/workflow/dto.rs:400-413`); ручной или подтверждённый AI-черновик становится completed review. Backend проверяет форму и внутреннюю согласованность, но сам не пересчитывает задачу, формулы или оформление.
- **Сценарий:** пользователь вручную вводит `pass`, нулевые счётчики и `style=null`; UI показывает проверку пройденной.
- **Ожидалось:** для технически непроверяемого критерия — `manual` и отдельное подтверждение, для автоматического — evidence/checker provenance.
- **Фактически:** поля review воспринимаются readiness как факты вне зависимости от метода получения.
- **Минимальное направление:** per-criterion evidence + verifier kind (`backend/provider/manual/external`) и запрет auto-pass без доказательства.
- **Тесты:** manual criterion blocks, provider error cannot pass, evidence/source mismatch.

### P2-2. Excel проверяется только на уровне безопасного извлечения

- **Доказательство:** extractor прямо сообщает, что формулы не вычисляются и используются cached values (`src-tauri/src/extraction/xlsx.rs:1-6`, `src-tauri/src/extraction/xlsx.rs:207-215`); hidden sheets и formula text читаются (`src-tauri/src/extraction/xlsx.rs:228-260`, `src-tauri/src/extraction/xlsx.rs:700-751`); comments пропускаются (`src-tauri/src/extraction/xlsx.rs:186-193`), external links не обновляются (`src-tauri/src/extraction/xlsx.rs:73-81`), charts/pivot не пересчитываются (`src-tauri/src/extraction/xlsx.rs:175-203`). Нет parsing print area/orientation/fit-to-page и diff формул между версиями.
- **Сценарий:** workbook с устаревшим cached value, broken external link или неверной print area может получить AI review `pass`.
- **Ожидалось:** UI разделяет автоматическую проверку, AI-эвристику и обязательную внешнюю/ручную проверку.
- **Фактически:** extractor честно предупреждает, но readiness не имеет Excel-specific gates.
- **Минимальное направление:** deterministic workbook report, explicit manual gates, optional read-only Excel/LibreOffice validation вне backend.
- **Тесты:** все error tokens, stale cache, print settings, hidden sheets, hyperlinks/comments, shared-formula diff.

### P2-3. Шаблон correction prompt не гарантирует «хирургический» контракт

- **Доказательство:** instruction требует лишь устранить перечисленные проблемы и не добавлять новых задач (`src-tauri/src/ai/context.rs:122-125`). Нет обязательных полей для preserve/forbid/control values/printing/new filename/no overwrite/self-check.
- **Сценарий:** AI возвращает общий текст «исправить ошибки»; схема принимает его как валидный prompt.
- **Ожидалось:** структурированная спецификация изменений и render перед handoff.
- **Фактически:** свободная строка.
- **Минимальное направление:** typed correction plan + renderer, каждый раздел обязателен или явно `not_applicable`.
- **Тесты:** missing preserve list, overwrite prohibition, filename, control values, local-only edit.

### P2-4. SourceKind шире реального маршрута данных

- **Доказательство:** контракт объявляет `style/current_version/previous_version/teacher_comments` (`src/lib/bridge/workflow-types.ts:43-53`), но classifier mapping source bundle выдаёт только lecture/seminar/task/data/extra/previous_work (`src-tauri/src/workflow/bundle.rs:382-401`). Требуется только task; отсутствие course material — warning, не blocker (`src-tauri/src/workflow/bundle.rs:222-243`).
- **Сценарий:** пользователь выбирает style MD как обычный material — он попадает как `extra`, не как style profile.
- **Ожидалось:** каждый эталонный источник имеет реальный typed ingestion и provenance.
- **Фактически:** часть типов существует в DTO/mock, но не создаётся production path.
- **Минимальное направление:** единый source registry с explicit user classification и authority.
- **Тесты:** каждый SourceKind из production UI до context payload; required policy per run kind.

### P2-5. История итераций неверно подписывает ручную проверку именем провайдера

- **Доказательство:** Result page учитывает `provenance === manual` (`src/pages/HomeworkDetailPage.tsx:489`), но `IterationChain` всегда пишет `providerName(review.provider)` (`src/components/domain/IterationChain.tsx:94`).
- **Сценарий:** demo review сохранён вручную; Result показывает «Вручную», цепочка — «Проверка Codex».
- **Ожидалось:** единая честная provenance label.
- **Фактически:** UI создаёт впечатление реального запуска Codex.
- **Минимальное направление:** общий provenance formatter.
- **Тесты:** manual/provider/legacy render cases.

### P3-1. Документация содержит исторические взаимоисключающие утверждения

- **Доказательство:** в `BACKEND_ARCHITECTURE.md` и `IMPLEMENTATION_STATUS.md` сохранились ранние описания «этапа 8» рядом с поздними реализациями этапов 10–13; comments в UI props также говорят «на следующем этапе» (`src/components/domain/TeacherCommentsPanel.tsx:25`).
- **Сценарий:** разработчик определяет возможности по первой найденной секции и получает устаревшую картину.
- **Ожидалось:** один актуальный capability table и архив changelog.
- **Фактически:** статус приходится восстанавливать по коду.
- **Минимальное направление:** отделить historical notes, пометить superseded sections.
- **Тесты:** doc link/checklist lint, не runtime.

## 6. Заявлено в UI, но не доказано backend

| Заявление UI | Что реально доказано | Статус |
|---|---|---|
| «Все проверки пройдены» при «Стиль не оценивался» | `None` принудительно считается style success | **MISLEADING** |
| «Обязательные приложения на месте · проверяется автоматически» | все файлы версии считаются required; отдельного задания об обязательных приложениях нет | **MISLEADING** |
| «Внешние ссылки проверены» / «доступна» | `https` сохраняется как `verified=true`, доступность не проверяется | **MISLEADING** |
| «Требования преподавателя учтены» | `manual` не блокирует package; новые требования не инвалидируют его | **UNSAFE** |
| «Пересобрать комплект» | существующий package возвращается без пересборки | **MISLEADING** |
| «Проверка Codex» в цепочке | ручной review тоже получает configured provider и подписывается как Codex | **MISLEADING** |
| «Сравнить версии» | сравниваются замечания/пустые sections, не бинарное или ячеечное содержимое | **PARTIAL** |
| SourceKind `style` | DTO/mock есть, production insertion/profile builder отсутствует | **MOCK/MISSING** |

## 7. Реальные возможности проверки Excel

| Возможность | Backend | Codex по переданному тексту | Excel/LibreOffice | Гарантия |
|---|---|---|---|---|
| Листы и скрытые листы | читает имена, hidden flag, declared/used range | может рассуждать по выдержке | не требуется для чтения | **PARTIAL**, лимиты/обрезка |
| Значения | читает сохранённые значения | может анализировать cached values | нужен для актуального пересчёта | backend не гарантирует актуальность |
| Формулы | читает formula text; shared dependent formula описывается через master | может искать подозрительные формулы | нужен для вычисления | формулы не вычисляются |
| `#REF!/#DIV/0!/#VALUE!/#NAME?` | error cell text сохраняется; regression явно есть для `#DIV/0!` | может заметить только попавшее в context | нужен для полного пересчёта | нет отдельного blocking scanner всех ошибок |
| Комментарии | не извлекаются | не видит | внешний просмотр | невозможно гарантировать backend |
| Гиперссылки/external links | внешние связи не открываются; cell hyperlink не моделируется | ограниченно | внешний просмотр/проверка | не проверены |
| Print area/orientation/fit-to-page | не читаются | не видит | Excel/LibreOffice/ручной preview | обязательно вручную |
| Метаданные | базовые OOXML properties читаются | может интерпретировать | внешний просмотр при необходимости | частично |
| Диаграммы/pivot/macros | не исполняются, предупреждение | не видит содержимое полностью | внешний просмотр/пересчёт | не проверены |
| Diff формул/ячеек между версиями | отсутствует | только если вручную передать оба извлечения | внешнее сравнение или новый checker | не гарантируется |

Итого: backend является безопасным extractor, а не Excel calculation/validation engine. Codex может дать эвристическое заключение только по обрезанному текстовому представлению. Проверка пересчёта, печати и визуального оформления требует Excel/LibreOffice и явного ручного gate.

## 8. Провайдеры и безопасность AI

- **PASS:** production Tauri всегда выбирает `TauriDesktopBridge`; mock доступен тестам и явному browser demo (`src/lib/bridge/index.ts:14-17`). IPC error не подменяется mock.
- **PASS:** Codex account mode включается только после versioned consent, isolation self-test, supported CLI и `logged_in=true` внутри изоляции (`src-tauri/src/ai/service.rs:333-479`, `src-tauri/src/ai/service.rs:543-601`). Наличие CLI само по себе недостаточно.
- **PASS:** isolation даёт private HOME/CODEX_HOME, разрешает только job и проверенный account `auth.json`, не читает пользовательский `config.toml`, очищает API-key env и блокирует другие job/user dirs (`src-tauri/src/ai/isolation.rs:139-267`, `src-tauri/src/ai/isolation.rs:314-510`). API-key auth.json не монтируется.
- **PASS:** Claude production остаётся manual-only, пока settings/hooks/MCP/slash commands не изолированы (`src-tauri/src/ai/service.rs:382-389`, `src-tauri/src/ai/service.rs:545-551`).
- **PASS:** UI различает «подписка аккаунта», «ручная передача» и «не авторизован» (`src/components/domain/ProviderCard.tsx:48-52`). Во время ручной проверки Codex и Claude честно показывались как manual.
- **PASS:** prompt/context показываются до отправки; AI response применяется только после подтверждения. Реальный AI-запрос в аудите не выполнялся.
- **PARTIAL:** prompt-injection снижена явной инструкцией считать документы недоверенными данными (`src-tauri/src/ai/context.rs:76-81`) и OS isolation, но семантическое следование модели нельзя доказать локально. Поэтому AI output остаётся недоверенным.

## 9. Что остаётся ручным и почему

- Передача prompt и получение результата Claude — намеренно manual, пока нет доказанной изоляции Claude Code.
- Любая отправка в LMS, финальный click и подтверждение факта отправки — manual; cookies/passwords не хранятся.
- Проверка фактической корректности решения и вычислений — manual/AI-assisted, не deterministic backend.
- Пересчёт Excel, print preview, визуальное оформление, comments/charts/pivot/external links — внешний Excel/LibreOffice и manual confirmation.
- Установление официальности ответа преподавателя сейчас полностью ручное и при этом не моделируется; до исправления такие данные нельзя считать надёжным gate.
- Состав обязательных приложений и доступность внешних ссылок нужно независимо сверять вручную.
- Style score — эвристика, не AI detector; без реального профиля он не должен использоваться как gate.

## 10. Проверки и ручной осмотр

Автоматические проверки:

- `cargo fmt --check` — PASS.
- `cargo check` — PASS.
- `cargo clippy --all-targets --all-features -- -D warnings` — PASS.
- `cargo test` во внешней managed sandbox — инфраструктурный FAIL: 360 passed, 22 AI sandbox tests получили macOS `sandbox-exec` exit 71, 1 ignored.
- тот же `cargo test` вне внешней sandbox — PASS: 382 passed, 0 failed, 1 ignored (`tesseract` не установлен). Настоящий AI не вызывался.
- `npm run typecheck` — PASS.
- `npm run test` — PASS: 17 files, 124 tests.
- `npm run build` — PASS.
- `npm run tauri:build -- --debug --bundles app` — PASS; bundle 64.62 MiB.
- `git diff --check` — PASS до создания отчёта; повторяется перед commit.

Собранное приложение: `src-tauri/target/debug/bundle/macos/Контур.app`.

Ручной осмотр demo workspace:

- открыты Dashboard, список и карточка ДЗ, AI Workspace, Result, Iterations, Comments, Package, Settings;
- подтверждены real desktop backend, режимы Codex/Claude, diagnosis control, light/dark и reduced motion (затем исходные значения восстановлены);
- min window `1100×700` подтверждён конфигурацией (`src-tauri/tauri.conf.json:19-20`), но отдельное пиксельное resize-тестирование не проводилось;
- onboarding не перезапускался, чтобы не сбрасывать существующее demo-состояние; создание demo workspace доказано TempDir-тестом;
- видимых runtime/terminal ошибок в приложении не было; отдельный DevTools console capture не выполнялся;
- реальная сдача, AI request, open Finder/Obsidian side effects и diagnostic export не запускались.

## 11. Go / no-go

**NO-GO для настоящих домашних работ** до устранения P0-1 и P1-1/P1-2/P1-3. Допустим **GO с ограничениями для локального тестирования на копиях**: импорт, раскладка, extraction, ручной AI handoff, append-only версии и просмотр immutable snapshot. Пользователь обязан игнорировать зелёную readiness как окончательное доказательство и вручную сверять требования, Excel, стиль, приложения, ссылки и package непосредственно перед отправкой.

## 12. План исправлений маленькими этапами

1. **Authority model:** actor role, source authority, official confirmation, quote/context, conflict relation; migration + DTO + UI + tests.
2. **Dependency invalidation:** generation/revision graph для comments/requirements → reviews/prompts/final/package; запрет confirm stale package.
3. **Package v2:** versioned rebuild, explicit required files/links, единая manual semantics, final revalidation at confirm.
4. **Style profile:** explicit source selection, local traits/provenance, `null => manual blocker`, change invalidation.
5. **Version lineage:** parent, author/provider/import provenance, acceptance/rejection, SHA-based file and spreadsheet diff.
6. **Criterion evidence:** для каждого review criterion фиксировать verifier kind и evidence; manual/external gates не превращать в auto-pass.
7. **Excel report:** полный error scan, formulas/hidden sheets metadata, print/link/comment limitations и обязательный external/manual checklist.
8. **Prompt contract:** typed surgical correction plan с preserve/forbid/control values/name/no-overwrite/self-check.
9. **UI truthfulness:** единый provenance formatter; убрать auto labels без доказательства; stale banners.
10. **Mandatory E2E:** выполнить 20-шаговый TempDir-сценарий, затем повторить read-only manual demo smoke.

До выполнения этапов 1–3 нельзя разрешать использование readiness/package confirmation как основания для реальной сдачи.
