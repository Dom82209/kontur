# Контур — frontend handoff

Документ фиксирует frontend-контракт и границу этапа 4C.2A. Источник истины по типам — `src/lib/bridge/types.ts` и `src/lib/bridge/workflow-types.ts`; mock остаётся исполняемым поведением UI. Rust read-only backend и `TauriDesktopBridge` для готовых чтений реализованы, но адаптер ещё не подключён к UI.

## 1. Граница архитектуры

```text
React pages/components
  → useBridgeQuery / useBridgeEvent / useRulesDraft / useWorkflowActions
  → bridge: DesktopBridge
  → MockDesktopBridge (сейчас) | TauriDesktopBridge (следующий этап)
  → Tauri commands/events
  → Rust: filesystem, watchers, provider adapters, backups
```

- React не обращается к файловой системе, shell, cookies или Tauri API напрямую.
- Единственная системная граница — типизированный `DesktopBridge` из `src/lib/bridge`.
- `createBridge()` сейчас всегда создаёт `MockDesktopBridge`. Явная фабрика `createTauriBridge()` существует отдельно и будет подключена только после согласованного набора write-команд.
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

`getInboxFiles`, `pickFiles`, `onNativeFileDrop`, `importDroppedFiles`, `classifyFiles`, `updateClassification`, `previewRoute`, `routeFiles`.

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

### Готовые read-only Tauri IPC-команды

`backend_health`, `get_subjects`, `get_weeks`, `get_week`, `get_homework_list`, `get_homework`, `get_inbox_files`, `get_automation_rules`, `get_activity_log` зарегистрированы в Rust и доступны через типизированные функции одноимённого уровня в `tauri/ipc.ts`. Аргументы IPC передаются в camelCase, DTO возвращаются в точной форме frontend-контракта. Ошибки Rust `{ code, message }` преобразуются в `BridgeError`; отсутствие native runtime даёт `backend_unavailable`.

`TauriDesktopBridge` подключает к этим командам `getSubjects`, `getWeeks`, `getWeek`, `getInboxFiles`, `getHomeworkList`, `getHomework`, `getAutomationRules`, `getActivityLog`. Только `getWeek` и `getHomework` переводят backend `not_found` в контрактный `null`; остальные ошибки пробрасываются. Все прочие методы интерфейса явно отклоняются с `backend_unavailable` и не используют mock.

Пока unsupported:

- dashboard/week writes: `getDashboard`, `createWeek`;
- intake/routing: `pickFiles`, `onNativeFileDrop`, `importDroppedFiles`, `classifyFiles`, `updateClassification`, `previewRoute`, `routeFiles`;
- версии: `getHomeworkCandidates`, `createHomeworkVersion`, `markVersionFinal`;
- AI/workflow: `getStyleProfile`, `prepareSourceBundle`, `setSourceIncluded`, `runMaterialAnalysis`, `runVersionReview`, `runRevisionPrompt`, `createRevisionPrompt`, `sendPromptToAuthor`, `updatePromptText`, `getPrompts`, `cancelRun`, `getReviewRuns`, `saveRunAsIteration`, `getIterations`, `compareVersions`;
- преподаватель: `getTeacherComments`, `saveTeacherComments`, `importTeacherTranscript`, `removeTeacherComment`, `runTeacherReview`, `getTeacherRequirements`, `resolveRequirement`;
- сдача: `getReadiness`, `markReadyToSubmit`, `buildSubmissionPackage`, `getSubmissionPackage`, `setPackageCheck`, `confirmSubmission`, `getSubmissionReceipt`;
- system/provider/write: `revealInFinder`, `openProviderApp`, `connectProvider`, `disconnectProvider`, `getProviderStatus`, `updateAutomationRules`, `undoOperation`.

UI продолжает использовать `MockDesktopBridge`: импорт Tauri-фабрики в основной `createBridge()` отсутствует.

### Tauri event adapter

`tauri/events.ts` лениво импортирует `@tauri-apps/api/event` и предоставляет типизированный `listenToBridgeEvent`. Синхронный `Unsubscribe` помечает подписку неактивной немедленно. Если настоящий `unlisten` приходит позже, он вызывается сразу; повторная отписка безопасна, а callback после неё игнорируется. Ошибка регистрации нормализуется в `BridgeError` и передаётся контролируемому error handler, поэтому rejected registration promise не становится unhandled rejection.

Runtime detection едина для IPC и events. В browser/mock окружении Tauri globals и модули не требуются. Rust хранит точные имена событий и camelCase payload DTO, но до появления write-операций в 4C.2B production events не эмитятся.

## 8. Native и browser/mock file intake

- Browser/mock использует HTML5 `FileList`; `DroppedItem` содержит `name`, `sizeBytes`, а `sourcePath` отсутствует.
- Native Tauri drag-and-drop включён через `app.windows[].dragDropEnabled: true`. Адаптер должен слушать window drag-drop, преобразовывать paths через `droppedItemsFromNativePaths` и вызывать подписчиков `onNativeFileDrop`.
- `pickFiles()` открывает системный выбор файлов в Tauri adapter и возвращает элементы с абсолютным `sourcePath`.
- Для native intake `sourcePath` обязателен. `assertNativeSourcePaths` выявляет отсутствие пути и останавливает импорт с `io_error`, а не создаёт имитацию.
- Размер, тип и существование native файла уточняет Rust backend; frontend не читает файл.
- Реальный диалог выбора и native event listener требуют Tauri adapter/plugin/backend и на текущем этапе отсутствуют.

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

На этапе 4C.2B реализовать согласованный набор write-команд и места фактического emit, затем отдельно принять решение о подключении `createTauriBridge()` к UI. Далее остаются системный picker/native drop, watcher, классификация, атомарные копирования/backup/undo, открытие Finder и официальных приложений, provider adapters, хэши, пакеты и immutable snapshot.
