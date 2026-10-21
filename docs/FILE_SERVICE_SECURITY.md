# Безопасность файлового сервиса

Статус: фундамент этапа 4D.1. Модуль `src-tauri/src/file_service/` не зарегистрирован как Tauri command и не вызывается при старте приложения. Все filesystem-тесты используют только `tempfile::TempDir`.

## Trust boundaries

- `SourceFile` — абсолютный путь, полученный в будущем от native intake. Он считается недоверенным, обязан существовать и быть обычным файлом; symlink и директория отклоняются. Путь не сериализуется и не сохраняется.
- `PersistentPath` — единственная сериализуемая форма учебного пути. Она относительная, использует `/`, не допускает `..`, `.`, пустые компоненты, `file://`, backslash, абсолютный или Windows drive prefix.
- `VaultRoot`, `InboxRoot`, `BackupRoot` — разные типы разрешённых корней. Корень должен быть абсолютным существующим обычным каталогом, canonicalized, не `/`, не домашним каталогом и не широким системным корнем. Значение AutomationRules `"."` не разрешает реальные операции.
- `VerifiedDestination` связывает проверенный root и `PersistentPath`; обычный `PathBuf` не пересекает публичную границу сервиса.

## Path traversal и symlink policy

Путь разбирается по компонентам, а не проверяется строковым `startsWith`. Каждый существующий parent проверяется через `symlink_metadata` и `canonicalize`; symlink или выход canonical parent за root блокирует операцию. Новые каталоги создаются по одному внутри root и после каждого создания проверяются повторно. Проверка повторяется непосредственно перед временной записью и перед atomic publish.

Source symlink отклоняется. Destination symlink, специальный файл, socket или каталог вместо обычного файла не принимаются. Назначение всегда имеет непустой относительный путь и не может быть самим root. Проверки уменьшают TOCTOU-окно; окончательная публикация использует системный no-clobber rename, поэтому конкурентно созданный destination не перезаписывается.

## SHA-256 и планирование

SHA-256 вычисляется блоками по 64 KiB и возвращается в lowercase hex. Файл целиком в память не загружается.

Read-only planner возвращает:

- `Copy`, если destination отсутствует;
- `SameContent`, если SHA-256 совпадает;
- `NameConflict { suggestedName }`, если имя занято другим содержимым.

Варианты имени имеют вид `Файл (2).pdf`, сохраняют расширение и кириллицу, пропускают занятые суффиксы и ограничены безопасной длиной. Planner не создаёт каталогов и файлов.

## Atomic copy

1. Source и destination проверяются повторно.
2. Parent-каталоги создаются по одному только внутри контролируемого root.
3. В destination-каталоге через `create_new` создаётся `.kontur-<uuid>.tmp`.
4. Данные копируются потоком; выполняются `flush` и `sync_all`.
5. Размер и SHA-256 временного файла сравниваются с source.
6. `renameat_with(..., RENAME_NOREPLACE)` атомарно публикует temp под окончательным именем.
7. Проверяются final size/hash и синхронизируется каталог.

RAII guards удаляют temp при любой ошибке и удаляют ещё не подтверждённый final при ошибке post-publish проверки. Source открывается только на чтение. Overwrite не поддерживается.

## Backup

`create_backup` принимает operation ID, `VaultRoot`, относительный source и отдельный `BackupRoot`. Backup создаётся atomic copy до изменения оригинала и никогда не перезаписывается. `BackupManifest` в camelCase содержит только operation ID, относительные source/backup paths, SHA-256, размер и UTC creation time. Абсолютные пути и содержимое файла отсутствуют.

## Undo foundation

`UndoManifest` в camelCase содержит operation ID, созданные файлы с ожидаемыми SHA-256 и backup entries с относительными original/backup paths и ожидаемым backup SHA-256. `validate_undo` требует `undoable`, отклоняет уже отменённую операцию и submitted Homework, повторно проверяет roots, типы файлов и хэши. Изменённый созданный файл или повреждённый backup возвращает `not_undoable`.

Применение undo, связь с SQLite Operation и Tauri command намеренно не реализованы в 4D.1.

## Не подключено к production

Нет native picker/drop, watcher Downloads, чтения реального vault, импорта, routing, ZIP, Tauri file-команд, новых capabilities или startup side effects. `createBridge()` продолжает использовать `MockDesktopBridge`. Следующий этап 4D.2 должен отдельно подключить native intake и импорт во «Входящие» через этот сервис.
