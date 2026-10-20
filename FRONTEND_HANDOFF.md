# Контур — frontend handoff

Документ для backend-разработчика. Frontend готов и работает на `MockDesktopBridge`. Задача backend — реализовать интерфейс `DesktopBridge` поверх Tauri-команд и событий, не меняя компоненты.

Источник истины по типам — `src/lib/bridge/types.ts`. Этот документ объясняет смысл и правила.

---

## 1. Архитектура

```
React-компоненты ──► хуки (useBridgeQuery / useBridgeEvent / useRulesDraft)
                          │
                          ▼
              src/lib/bridge/index.ts  →  export const bridge: DesktopBridge
                          │
          ┌───────────────┴───────────────┐
  MockDesktopBridge (сейчас)        TauriDesktopBridge (написать)
  src/lib/bridge/mock/              invoke() + listen()
```

Правила:

- Компоненты импортируют только `bridge` и типы из `src/lib/bridge`. Прямых вызовов `@tauri-apps/api`, `fetch`, файловой системы во frontend нет и быть не должно.
- Frontend **не вычисляет** пути, имена файлов и номера версий окончательно. `previewRoute` и `applyNamePattern` (`src/lib/naming.ts`) нужны только для предпросмотра; итог всегда приходит в ответе операции.
- Все мутации возвращают `OperationResult<T>`: значение + запись `Operation` для журнала и отмены.

### Где подключается настоящий backend

Файл `src/lib/bridge/index.ts`, функция `createBridge()`:

```ts
function createBridge(): DesktopBridge {
  if (import.meta.env.VITE_BRIDGE === "tauri") return new TauriDesktopBridge();
  return new MockDesktopBridge();
}
```

Создайте `src/lib/bridge/tauri/TauriDesktopBridge.ts`, реализуйте `DesktopBridge`. Больше во frontend ничего менять не нужно. Rust-команды регистрируются в `src-tauri/src/lib.rs` (`invoke_handler`), права — в `src-tauri/capabilities/default.json`.

Рекомендуемое соответствие: метод `getInboxFiles` → команда `get_inbox_files`, аргументы — camelCase объект, как в интерфейсе. Ошибки команды сериализуйте как `{ code: BridgeErrorCode, message: string }` и в адаптере бросайте `new BridgeError(code, message)` — UI различает `confirmation_required` (жёлтый тост) и остальные (красный).

---

## 2. Маршруты

Hash-router (`createHashRouter`) — работает под `tauri://` без настройки.

| Путь | Компонент | Данные |
|---|---|---|
| `/` | `DashboardPage` | `getDashboard` |
| `/inbox` | `InboxPage` | `getInboxFiles`, `previewRoute`, `routeFiles`, `updateClassification`, `importDroppedFiles` |
| `/week` | `WeekIndexRedirect` | редирект на текущую неделю предмета |
| `/subjects/:subjectId/week/:n` | `WeekPage` | `getWeeks`, `getHomework`, `getActivityLog`, `createWeek` |
| `/homework` | `HomeworkListPage` | `getHomeworkList` |
| `/homework/:homeworkId` | `HomeworkDetailPage` | `getHomework`, `getHomeworkCandidates`, `createHomeworkVersion`, `markVersionFinal`, `markHomeworkSubmitted`, `getAutomationRules` |
| `/ai?homework=<id>` | `AiWorkspacePage` | `getHomeworkList`, `getWeek`, `getProviderStatus`, `prepareAiPackage`, `runAiPackage`, `getAiRuns` |
| `/automation` | `AutomationPage` | `getAutomationRules`, `updateAutomationRules`, `getActivityLog`, `undoOperation` |
| `/settings` | `SettingsPage` | то же + провайдеры |

---

## 3. Модель данных

Кратко (полные определения — `types.ts`):

| Тип | Назначение |
|---|---|
| `Subject` | предмет: `id`, `name`, `shortName`, `currentWeek`, `weekCount` |
| `Week` | неделя предмета: даты, тема, `lectures` / `seminars` / `materials: FileRef[]`, `homeworkId` |
| `FileRef` | файл в хранилище: `relativePath` от корня хранилища Obsidian (только для показа) |
| `InboxFile` | файл во «Входящих»: `status`, `classification`, `duplicate`, `origin` |
| `Classification` | `subjectId`, `weekNumber`, `category`, `confidence 0…1`, `reasons[]`, `source: auto/manual` |
| `RoutePreview` | итоговые `targetPath`, `targetName`, `createsWeek`, `duplicate` |
| `Homework` | `stage`, `taskFiles`, `versions[]`, `finalVersionId`, `finalFileName`, `submittedAt`, `folderPath` |
| `HomeworkVersion` | `number`, `label` (`v01`), `fileName`, `sections[]` с хэшами для сравнения |
| `ProviderStatus` | `state`, `accountLabel` (отображаемое имя, **не токен**), `errorMessage` |
| `AiPackage` / `AiRun` | пакет материалов + промпт; запуск с прогрессом и `resultFiles` |
| `AutomationRules` | пути, имя студента, шаблоны, правила предметов, порог уверенности, подтверждения |
| `Operation` | запись журнала: `kind`, `status`, `undoable`, `paths`, `backupPath` |

Структура хранилища, на которую рассчитан UI:

```
<Obsidian>/<Предмет>/Неделя 07/
├── 01 Лекции
├── 02 Семинары
├── 03 ДЗ
│   ├── 01 Условие и данные
│   ├── 02 Работа
│   ├── 03 На сдачу
│   └── 04 Отправлено
└── 04 Материалы
```

Категория → папка: `lecture` → `01 Лекции`, `seminar` → `02 Семинары`, `homework_task`/`homework_data` → `03 ДЗ/01 Условие и данные`, `homework_draft` → `03 ДЗ/02 Работа`, `material` → `04 Материалы`.

Статус файла во «Входящих»: `new` → `classifying` → `classified` | `ambiguous` → `routed`. `ambiguous`, если нет предмета/недели, тип `unknown` или `confidence < rules.lowConfidenceThreshold`. `getInboxFiles` не возвращает `routed`.

---

## 4. Методы моста

| Метод | Контракт |
|---|---|
| `getDashboard()` | агрегат для главной; дедлайны отсортированы по возрастанию |
| `getSubjects()` / `getWeeks(subjectId)` / `getWeek(subjectId, n)` | чтение структуры |
| `createWeek(subjectId, n)` | создаёт папки 01–04; `conflict`, если неделя есть |
| `getInboxFiles()` | нераз­ложенные файлы |
| `importDroppedFiles(items)` | регистрирует перетащенные файлы, эмитит `inbox:file-added`, запускает классификацию. В браузере `items` без `path`; пустой массив = демо-набор (только mock) |
| `classifyFiles(ids)` | на каждый файл — `inbox:classification-updated` и `operation:progress` |
| `updateClassification(id, patch)` | ручная правка; `source: "manual"`, эмитит `inbox:classification-updated` |
| `previewRoute(id, destination)` | итоговый путь и дубль **без** изменений на диске |
| `routeFiles(ids, destination \| null)` | `null` = по классификации каждого файла. Нерешённые → `BridgeError("confirmation_required")`. Дубль никогда не перезаписывает: сохраняется под `duplicate.suggestedName` |
| `getHomeworkList()` / `getHomework(id)` | ДЗ |
| `getHomeworkCandidates(id)` | файлы в `02 Работа`, ещё не ставшие версиями |
| `createHomeworkVersion(hwId, fileId)` | следующий номер, эмитит `homework:version-created`; `task` → `in_progress` |
| `markVersionFinal(versionId)` | копия в `03 На сдачу` с именем по шаблону; `stage = ready` |
| `markHomeworkSubmitted(hwId, submittedAt)` | **только после подтверждения пользователя**; перенос в `04 Отправлено`; ничего не отправляет в сеть |
| `prepareAiPackage(hwId, provider, {fileIds?, prompt?})` | собирает папку пакета, возвращает промпт по умолчанию, если `prompt` не передан |
| `runAiPackage(packageId)` | передаёт пакет клиенту провайдера; ошибки `provider_unavailable` / `reauth_required`; прогресс через `operation:progress` с `kind: "ai_run"` |
| `getAiRuns()` | история, новые сверху |
| `connectProvider(p)` / `disconnectProvider(p)` / `getProviderStatus(p)` | см. раздел 6 |
| `getAutomationRules()` / `updateAutomationRules(rules)` | backend обязан игнорировать попытки поменять `confirm.markSubmitted` (всегда `true`) и `confirm.allowOverwrite` (всегда `false`) |
| `getActivityLog()` | операции, новые сверху |
| `undoOperation(opId)` | откат из резервной копии; `not_undoable`, если нельзя |
| `on(event, listener)` | подписка, возвращает отписку |

Методы, добавленные к исходному списку (нужны UI): `getWeek`, `importDroppedFiles`, `updateClassification`, `previewRoute`, `getHomeworkList`, `getHomework`, `getHomeworkCandidates`, `runAiPackage`, `getAiRuns`.

---

## 5. События

| Событие | Payload | Кто слушает |
|---|---|---|
| `inbox:file-added` | `{ files }` | Входящие, счётчик в боковой панели, главная |
| `inbox:classification-updated` | `{ fileId, classification, status }` | строки Входящих (сканирующая линия → метки и метр) |
| `operation:progress` | `{ operationId, kind, progress 0…1, message }` | индикатор в заголовке окна; волна AI для `kind: "ai_run"` |
| `operation:completed` | `{ operation }` | перезагрузка данных всех экранов; кнопка «Отменить» в заголовке; тост AI |
| `operation:failed` | `{ operation, error }` | красный тост |
| `provider:status-changed` | `{ status }` | карточки провайдеров (главная, AI, настройки) |
| `homework:version-created` | `{ homeworkId, version }` | карточка ДЗ |

Порядок: `operation:completed` должен эмититься **до** резолва промиса метода (так делает mock) — экраны перезагружаются по событию, а локальная анимация срабатывает по ответу.

Файловый наблюдатель backend (загрузки, изменения в `02 Работа`) должен эмитить те же события — UI обновится без опроса.

---

## 6. Авторизация провайдеров

«Контур» не хранит и не видит пароли, cookies, токены и API-ключи. claude.ai и ChatGPT в webview не встраиваются. Вход выполняется в официальном клиенте провайдера (CLI или desktop-приложение); backend сообщает только состояние.

| `state` | UI | Действие |
|---|---|---|
| `disconnected` | серая точка, «Подключить» | `connectProvider` |
| `connecting` | вращающееся кольцо, кнопки заблокированы | ждать `provider:status-changed` |
| `connected` | зелёная точка, `accountLabel` | «Отключить» |
| `error` | красная точка, `errorMessage` | «Повторить» → `connectProvider` |
| `reauth_required` | жёлтая точка, «Войти снова» | `connectProvider` |

`connectProvider` сразу эмитит `connecting`, затем итоговое состояние. Mock: Claude стартует в `reauth_required`; первая попытка Codex заканчивается `error`, вторая — `connected`.

---

## 7. Правила версионирования

- Версия = файл в `02 Работа`, зарегистрированный через `createHomeworkVersion`. Исходник не меняется.
- Номер — следующий после максимального, формат `v01`, `v02`… Номера не переиспользуются, даже после отмены.
- Итоговая — ровно одна. `markVersionFinal` копирует файл в `03 На сдачу` с именем `rules.finalNamePattern` (токены `{Фамилия} {Имя} {Неделя} {Предмет} {Версия}`), по умолчанию `Гончарова_Доминика_Неделя7 ДЗ.<ext>`. Повторное назначение заменяет копию, предварительно сделав резервную.
- После `submitted` версии и итоговую менять нельзя (`conflict`), пока операция не отменена.
- `sections[].hash` — хэши логических разделов (листы Excel, задачи, ячейки notebook). Frontend сравнивает версии только по ним.

Этапы ДЗ: `task` (нет версий) → `in_progress` (есть версия) → `ready` (есть итоговая) → `submitted` (подтверждено вручную).

---

## 8. Нельзя обходить

- `bridge` из `src/lib/bridge` — единственный вход в backend.
- `useBridgeQuery` / `useBridgeEvent` — загрузка и подписки (обновление по событиям, отмена гонок).
- `OperationsProvider` (`useOps`) — отмена (`undo`) и показ ошибок (`reportError`). Не вызывайте `bridge.undoOperation` в обход него, иначе кнопка «Отменить» в заголовке рассинхронизируется.
- `useRulesDraft` — единственный способ менять правила (общий для «Автоматизации» и «Настроек»).
- `ProviderCard` — единственное место подключения/отключения провайдеров.
- Диалог подтверждения в `HomeworkDetailPage` — единственный путь к `markHomeworkSubmitted`. Не вызывайте его из фоновых задач.
- `DropZone` → `importDroppedFiles`. В Tauri drag-drop окна выключен (`dragDropEnabled: false`), используется HTML5 drop. Если нужны абсолютные пути, включите `dragDropEnabled`, слушайте `onDragDropEvent` в адаптере и передавайте `DroppedItem.path`.

---

## 9. Анимации, привязанные к backend

| Анимация | Триггер |
|---|---|
| Сканирующая линия в строке файла | `status: new/classifying` → смена по `inbox:classification-updated` |
| Перелёт плашки из списка в «Разложено сейчас» (shared `layoutId="file-<id>"`) + импульс группы | успешный ответ `routeFiles` |
| Индикатор операции в заголовке | `operation:progress` / `operation:completed` |
| Перестройка рельса ДЗ (заполнение + перетекание маркера `hwrail-marker-<id>`) | смена `Homework.stage` |
| Перетекание карточки ДЗ между колонками доски (`hwcard-<id>`) | смена `stage` после перезагрузки |
| Новая карточка версии справа, сдвиг старых, перелёт флажка «Итоговая» (`final-flag`) | `createHomeworkVersion` / `markVersionFinal` |
| Волна обработки AI | `operation:progress` с `kind: "ai_run"`; останавливается при скрытом окне и reduced motion |
| Кольцо подключения провайдера | `state: connecting` |
| Импульс успеха в тосте | любая успешная мутация |

Не привязаны к backend: стартовая история (точка → линия процесса), переходы экранов (≈220 мс), подсветка в палитре и навигации, магнитное свечение drop-зоны.

---

## 10. Что должен подключить backend

1. `TauriDesktopBridge` и переключение в `createBridge()`.
2. Rust-команды для всех методов и эмиссию событий (`app.emit`).
3. Наблюдатель папки загрузок → перенос во «Входящие» → `inbox:file-added`.
4. Классификатор (ключевые слова из `rules.subjectRules`, неделя по имени/календарю), уверенность и `reasons`.
5. Файловые операции с резервной копией (`.kontur/backup/...`) и откатом; запрет перезаписи.
6. Хэши разделов для сравнения версий.
7. Обновление заметок Obsidian и Finder (теги, ссылки, `Неделя NN.md`) — UI этого не делает.
8. Интеграция с локальными клиентами Codex и Claude: статус, передача папки пакета, сбор результата.
9. Выбор папок через системный диалог (сейчас пути в настройках — текстовые поля).
10. Права в `src-tauri/capabilities/default.json` — только необходимые; frontend доступа к FS не получает.
11. Сайт университета — **только** чтение условий (если нужно); отправка работ не автоматизируется.

Mock-реализация (`src/lib/bridge/mock/`) — рабочий пример ожидаемого поведения: порядок событий, ошибки, тексты.
