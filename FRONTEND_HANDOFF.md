# Контур — frontend handoff

## Этап 10 — native-файловый цикл

В `TauriDesktopBridge` подключены реальные `createWeek`, `classifyFiles`, `previewRoute`, `routeFiles`, `undoOperation`, `getStyleProfile`, `importTeacherTranscript`, `removeTeacherComment`, `buildSubmissionPackage`, `setPackageCheck` и `confirmSubmission`. `RoutePreview` содержит полный план (`sourceRelativePath`, `createsDirectories`, `sha256Match`, `nameConflict`, `renamed`, `action`, `undoable`). UI обязан показывать план до `routeFiles`; `ambiguous` нельзя раскладывать автоматически. Импорт файла переписки использует native `pickFiles()`, поэтому абсолютный путь не сохраняется в React или SQLite.

С этапа 11 настоящий Tauri runtime всегда использует `TauriDesktopBridge`; mock доступен только тестам и явному `VITE_KONTUR_BRIDGE=demo` вне Tauri. Ошибка IPC показывается пользователю и не подменяется demo-результатом.

## Этап 11 — runtime и первый запуск

Все методы, вызываемые UI, имеют обязательный IPC-клиент; временные stage-10 fallback удалены. Первый запуск предлагает создать новый real workspace, повторно подключить известный vault или создать demo. Preview структуры остаётся локальным состоянием UI, а запись начинается только через `prepareInboxRoot`. Demo workspace создаётся только в app-data и проверяется TempDir-тестом.

Production workflow, этап 1: `ManagedRoots.workspace` показывает активный workspace (`id`, `kind`, fingerprint, created/state). `getWorkspaces()` возвращает активное, все зарегистрированные пространства и журнал переходов. Любой переход — `previewWorkspaceTransition(request)` → `confirmWorkspaceTransition(id)` или `cancelWorkspaceTransition(id)`; `useWorkspaceTransition` показывает эффекты/предупреждения и не позволяет применить без подтверждения. После confirm backend перезапускает приложение (`restartRequired`), поэтому UI не делает `location.reload()`. Кнопки смены vault у активного workspace нет; backup/downloads остаются обычными управляемыми корнями.

AI UI показывает один из режимов: «подписка аккаунта», «ручная передача», «не авторизован». API-key auth не поддерживается: в изоляцию Codex монтируется только официальный `auth.json` с `auth_mode=chatgpt`, без `OPENAI_API_KEY`; Claude production остаётся manual-only до доказанной изоляции settings/hooks/MCP.

## Этап 13 — диагностика и release candidate

`DesktopBridge.exportDiagnosticReport()` вызывает native save dialog. Backend записывает JSON только с version/schema, состояниями корней, счётчиками и обезличенными статусами. Имена, пути, тексты, prompts/responses и credentials не экспортируются.

## Этап 9.2 — isolation UI

`ProviderStatus.isolation` содержит capability, последний `IsolationReport`, версию consent, `executionPolicy` и причину блокировки. `DesktopBridge.runAiIsolationSelfTest()` запускает native self-test. В «Настройки → Нейросети» показаны ручной/изолированный режим, версия профиля, дата проверки, кнопки проверки, явного включения и отключения. Старый `cliEnabled=true` без версии профиля не активирует CLI. Claude production всегда manual-only; frontend показывает backend reason и не подменяет его mock fallback.

Пользовательская формулировка: «Изолированный режим ограничивает файловый доступ внешнего CLI. Сеть и обработка переданного текста остаются на стороне провайдера».

Документ фиксирует frontend-контракт. Источник истины по типам — `src/lib/bridge/types.ts` и `src/lib/bridge/workflow-types.ts`; production использует реальный Tauri backend.

## 1. Граница архитектуры

```text
React pages/components
  → useBridgeQuery / useBridgeEvent / useRulesDraft / useWorkflowActions
  → bridge: DesktopBridge
  → TauriDesktopBridge (production) | MockDesktopBridge (тесты / browser demo)
  → Tauri commands/events
  → Rust: filesystem, watchers, provider adapters, backups
```

- React не обращается к файловой системе, shell, cookies или Tauri API напрямую.
- Единственная системная граница — типизированный `DesktopBridge` из `src/lib/bridge`.
- `createBridge()` в native runtime всегда создаёт `TauriDesktopBridge`. В браузере mock разрешён только явным `VITE_KONTUR_BRIDGE=demo`; без него bridge честно сообщает `backend_unavailable`.
- Hybrid bridge запрещён: реальный адаптер не смешивает SQLite-чтения с mock-записями, поскольку это создаёт несогласованное состояние и ложный успешный результат операций.
- Пути в моделях — относительные пути внутри выбранного хранилища. Абсолютный путь пользователя не хардкодится.
- Все даты — ISO 8601, размеры — байты, идентификаторы непрозрачны для frontend.

## 2. Восемь основных экранов

Навигация содержит восемь экранов (этап 7 добавил «Материалы»); карточка ДЗ является детальным маршрутом раздела «Домашние задания».

| Маршрут | Экран | Основные данные и действия |
|---|---|---|
| `/` | Главная | `getDashboard`, `getHomeworkList`; девятиэтапная сводка, ближайшее действие и счётчики |
| `/inbox` | Входящие | intake, классификация, preview и раскладка без перезаписи; «Текст и структура» файла |
| `/materials` | Материалы | `listMaterialExtractions`, `analyzeMaterials`/`cancelExtraction`, просмотр извлечённого содержания, подборка для анализа (⌘7) |
| `/week`, `/subjects/:subjectId/week/:n` | Неделя | предмет, файлы недели, связанное ДЗ, создание недели |
| `/homework` | Домашние задания | доска по папочному `Homework.stage`; подробный workflow в карточках |
| `/ai` | AI Workspace | source bundle, запуск анализа/проверок, результаты, промты и история |
| `/automation` | Автоматизация | правила классификации/именования, подтверждения, журнал и undo |
| `/settings` | Настройки | пути, имя, тема, провайдер каждой AI-задачи, порог стиля, состояние и подключение официальных CLI |

Детальный маршрут `/homework/:homeworkId` содержит вкладки «Исходные материалы», «Рабочий цикл», «Результат проверки», «Комментарии преподавателя» и «Комплект на сдачу». Переход с Главной ведёт на вкладку, соответствующую ближайшему действию.

## 3. Девять этапов workflow

Порядок `WORKFLOW_STAGE_ORDER`:

1. `materials_collected` — Материалы.
2. `codex_analysis` — Анализ (имя типа историческое; фактический провайдер берётся из задачи `material_analysis`).
3. `prompt_ready` — Промт.
4. `claude_solving` — Решение (провайдер роли `author`; решение создаётся вне «Контура»).
5. `codex_review` — Проверка (фактический провайдер задачи `solution_review`).
6. `needs_fixes` — Исправления.
7. `teacher_review` — Преподаватель.
8. `ready_to_submit` — К сдаче.
9. `submitted` — Отправлено.

Имена `codex_analysis` и `claude_solving` сохраняются ради совместимости типов, но UI и backend опираются на задачи (`AiTask`) и роли, а не на имя этапа.

### `Homework.stage` и `Homework.workflowStage`

- `stage: task | in_progress | ready | submitted` — папочное состояние и колонки общего списка: нет версии; есть рабочая версия; есть итоговый файл в `03 На сдачу`; существует подтверждённый снимок в `04 Отправлено`.
- `workflowStage` — текущий логический шаг девятиэтапного процесса, навигация карточки и доступность AI/сдачи.
- Однозначное правило: `stage` меняется только вследствие файлового факта, `workflowStage` — вследствие workflow-события. Нельзя выводить один автоматически только из другого. Для отправленной работы оба значения обязаны быть `submitted`.
- `ready_to_submit` требует readiness и итоговой версии, поэтому к этому моменту `stage` уже `ready`. Само назначение итоговой версии не означает `ready_to_submit`.

## 4. Актуальные workflow types

- `WorkflowSettings`: `roles: { analyst, author, reviewer }`, `tasks: { material_analysis, initial_prompt, solution_review, revision_prompt, teacher_review }` (этап 9: провайдер каждой задачи; у настроек до этапа 9 backend выводит задачи из ролей), `styleThreshold` (целое 0–10, по умолчанию 4), `styleProfilePath`, `requireTeacherReview`.
- `SourceBundle`: подборка `SourceItem[]`, `missing[]`, `complete`, purpose и optional version. Обязательный исключённый источник делает bundle неполным.
- `ReviewRun`: kind, role, фактический provider, version, bundle, полный `sourcesUsed`, очередь/прогресс/результат/ошибка. Новый запуск берёт роли и style threshold из актуальных настроек.
- `ReviewResult`: verdict, критерии, critical/normal findings, unsupported claims, style divergence с порогом, teacher requirements, recommended action и optional prompt.
- `RevisionPrompt`: initial/revision/teacher, текст, автор, версия и run-основание, закрываемые finding/requirement ids, evidence и `sentAt`. После передачи автору текст не редактируется.
- `TeacherComment`: отдельные вопрос и ответ, источник, дата, optional attachment; `TeacherRequirement` связан через `commentId`, хранит origin, статус и проверенную версию.
- `SubmissionPackage`: версия, папка, файлы/приложения, ссылки, предупреждения, автоматические и ручные checklist items, упорядоченный `uploadList`.
- `SubmissionReceipt`: ручное подтверждение, package/version ids, дата, URL задания, optional proof, файлы снимка, `snapshotPath` и `snapshotHash`.
- `HomeworkVersion` всегда `frozen: true`; старые версии не изменяются и не удаляются.

## 5. Структура хранилища и маршрутизация

```text
Предметы/
  00 Входящие/
  <Предмет>/
    Неделя XX/
      01 Лекции/
      02 Семинары/
      03 ДЗ/
        01 Условие и данные/
        02 Работа/
        03 На сдачу/
        04 Отправлено/
      04 Дополнительно/
```

- `lecture` → `Week.lectures` / `01 Лекции`;
- `seminar` → `Week.seminars` / `02 Семинары`;
- `material` → `Week.materials` / `04 Дополнительно`;
- `homework_task`, `homework_data` → `Homework.taskFiles` / `03 ДЗ/01 Условие и данные`;
- `homework_draft` → кандидат версии в `03 ДЗ/02 Работа`;
- если ДЗ отсутствует, создаются согласованные `Homework` и `Week.homeworkId`;
- дубликаты ищутся во всех массивах недели, task files, кандидатах и версиях; существующий файл никогда не перезаписывается.

## 6. DesktopBridge: полный список методов

### Чтение структуры и Главной

`getDashboard`, `getSubjects`, `getWeeks`, `getWeek`, `createWeek`.

### Intake и раскладка

`getInboxFiles`, `pickFiles`, `onNativeFileDrop`, `onNativeDragState`, `importDroppedFiles`, `importFiles`, `getOperationResult`, `openInboxFile`, `revealInboxFile`, `classifyFiles`, `updateClassification`, `previewRoute`, `routeFiles`.

### Управляемые каталоги

`getManagedRoots`, `chooseManagedRoot`, `prepareInboxRoot`.

### Загрузки, экспорт ЦУ и ссылки LMS (этап 6)

`getDownloadsWatcher`, `setDownloadsWatcher`, `dismissDownloadCandidate`, `pickCuExport`, `previewCuExport`, `importCuExport`, `setSubjectCourseUrl`, `setHomeworkAssignmentUrl`, `openCourse`, `openAssignment`.

### Извлечение текста и подборка (этап 7)

`listMaterialExtractions`, `getMaterialExtraction`, `analyzeMaterial`, `analyzeMaterials`, `cancelExtraction`, `clearExtractionCache`, `getExtractionSettings`, `setExtractionSettings`, `getCorpusSelection`, `setCorpusSelection`, `previewCorpus`.

### ДЗ и версии

`getHomeworkList`, `getHomework`, `getHomeworkCandidates`, `createHomeworkVersion`, `markVersionFinal`, `getWorkflow` (весь агрегат рабочего цикла одним вызовом; `not_found` → `null`), `transitionWorkflow`.

### Источники, AI и итерации

`getStyleProfile`, `prepareSourceBundle`, `setSourceIncluded`, `runMaterialAnalysis`, `runVersionReview`, `runRevisionPrompt`, `createRevisionPrompt`, `sendPromptToAuthor`, `updatePromptText`, `commitPrompt`, `getPrompts`, `cancelRun`, `getReviewRuns`, `saveRunAsIteration`, `saveManualAnalysis`, `saveManualReviewResult`, `confirmStyleDivergence`, `getIterations`, `compareVersions`.

Мутации рабочего цикла принимают последним аргументом `WorkflowCommandOptions { operationId?, expectedRevision? }`. Без `operationId` Tauri bridge генерирует UUID v4; `expectedRevision` — ревизия агрегата, которую видит пользователь: устаревшая даёт `conflict`, после чего UI перечитывает `getWorkflow` и предлагает повторить действие (`isStaleConflict` в `src/lib/workflow-ui.ts`). Запуски принимают `mode: "provider" | "manual"`.

### Преподаватель

`getTeacherComments`, `saveTeacherComments`, `importTeacherTranscript`, `removeTeacherComment`, `runTeacherReview`, `getTeacherRequirements`, `addTeacherRequirement`, `resolveRequirement`.

### Готовность и сдача

`getReadiness`, `markReadyToSubmit`, `buildSubmissionPackage`, `getSubmissionPackage`, `setPackageCheck`, `confirmSubmission`, `getSubmissionReceipt`.

### Провайдеры и задания передачи (этап 9)

`getProviderStatuses`, `getProviderStatus`, `checkProvider`, `connectProvider`, `disconnectProvider`, `openProviderApp`, `listAiJobs`, `getAiJob`, `prepareAiJob`, `sendAiJob`, `cancelAiJob`, `submitAiResponse`, `importAiResponseFile`, `discardAiJob`, `confirmAiResult`.

### Система, правила и журнал

`revealInFinder`, `getAutomationRules`, `updateAutomationRules`, `getActivityLog`, `undoOperation`, `on`.

Сигнатуры и payload находятся в `DesktopBridge`. Источники, запуски и сдача обслуживаются только перечисленными актуальными методами; сдача подтверждается `confirmSubmission`.

## 7. События bridge

| Событие | Payload и назначение |
|---|---|
| `inbox:file-added` | `{ files }`, обновление входящих |
| `inbox:classification-updated` | `{ fileId, classification, status }` |
| `operation:progress` | operation id/kind, progress 0–1, message |
| `operation:completed` | завершённая `Operation`; состояние уже закоммичено |
| `operation:failed` | operation и типизированная ошибка |
| `provider:status-changed` | актуальный `ProviderStatus`, включая роли |
| `homework:version-created` | homework id и новая frozen version |
| `workflow:stage-changed` | homework id, from/to и причина |
| `review:progress` | run id, homework id, progress и step |
| `review:completed` | завершённый `ReviewRun` |
| `review:failed` | run и ошибка |
| `submission:confirmed` | неизменяемый `SubmissionReceipt` |
| `homework:updated` | `{ homeworkId, revision, stage, reason, operationId }` — агрегат ДЗ изменился, перечитать `getWorkflow` (этап 8) |
| `homework:revision-prompt-created` | `{ homeworkId, promptId, kind, status, reviewRunId, basedOnVersionId }` — новый черновик промта |
| `homework:readiness-updated` | `{ homeworkId, ready, blockers, revision }` — пересчитана готовность |
| `inbox:updated` | `{ reason: "import", fileIds }` — список «Входящих» изменился, перечитать `getInboxFiles` |
| `ai-job:progress` | `{ jobId, runId, homeworkId, phase: started \| running \| parsing, elapsedMs, outputBytes }` — работа официального CLI (без содержимого вывода, без процентов) (этап 9) |
| `ai-job:updated` | `{ jobId, runId, homeworkId, status, parseStatus }` — задание изменилось, перечитать `getAiJob`/`getWorkflow` (этап 9) |
| `downloads:watcher-updated` | `{ watcher: DownloadsWatcher }` — состояние watcher и список кандидатов |
| `cu-import:progress` | `{ operationId, phase: inspect \| extract \| import \| cleanup, completed, total, entryName }` — фазы импорта экспорта ЦУ |
| `extraction:progress` | `{ operationId, completed, total, material, name, outcome }` — прогресс пакетного извлечения текста |
| `extraction:updated` | `{ reason: analyzed \| cleared, materials }` — результаты извлечения изменились, перечитать `listMaterialExtractions` |

Событие «файл добавлен» в контракте называется `inbox:file-added` (`{ files }`) и эмитится только для впервые импортированных файлов. Служебные `intake:native-drop` и `intake:drag-state` слушает только Tauri adapter; в `BridgeEvents` они не входят.

Для любой undoable мутации обязателен порядок:

```text
start operation
→ progress events
→ commit всех связанных сущностей
→ регистрация undo
→ operation:completed
→ resolve Promise<OperationResult<T>>
```

После `operation:completed` слушатель обязан видеть новое состояние. После resolve не должно оставаться отложенных мутаций. Undo восстанавливает весь согласованный снимок сущностей, а не только один массив.

### Готовые Tauri IPC-команды

`backend_health`, `get_subjects`, `get_weeks`, `get_week`, `get_homework_list`, `get_homework`, `get_inbox_files`, `get_automation_rules`, `get_activity_log` зарегистрированы в Rust и доступны через типизированные функции одноимённого уровня в `tauri/ipc.ts`. Аргументы IPC передаются в camelCase, DTO возвращаются в точной форме frontend-контракта. Ошибки Rust `{ code, message }` преобразуются в `BridgeError`; отсутствие native runtime даёт `backend_unavailable`.

`update_automation_rules` и `update_classification` также зарегистрированы. Первая возвращает контрактный `OperationResult<AutomationRules>`, вторая — актуальный `InboxFile`, как определено методами `DesktopBridge`. Только контрактно nullable-геттеры переводят backend `not_found` в `null`; остальные ошибки пробрасываются. Все UI-методы подключены к настоящим IPC-командам и не используют mock fallback.

`ClassificationPatch` сохраняет различие между отсутствием и очисткой: TypeScript `undefined` кодируется как `{ state: "absent" }`, `null` как `{ state: "null" }`, значение как `{ state: "value", value }`. Для category явная очистка означает `unknown`. Команда не перемещает файл и не обращается к файловой системе.

Команды этапа 5: `check_managed_roots`, `choose_managed_root { kind }`, `prepare_inbox_root`, `pick_import_files`, `import_inbox_files { operationId, items: [{ sourcePath, origin }] }`, `get_operation_result { operationId }`, `open_inbox_file { fileId }`, `reveal_inbox_file { fileId }`, `reveal_in_finder { relativePath }`. `TauriDesktopBridge` подключает к ним `getManagedRoots`, `chooseManagedRoot`, `prepareInboxRoot`, `pickFiles`, `importFiles`, `importDroppedFiles` (файлы из отчёта без ошибок), `getOperationResult` (`not_found` → `null`), `openInboxFile`, `revealInboxFile`, `revealInFinder`; `onNativeFileDrop` и `onNativeDragState` слушают `intake:native-drop`/`intake:drag-state`.

Команды этапа 6: `get_downloads_watcher`, `set_downloads_watcher { enabled }`, `dismiss_download_candidate { candidateId }`, `pick_cu_export`, `preview_cu_export { candidateId }`, `import_cu_export { operationId, request }`, `set_subject_course_url { subjectId, url }`, `set_homework_assignment_url { homeworkId, url }`, `open_course { subjectId, fileId }`, `open_assignment { homeworkId, fileId }` (в двух последних задаётся ровно одно поле, второе — `null`). `TauriDesktopBridge` подключает к ним одноимённые методы `DesktopBridge` напрямую; `importCuExport` без `operationId` генерирует UUID v4. Mock fallback нет.

Команды этапа 7: `list_material_extractions`, `get_material_extraction { material }`, `analyze_material { material, force }`, `analyze_materials { operationId, materials, force }`, `cancel_extraction { operationId }`, `clear_extraction_cache`, `get_extraction_settings`, `set_extraction_settings { ocrEnabled }`, `get_corpus_selection { homeworkId }`, `set_corpus_selection { homeworkId, materials }`, `preview_corpus { homeworkId }`. `material` — ровно `{ kind: "inbox" | "file", id }` (лишние поля не передаются). `TauriDesktopBridge` подключает к ним одноимённые методы; `getMaterialExtraction` переводит `not_found` в `null`; `analyzeMaterials` без `operationId` генерирует UUID v4; `force` по умолчанию `false`. Mock fallback нет.

Команды этапа 8 (рабочий цикл ДЗ): чтение — `get_workflow { homeworkId }`, `get_dashboard`, `get_iterations { homeworkId }`, `compare_versions { fromVersionId, toVersionId }`, `get_homework_candidates { homeworkId }`; мутации (все с `operationId` и `expectedRevision`, результат `OperationResult<T>`) — `create_source_bundle { homeworkId, purpose, versionId }`, `start_workflow_run { request: { homeworkId, kind, versionId, bundleId, mode } }`, `cancel_workflow_run { runId }`, `pin_workflow_run { runId }`, `save_manual_analysis { runId, input: { summary, promptText } }`, `save_manual_review { runId, input }`, `create_revision_prompt { reviewRunId, text }`, `update_prompt_draft { promptId, text }`, `commit_prompt { promptId }`, `hand_off_prompt { promptId }`, `create_homework_version { homeworkId, input: { fileId, attachmentFileIds, author, promptId, note } }`, `select_homework_version { versionId }`, `add_teacher_comments { homeworkId, comments }`, `add_teacher_requirement { homeworkId, input }`, `resolve_teacher_requirement { requirementId, status, note }`, `confirm_style_divergence { reviewRunId }`, `mark_ready_to_submit { homeworkId, noTeacherComments }`, `transition_workflow { homeworkId, to, reason }`.

`TauriDesktopBridge` подключает к ним методы рабочего цикла напрямую, без гибридного bridge и без mock fallback. Списочные геттеры (`getReviewRuns`, `getPrompts`, `getTeacherComments`, `getTeacherRequirements`, `getReadiness`, `getSubmissionPackage`, `getSubmissionReceipt`) читают части одного `get_workflow`; `not_found` → `null` только в `getWorkflow`. `setSourceIncluded` отвечает `immutable`: набор источников — неизменяемый снимок, состав меняется в подборке этапа 7 и новым снимком.

Команды этапа 9: `get_provider_statuses`, `check_provider { provider }`, `connect_provider { provider }`, `disconnect_provider { provider }`, `open_provider_app { provider }`, `list_ai_jobs { homeworkId }`, `get_ai_job { jobId }`, `prepare_ai_job { runId, task, transport }`, `send_ai_job { jobId }`, `cancel_ai_job { jobId }`, `submit_ai_response { jobId, text }`, `import_ai_response_file { jobId }`, `discard_ai_job { jobId }`, `confirm_ai_result { operationId, jobId, supportingJobId, input, expectedRevision }`. `TauriDesktopBridge` подключает к ним одноимённые методы напрямую; `prepareAiJob` без `transport` передаёт `auto`; `confirmAiResult` без `operationId` генерирует UUID v4 и передаёт `expectedRevision`; `input` — `{ kind: "analysis" | "review" | "prompt", value }` с явно заполненными необязательными полями ручного ввода. `getProviderStatus` читает `get_provider_statuses`. Ошибки (`provider_unavailable`, `reauth_required`, `blocked`, `conflict`) пробрасываются, mock fallback нет.

Пока unsupported (`backend_unavailable`):

- недели и раскладка: `createWeek`, `classifyFiles`, `previewRoute`, `routeFiles`;
- профиль стиля: `getStyleProfile`;
- преподаватель: `importTeacherTranscript`, `removeTeacherComment` (комментарии добавляются `saveTeacherComments`, требования — `addTeacherRequirement`);
- сдача (этап 10): `buildSubmissionPackage`, `setPackageCheck`, `confirmSubmission`;
- `undoOperation`.

UI в Tauri по умолчанию использует `TauriDesktopBridge`; browser mock требует `VITE_KONTUR_BRIDGE=demo`.

### Tauri event adapter

`tauri/events.ts` лениво импортирует `@tauri-apps/api/event` и предоставляет типизированный `listenToBridgeEvent`. Синхронный `Unsubscribe` помечает подписку неактивной немедленно. Если настоящий `unlisten` приходит позже, он вызывается сразу; повторная отписка безопасна, а callback после неё игнорируется. Ошибка регистрации нормализуется в `BridgeError` и передаётся контролируемому error handler, поэтому rejected registration promise не становится unhandled rejection.

Runtime detection едина для IPC и events. В browser/mock окружении Tauri globals и модули не требуются. После commit `updateClassification` эмитит `inbox:classification-updated`, затем `operation:completed`; `updateAutomationRules` эмитит `operation:completed`. При записанной failed-операции эмитится `operation:failed`. Ошибка доставки не откатывает commit и возвращает контролируемую ошибку, после которой клиент должен перечитать состояние.

## 8. Native и browser/mock file intake

- Browser/mock использует HTML5 `FileList`; `DroppedItem` содержит `name`, `sizeBytes`, а `sourcePath` отсутствует. Mock `importFiles` реализует тот же контракт отчёта: дубль по имени и размеру (содержимого HTML5 File для SHA-256 нет), « (N)» при совпадении имени, повтор по `operationId`.
- Native Tauri drag-and-drop включён (`dragDropEnabled: true`). Rust регистрирует пути из `WindowEvent::DragDrop` как разрешённые и отправляет `intake:native-drop { items }` с `name`, `sizeBytes`, `sourcePath`; adapter добавляет `origin: "drop"`. `onNativeDragState(active)` подсвечивает зону.
- `pickFiles()` вызывает `pick_import_files`: системный диалог открывает Rust через официальный `tauri-plugin-dialog`. Отмена — `[]`; элементы получают `origin: "picker"`.
- `importFiles(items, operationId?)` — основной путь импорта. Клиент генерирует UUID v4 (`newOperationId()`), чтобы сопоставить `operation:progress` с вызовом и безопасно повторить команду: повтор с тем же id возвращает сохранённый итог (`replayed: true`). Для native intake `sourcePath` обязателен — `assertNativeSourcePaths` останавливает импорт с `io_error` до IPC.
- Backend принимает только пути, полученные им самим из picker или drop; произвольный путь из WebView отклоняется `blocked`.
- `InboxImportReport.items[]`: `imported` (`file`, `renamedFrom`, `recovered`), `duplicate` (`file`, `restored`), `failed` (`error`). Ошибка отдельного файла не прерывает остальные; если не удались все — операция `failed` и событие `operation:failed`. Ошибки уровня команды (не выбрано хранилище, неверный id, незавершённая операция с тем же id) отклоняют Promise структурированным `BridgeError`.
- Порядок событий импорта: `operation:progress` (0, затем после каждого файла) → `inbox:file-added` (если есть новые) → `inbox:updated` → `operation:completed`/`operation:failed` → resolve.
- `getManagedRoots()` возвращает `vault`, `inbox`, `backup`, `downloads` с состоянием `ok | not_configured | missing | unsafe` и `displayPath` только для показа (`~` вместо home). `chooseManagedRoot(kind)` открывает системный выбор папки, `cancelled: true` при отмене. «Входящие» всегда `Предметы/00 Входящие` внутри хранилища; `prepareInboxRoot()` создаёт папку заранее, иначе её создаёт первый импорт.
- `openInboxFile(fileId)` открывает файл приложением по умолчанию (исполняемые файлы и скрипты — `blocked`), `revealInboxFile(fileId)` и `revealInFinder(relativePath)` показывают элемент в Finder.
- Экран «Входящие» показывает `ImportPanel`: прогресс по `operation:progress` с тем же `operationId`, итог по каждому файлу, «В Finder». «Симулировать загрузку» — только mock. В Tauri доступны реальные классификация, обязательный preview и подтверждённая раскладка.

## 8a. Загрузки и экспорт CU LMS Enhancer (этап 6)

- «Контур» не входит в LMS, не хранит логин, пароль, cookies или сессию и не отправляет работы. Читаются только файлы, скачанные пользователем браузером.
- `DownloadsWatcher`: `enabled` (по умолчанию `false`, меняется только `setDownloadsWatcher` из переключателя в Настройках), `state: disabled | no_folder | watching | error`, `message`, `lastScanAt`, `candidates: DownloadCandidate[]`. Включение без выбранной папки загрузок → `blocked`. Watcher ничего не импортирует сам.
- `DownloadCandidate`: `id` (непрозрачный), `name`, `sizeBytes`, `detectedAt`, `origin: watcher | manual`, `kind: zip | pdf | file`. Абсолютного пути нет. `pickCuExport()` — системный выбор ZIP/PDF, отмена `null`; `dismissDownloadCandidate` скрывает файл (watcher не предложит его снова).
- `previewCuExport(candidateId)` → `ExportPreview`: `archiveSha256` (отпечаток), `format: zip | single`, `courseTitle`, `entries: ExportEntryPreview[]` (`entryId`, путь внутри архива, имя, `inboxName`, размер, тип, `classification`, `status: classified | ambiguous`, `selectable`, `selected`, `warnings`), `ignored`, `warnings`, счётчики. Ничего не пишет на диск. Вредоносный архив отклоняется `blocked` с объяснением.
- `importCuExport(request, operationId?)`: `ExportImportRequest { candidateId, archiveSha256, entries: ExportImportEntry[], courseUrl?, assignmentUrl? }`; в `entries` — только выбранные записи, у каждой `subjectId`, `weekNumber`, `category` и `edited` (пользователь изменил классификацию → `source: manual`). Изменённый после preview файл → `conflict`. Результат — `OperationResult<InboxImportReport>` (поле `skipped`). Файлы попадают только во «Входящие» со статусом `classified`/`ambiguous`; раскладки нет.
- `InboxFile.provenance: InboxProvenance | null` — `source: manual | watcher | cu_export`, `via: picker | drop | watcher`, `archiveName`, `entryPath` (относительный путь в архиве), `courseUrl`, `assignmentUrl`. `InboxFile.origin` — `lms` для экспорта, `downloads` для обычной загрузки из watcher, `drop` для ручного импорта.
- `Subject.courseUrl` — необязательная ссылка; `Homework.assignmentUrl` — как раньше. `openCourse({ subjectId } | { fileId })`, `openAssignment({ homeworkId } | { fileId })` открывают сохранённую ссылку во внешнем браузере после повторной проверки allowlist (`https://my.centraluniversity.ru`). Frontend не открывает URL сам; `lmsUrlProblem` в `src/lib/lmsLinks.ts` — только подсказка для формы.
- UI: «Настройки» — переключатель watcher; «Автоматизация» — статус и кандидаты; «Входящие» — «Импортировать экспорт ЦУ», «Новые загрузки», окно предпросмотра (`CuExportDialog`), итог в `ImportPanel`, происхождение и кнопки ссылок в панели файла.

## 8b. Извлечение текста и подборка материалов (этап 7)

- Всё локально: Codex/Claude и сеть не вызываются. Оригинал только читается; результат привязан к SHA-256 содержимого и версии правил (`extractorVersion`).
- `MaterialRef { kind: "inbox" | "file", id }` — файл «Входящих» или файл недели/ДЗ. `MaterialExtractionItem` — материал + `extraction: ExtractionSummary | null` (статус, формат, `textChars`, `warningCount`, `stale` — файл изменился после анализа).
- `ExtractionStatus`: `complete`, `partial` (часть пропущена — см. `warnings`), `needs_ocr` (скан/изображение без текста, metadata сохранены), `unsupported`, `encrypted`, `corrupt`, `rejected` (безопасность: DTD, zip-бомба, обход пути), `too_large`, `timeout`, `failed` (последние два не кэшируются). Неподдерживаемый или повреждённый файл — это результат со статусом, а не ошибка Promise.
- `MaterialExtraction`: `blocks: ExtractedBlock[]` (`heading`, `paragraph`, `list_item`, `table`, `page`, `slide`, `sheet` с ячейками и текстами формул, `code`), нормализованный `text`, `metadata`, `warnings[] { code, message, location }`, `cached`, `ocrUsed`, `durationMs`.
- `analyzeMaterial(material, { force })` — тот же SHA-256 берётся из cache (`cached: true`), `force` перестраивает. `analyzeMaterials(materials, { force, operationId })` → `OperationResult<ExtractionBatchReport>`; прогресс — `extraction:progress` и `operation:progress` (kind `extract`), отмена — `cancelExtraction(operationId)`: уже обработанные файлы сохраняются, `stoppedByUser: true`, операция `failed` с кодом `cancelled`. Повтор с тем же `operationId` — `replayed: true` без событий.
- `ExtractionSettings`: `ocrEnabled` (по умолчанию `false`), `ocr: { available, engine, languages, message }`, `limits`. OCR — только локальный Tesseract, если установлен; без него изображения получают `needs_ocr`.
- `CorpusSelection` — явный выбор пользователя (общий или для ДЗ): элементы с `ready`/`problem`. `setCorpusSelection` принимает только материалы с извлечённым текстом (`complete`/`partial`) и неизменённым содержимым, иначе `blocked`; подборку отправленной работы — `immutable`. `previewCorpus` — ровно тот текст, который получит будущий анализ (лимит `limitChars`, список `excluded`). Построение `SourceBundle` и запуск анализа — будущий этап.
- UI: «Материалы» (`MaterialsPage`, `ExtractionViewer`, `CorpusPanel`), кнопка «Текст и структура» в панели файла «Входящих», «Настройки → Извлечение текста» (`ExtractionSettingsPanel`).

## 8c. Провайдеры и задания передачи (этап 9)

- Учётные данные провайдеров не запрашиваются, не читаются и не хранятся; вход — только в официальном CLI (`codex login`, `claude auth login`) или приложении. GUI провайдеров, браузера и LMS не автоматизируется. Официальной интеграции нет — используются публичные неинтерактивные режимы официальных CLI.
- `ProviderStatus` (runtime-проекция backend): `availability: connected | manual | unavailable | error`, `cli { enabled, detected, executable, version, supported, missingFlags, loggedIn, checkedAt }` (без абсолютных путей), `handoff { appInstalled, webUrl }`, `tasks: AiTask[]`, а также `state`, `adapter` (`cli | manual_handoff | unavailable | mock | desktop_app`), `roles`, `accountLabel` (метка клиента, не аккаунт). Найденный CLI не подключается сам — только `connectProvider` по кнопке «Подключить CLI».
- `AiJob`: `task`, `provider`, `transport: cli | manual_handoff`, `status: prepared | running | response_ready | failed | cancelled | confirmed | discarded`, полный `promptText` и состав `context[] { label, name, kind, charsTotal, charsIncluded, truncated }` (в `getWorkflow.aiJobs` и `listAiJobs` тексты — `null`), `responseText`, `responseSource: cli | paste | file`, `responseFileName` (только имя), `parseStatus: valid | partial | corrupt | empty`, `draft` (поля ручного ввода, любые могут отсутствовать), `issues`, `log` (без секретов и путей), `errorCode/errorMessage`, `exitCode`, `durationMs`, `edited`, `promptId`.
- Поток: шаг стартует как раньше (`run*` с `mode: "provider"` → `waiting_for_provider`) → `prepareAiJob(runId, task)` (ничего не отправляет; UI показывает весь текст и список файлов) → CLI: `sendAiJob` (только по кнопке) → `ai-job:progress` … `ai-job:updated` → `response_ready`; ручная передача: копирование по кнопке, `openProviderApp` по кнопке, `submitAiResponse`/`importAiResponseFile` → `response_ready` → пользователь проверяет и правит черновик в форме → `confirmAiResult`. Без подтверждения рабочий цикл не меняется; provenance результата — `provider`, ручной ввод (`saveManual*`) остаётся `manual`.
- Анализ материалов и первичный промт — разные задачи: если им назначены разные провайдеры, анализ просит только итог, а промт готовится отдельным заданием `initial_prompt` по черновику анализа; подтверждаются вместе (`supportingJobId`). Промт исправлений можно получить заданием `revision_prompt` по завершённой проверке (`{ kind: "prompt" }`) или собрать локально `createRevisionPrompt`.
- `cancelAiJob` останавливает выполняющийся CLI; `cancelRun` останавливает задания шага. `discardAiJob` отклоняет ответ.
- UI: `AiStepActions` (кнопки «Передать …» и «Ввести результат вручную»), `AiHandoffDialog`, `ManualResultDialog` с `initial`/`confirming`, `RunMonitor` с `job` и `onCancelJob`, `ProviderCard` с подключением CLI; копирование — `lib/clipboard.ts` (Clipboard API с запасным выделением текста).
- Mock реализует тот же контракт: демо-«CLI» отвечает только после `sendAiJob`, ответ ждёт подтверждения; `importAiResponseFile` в браузере — `backend_unavailable`.

## 9. Инварианты workflow и сдачи

- Пять AI-задач и роль `author` независимо назначаются Codex или Claude. ProviderCard получает задачи из `ProviderStatus.tasks`; сохранение правил эмитит `provider:status-changed` (mock).
- Порог стиля 0–10 применяется к новым результатам проверки и к readiness. Сохранённый `ReviewRun` не переписывается при последующей смене порога.
- Кнопка не активна без полного source bundle, нужной версии либо при уже активном запуске; без подключённого CLI шаг передаётся провайдеру вручную (`manual_handoff`) или результат вводится сам. Переходы между этапами предлагаются только из `HomeworkWorkflow.transitions` (решение backend state machine), «Готово к сдаче» — только при пройденной readiness, «Отправлено» — только подтверждением отправки.
- Промт: черновик правится (`updatePromptText`), затем фиксируется (`commitPrompt`) и после этого неизменяем; передать автору можно только зафиксированный промт.
- Расхождение со стилем выше порога блокирует готовность до явного подтверждения пользователем (`confirmStyleDivergence`); это эвристика сравнения со стилем 0–10, не определение авторства.
- `markReadyToSubmit` возможен только при всех readiness checks; отсутствие комментариев при обязательной teacher review требует явного подтверждения.
- `confirmSubmission` ничего не отправляет в LMS. Пользователь сначала отправляет файлы вручную, затем ставит `userConfirmed: true` и при необходимости прикладывает proof.
- После подтверждения создаётся `SubmissionReceipt`; submitted homework, versions, package и snapshot нельзя изменить или откатить.
- Создание версии всегда добавляет новую frozen entity. Старые версии и их файлы не перезаписываются.

## 10. Зоны ответственности

### Frontend

- отображение состояния, вкладок, прогресса и ошибок;
- локальный черновик настроек с Save/Reset;
- доступная навигация и подтверждения пользователя;
- HTML5 drop в browser/mock;
- вызовы только через `DesktopBridge`.

### Tauri adapter

- отображение каждого метода на command и каждого события на listener;
- нормализация ошибок в `BridgeError`;
- native file picker/drop с обязательным `sourcePath`;
- `revealInFinder` и открытие официального клиента провайдера;
- никаких бизнес-решений и файловых мутаций в React.

### Rust backend

- проверка всех инвариантов независимо от UI;
- безопасные файловые операции, резервирование, атомарный commit и полный undo;
- наблюдение входящих, классификация, создание структуры и дедупликация;
- хранение метаданных, хэширование версий и submitted snapshot;
- адаптеры официальных CLI Codex/Claude, прогресс, отмена, разбор ответа и применение только подтверждённого результата;
- сборка комплекта, но не отправка в LMS.

## 11. Секреты и авторизация

- «Контур» не запрашивает и не хранит пароли, cookies, session tokens или API keys Codex/Claude.
- Нельзя автоматизировать браузерный вход, встраивать страницы авторизации или извлекать пользовательские cookies.
- Авторизация выполняется только в официальном CLI/desktop-приложении провайдера. В bridge возвращаются состояние, безопасный `accountLabel`, adapter, задачи и роли — никогда секрет. Вход проверяется только по коду выхода `codex login status`/`claude auth status`; их вывод не читается.
- CLI запускается без shell, с очищенным окружением (переменные с ключами не передаются), в пустом каталоге; промт — через stdin, не в командной строке.
- Настройки frontend хранят только правила workflow, пути, имя, тему и motion preference.

## 12. Что остаётся backend-этапу

Этапы 10–11 добавили реальную классификацию/раскладку/undo, пакеты, immutable snapshot, production Tauri bridge и мастер первого запуска. Mock не является production-default.
