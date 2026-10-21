# Контур — frontend handoff

Документ фиксирует frontend-контракт и границу этапа 6. Источник истины по типам — `src/lib/bridge/types.ts` и `src/lib/bridge/workflow-types.ts`; mock остаётся исполняемым поведением UI по умолчанию. Этап 5 добавил в контракт управляемые каталоги, отчётный импорт во «Входящие», native drag state и действия Finder; этап 6 — watcher загрузок, кандидатов, предпросмотр и импорт экспорта CU LMS Enhancer, происхождение файлов и ссылки LMS.

## 1. Граница архитектуры

```text
React pages/components
  → useBridgeQuery / useBridgeEvent / useRulesDraft / useWorkflowActions
  → bridge: DesktopBridge
  → MockDesktopBridge (сейчас) | TauriDesktopBridge (явная отдельная фабрика)
  → Tauri commands/events
  → Rust: filesystem, watchers, provider adapters, backups
```

- React не обращается к файловой системе, shell, cookies или Tauri API напрямую.
- Единственная системная граница — типизированный `DesktopBridge` из `src/lib/bridge`.
- `createBridge()` по умолчанию создаёт `MockDesktopBridge`. Только флаг разработчика `VITE_KONTUR_BRIDGE=tauri` в нативном runtime создаёт `TauriDesktopBridge` целиком (для ручной проверки реального импорта); ни один npm-скрипт его не выставляет. Переключение UI по умолчанию — отдельный будущий этап.
- Hybrid bridge запрещён: реальный адаптер не смешивает SQLite-чтения с mock-записями, поскольку это создаёт несогласованное состояние и ложный успешный результат операций.
- Пути в моделях — относительные пути внутри выбранного хранилища. Абсолютный путь пользователя не хардкодится.
- Все даты — ISO 8601, размеры — байты, идентификаторы непрозрачны для frontend.

## 2. Семь основных экранов

Навигация содержит семь экранов; карточка ДЗ является детальным маршрутом раздела «Домашние задания».

| Маршрут | Экран | Основные данные и действия |
|---|---|---|
| `/` | Главная | `getDashboard`, `getHomeworkList`; девятиэтапная сводка, ближайшее действие и счётчики |
| `/inbox` | Входящие | intake, классификация, preview и раскладка без перезаписи |
| `/week`, `/subjects/:subjectId/week/:n` | Неделя | предмет, файлы недели, связанное ДЗ, создание недели |
| `/homework` | Домашние задания | доска по папочному `Homework.stage`; подробный workflow в карточках |
| `/ai` | AI Workspace | source bundle, запуск анализа/проверок, результаты, промты и история |
| `/automation` | Автоматизация | правила классификации/именования, подтверждения, журнал и undo |
| `/settings` | Настройки | пути, имя, тема, роли AI, порог стиля, статусы провайдеров |

Детальный маршрут `/homework/:homeworkId` содержит вкладки «Исходные материалы», «Рабочий цикл», «Результат проверки», «Комментарии преподавателя» и «Комплект на сдачу». Переход с Главной ведёт на вкладку, соответствующую ближайшему действию.

## 3. Девять этапов workflow

Порядок `WORKFLOW_STAGE_ORDER`:

1. `materials_collected` — Материалы.
2. `codex_analysis` — Анализ (имя типа историческое; фактический провайдер берётся из роли `analyst`).
3. `prompt_ready` — Промт.
4. `claude_solving` — Решение (фактический провайдер роли `author`).
5. `codex_review` — Проверка (фактический провайдер роли `reviewer`).
6. `needs_fixes` — Исправления.
7. `teacher_review` — Преподаватель.
8. `ready_to_submit` — К сдаче.
9. `submitted` — Отправлено.

Имена `codex_analysis` и `claude_solving` сохраняются ради совместимости типов, но UI и backend обязаны опираться на роли, а не на имя этапа.

### `Homework.stage` и `Homework.workflowStage`

- `stage: task | in_progress | ready | submitted` — папочное состояние и колонки общего списка: нет версии; есть рабочая версия; есть итоговый файл в `03 На сдачу`; существует подтверждённый снимок в `04 Отправлено`.
- `workflowStage` — текущий логический шаг девятиэтапного процесса, навигация карточки и доступность AI/сдачи.
- Однозначное правило: `stage` меняется только вследствие файлового факта, `workflowStage` — вследствие workflow-события. Нельзя выводить один автоматически только из другого. Для отправленной работы оба значения обязаны быть `submitted`.
- `ready_to_submit` требует readiness и итоговой версии, поэтому к этому моменту `stage` уже `ready`. Само назначение итоговой версии не означает `ready_to_submit`.

## 4. Актуальные workflow types

- `WorkflowSettings`: `roles: { analyst, author, reviewer }`, `styleThreshold` (целое 0–10, по умолчанию 4), `styleProfilePath`, `requireTeacherReview`.
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

### ДЗ и версии

`getHomeworkList`, `getHomework`, `getHomeworkCandidates`, `createHomeworkVersion`, `markVersionFinal`.

### Источники, AI и итерации

`getStyleProfile`, `prepareSourceBundle`, `setSourceIncluded`, `runMaterialAnalysis`, `runVersionReview`, `runRevisionPrompt`, `createRevisionPrompt`, `sendPromptToAuthor`, `updatePromptText`, `getPrompts`, `cancelRun`, `getReviewRuns`, `saveRunAsIteration`, `getIterations`, `compareVersions`.

### Преподаватель

`getTeacherComments`, `saveTeacherComments`, `importTeacherTranscript`, `removeTeacherComment`, `runTeacherReview`, `getTeacherRequirements`, `resolveRequirement`.

### Готовность и сдача

`getReadiness`, `markReadyToSubmit`, `buildSubmissionPackage`, `getSubmissionPackage`, `setPackageCheck`, `confirmSubmission`, `getSubmissionReceipt`.

### Система, провайдеры, правила и журнал

`revealInFinder`, `openProviderApp`, `connectProvider`, `disconnectProvider`, `getProviderStatus`, `getAutomationRules`, `updateAutomationRules`, `getActivityLog`, `undoOperation`, `on`.

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
| `inbox:updated` | `{ reason: "import", fileIds }` — список «Входящих» изменился, перечитать `getInboxFiles` |
| `downloads:watcher-updated` | `{ watcher: DownloadsWatcher }` — состояние watcher и список кандидатов |
| `cu-import:progress` | `{ operationId, phase: inspect \| extract \| import \| cleanup, completed, total, entryName }` — фазы импорта экспорта ЦУ |

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

`update_automation_rules` и `update_classification` также зарегистрированы. Первая возвращает контрактный `OperationResult<AutomationRules>`, вторая — актуальный `InboxFile`, как определено методами `DesktopBridge`. `TauriDesktopBridge` подключает соответствующие `updateAutomationRules` и `updateClassification` напрямую к IPC. Только `getWeek` и `getHomework` переводят backend `not_found` в контрактный `null`; остальные ошибки пробрасываются. Все прочие методы интерфейса явно отклоняются с `backend_unavailable` и не используют mock.

`ClassificationPatch` сохраняет различие между отсутствием и очисткой: TypeScript `undefined` кодируется как `{ state: "absent" }`, `null` как `{ state: "null" }`, значение как `{ state: "value", value }`. Для category явная очистка означает `unknown`. Команда не перемещает файл и не обращается к файловой системе.

Команды этапа 5: `check_managed_roots`, `choose_managed_root { kind }`, `prepare_inbox_root`, `pick_import_files`, `import_inbox_files { operationId, items: [{ sourcePath, origin }] }`, `get_operation_result { operationId }`, `open_inbox_file { fileId }`, `reveal_inbox_file { fileId }`, `reveal_in_finder { relativePath }`. `TauriDesktopBridge` подключает к ним `getManagedRoots`, `chooseManagedRoot`, `prepareInboxRoot`, `pickFiles`, `importFiles`, `importDroppedFiles` (файлы из отчёта без ошибок), `getOperationResult` (`not_found` → `null`), `openInboxFile`, `revealInboxFile`, `revealInFinder`; `onNativeFileDrop` и `onNativeDragState` слушают `intake:native-drop`/`intake:drag-state`.

Команды этапа 6: `get_downloads_watcher`, `set_downloads_watcher { enabled }`, `dismiss_download_candidate { candidateId }`, `pick_cu_export`, `preview_cu_export { candidateId }`, `import_cu_export { operationId, request }`, `set_subject_course_url { subjectId, url }`, `set_homework_assignment_url { homeworkId, url }`, `open_course { subjectId, fileId }`, `open_assignment { homeworkId, fileId }` (в двух последних задаётся ровно одно поле, второе — `null`). `TauriDesktopBridge` подключает к ним одноимённые методы `DesktopBridge` напрямую; `importCuExport` без `operationId` генерирует UUID v4. Mock fallback нет.

Пока unsupported:

- dashboard/week writes: `getDashboard`, `createWeek`;
- классификация и раскладка: `classifyFiles`, `previewRoute`, `routeFiles`;
- версии: `getHomeworkCandidates`, `createHomeworkVersion`, `markVersionFinal`;
- AI/workflow: `getStyleProfile`, `prepareSourceBundle`, `setSourceIncluded`, `runMaterialAnalysis`, `runVersionReview`, `runRevisionPrompt`, `createRevisionPrompt`, `sendPromptToAuthor`, `updatePromptText`, `getPrompts`, `cancelRun`, `getReviewRuns`, `saveRunAsIteration`, `getIterations`, `compareVersions`;
- преподаватель: `getTeacherComments`, `saveTeacherComments`, `importTeacherTranscript`, `removeTeacherComment`, `runTeacherReview`, `getTeacherRequirements`, `resolveRequirement`;
- сдача: `getReadiness`, `markReadyToSubmit`, `buildSubmissionPackage`, `getSubmissionPackage`, `setPackageCheck`, `confirmSubmission`, `getSubmissionReceipt`;
- system/provider/write: `openProviderApp`, `connectProvider`, `disconnectProvider`, `getProviderStatus`, `undoOperation`.

UI по умолчанию продолжает использовать `MockDesktopBridge`; Tauri-фабрика выбирается только флагом `VITE_KONTUR_BRIDGE=tauri` при наличии Tauri runtime.

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
- Экран «Входящие» показывает `ImportPanel`: прогресс по `operation:progress` с тем же `operationId`, итог по каждому файлу, «В Finder». «Симулировать загрузку» — только mock. «Настройки» содержат «Управляемые каталоги» вместо текстовых полей путей. Классификация и раскладка в Tauri-режиме пока unsupported: новые файлы остаются со статусом `new` («Ждёт распознавания»).

## 8a. Загрузки и экспорт CU LMS Enhancer (этап 6)

- «Контур» не входит в LMS, не хранит логин, пароль, cookies или сессию и не отправляет работы. Читаются только файлы, скачанные пользователем браузером.
- `DownloadsWatcher`: `enabled` (по умолчанию `false`, меняется только `setDownloadsWatcher` из переключателя в Настройках), `state: disabled | no_folder | watching | error`, `message`, `lastScanAt`, `candidates: DownloadCandidate[]`. Включение без выбранной папки загрузок → `blocked`. Watcher ничего не импортирует сам.
- `DownloadCandidate`: `id` (непрозрачный), `name`, `sizeBytes`, `detectedAt`, `origin: watcher | manual`, `kind: zip | pdf | file`. Абсолютного пути нет. `pickCuExport()` — системный выбор ZIP/PDF, отмена `null`; `dismissDownloadCandidate` скрывает файл (watcher не предложит его снова).
- `previewCuExport(candidateId)` → `ExportPreview`: `archiveSha256` (отпечаток), `format: zip | single`, `courseTitle`, `entries: ExportEntryPreview[]` (`entryId`, путь внутри архива, имя, `inboxName`, размер, тип, `classification`, `status: classified | ambiguous`, `selectable`, `selected`, `warnings`), `ignored`, `warnings`, счётчики. Ничего не пишет на диск. Вредоносный архив отклоняется `blocked` с объяснением.
- `importCuExport(request, operationId?)`: `ExportImportRequest { candidateId, archiveSha256, entries: ExportImportEntry[], courseUrl?, assignmentUrl? }`; в `entries` — только выбранные записи, у каждой `subjectId`, `weekNumber`, `category` и `edited` (пользователь изменил классификацию → `source: manual`). Изменённый после preview файл → `conflict`. Результат — `OperationResult<InboxImportReport>` (поле `skipped`). Файлы попадают только во «Входящие» со статусом `classified`/`ambiguous`; раскладки нет.
- `InboxFile.provenance: InboxProvenance | null` — `source: manual | watcher | cu_export`, `via: picker | drop | watcher`, `archiveName`, `entryPath` (относительный путь в архиве), `courseUrl`, `assignmentUrl`. `InboxFile.origin` — `lms` для экспорта, `downloads` для обычной загрузки из watcher, `drop` для ручного импорта.
- `Subject.courseUrl` — необязательная ссылка; `Homework.assignmentUrl` — как раньше. `openCourse({ subjectId } | { fileId })`, `openAssignment({ homeworkId } | { fileId })` открывают сохранённую ссылку во внешнем браузере после повторной проверки allowlist (`https://my.centraluniversity.ru`). Frontend не открывает URL сам; `lmsUrlProblem` в `src/lib/lmsLinks.ts` — только подсказка для формы.
- UI: «Настройки» — переключатель watcher; «Автоматизация» — статус и кандидаты; «Входящие» — «Импортировать экспорт ЦУ», «Новые загрузки», окно предпросмотра (`CuExportDialog`), итог в `ImportPanel`, происхождение и кнопки ссылок в панели файла.

## 9. Инварианты workflow и сдачи

- Роли `analyst`, `author`, `reviewer` независимо назначаются Codex или Claude. ProviderCard получает роли из `ProviderStatus.roles`; сохранение правил эмитит `provider:status-changed`.
- Порог стиля 0–10 применяется к новым результатам проверки и к readiness. Сохранённый `ReviewRun` не переписывается при последующей смене порога.
- Кнопка не активна без полного source bundle, нужной версии, подключённого назначенного провайдера либо при уже активном запуске.
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
- адаптеры Codex/Claude, прогресс, отмена и формирование результатов;
- сборка комплекта, но не отправка в LMS.

## 11. Секреты и авторизация

- «Контур» не запрашивает и не хранит пароли, cookies, session tokens или API keys Codex/Claude.
- Нельзя автоматизировать браузерный вход, встраивать страницы авторизации или извлекать пользовательские cookies.
- Авторизация выполняется только в официальном CLI/desktop-приложении провайдера. В bridge возвращаются состояние, безопасный `accountLabel`, adapter и роли — никогда секрет.
- Настройки frontend хранят только правила workflow, пути, имя, тему и motion preference.

## 12. Что остаётся backend-этапу

Этап 6 закрыл watcher загрузок и импорт экспорта ЦУ во «Входящие». Дальше — автоматическая классификация обычных файлов и раскладка по предметам поверх готового импорта, применение backup/undo, открытие официальных приложений провайдеров, provider adapters, пакеты и immutable snapshot. UI по умолчанию всё ещё использует `MockDesktopBridge`; его переключение должно быть отдельным принятым этапом.
