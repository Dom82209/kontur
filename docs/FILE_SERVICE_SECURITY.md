# Безопасность файлового сервиса

Статус: этап 5 — fd-relative фундамент подключён к импорту во «Входящие» (`src-tauri/src/intake/`). Прямого IPC к file service нет; при старте приложение файлов не трогает. Все filesystem-тесты используют только `tempfile::TempDir`.

## Модель угроз

Атакующий — любой локальный процесс того же пользователя, который во время операции может переименовывать, удалять и создавать файлы, каталоги и symlink внутри vault, inbox, backup и рядом с source. Цель атаки — заставить «Контур» записать, перезаписать, прочитать как доверенный или удалить файл за пределами контролируемого корня, либо удалить чужой файл внутри корня.

Строковые проверки пути (`canonicalize`, `starts_with`, `symlink_metadata`) с последующей операцией по абсолютному пути не защищают от этого: между проверкой и `open`/`rename`/`unlink` родительский каталог можно заменить symlink (TOCTOU). Поэтому после этапа 4D.1.1 строковый путь разрешается только один раз — при открытии корня или source.

## Trust boundaries

- `VaultRoot`, `BackupRoot` — абсолютный существующий каталог, не `/`, не домашний и не широкий системный (`/x`). Политика проверяется по canonical path, затем корень **один раз** открывается `open(O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`. Дальше корень представлен только открытым directory fd (`Arc<OwnedFd>`). Значение AutomationRules `"."` по-прежнему отклоняется.
- `InboxRoot` — не путь, а fd, полученный покомпонентным обходом от fd vault. Symlink или не-каталог в любом компоненте отклоняется.
- `SourceFile` — недоверенный абсолютный путь из native intake. Он открывается **один раз** `open(O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_NOCTTY | O_CLOEXEC)`; symlink в последнем компоненте даёт `ELOOP` → `blocked`. Тип проверяется `fstat` открытого fd: каталог, FIFO, socket и устройства отклоняются. `O_NONBLOCK` исключает зависание на FIFO до проверки типа. Путь не сериализуется и не сохраняется.
- `PersistentPath` — единственная сериализуемая форма учебного пути: относительная, `/`-разделители, без `..`, `.`, пустых компонентов, `file://`, backslash, абсолютного или Windows drive prefix.
- `VerifiedDestination` связывает открытый root и `PersistentPath`. Абсолютный `PathBuf` наружу не выдаётся; метода получения абсолютного пути больше нет.

## fd-relative обход

`fd::walk_parent` открывает каждый промежуточный компонент отдельно:

```text
current = root_fd
for name in components[..last]:
    current = openat(current, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
```

- Symlink в любом компоненте отклоняется ядром (`ELOOP`); regular file — `ENOTDIR`. Оба дают `blocked`.
- При создании отсутствующий компонент создаётся `mkdirat(current, name, 0755)`, затем снова открывается `openat(... O_NOFOLLOW)`. Если конкурент успел положить на это имя symlink, повторное открытие отклоняется; `EEXIST` от гонки двух `mkdirat` безопасен.
- Результат — fd родительского каталога. Все дальнейшие операции идут через него, поэтому последующая подмена любого компонента пути не перенаправляет операцию: fd продолжает указывать на исходный inode каталога.

### Контроль подмены родителя

Fd защищает от записи «не туда», но после подмены относительный путь в receipt мог бы перестать указывать на созданный файл. Поэтому `fd::verify_parent_identity` повторно обходит путь от root fd без создания и сравнивает `(st_dev, st_ino)` полученного каталога с уже открытым fd родителя. Несовпадение, symlink или исчезновение каталога дают `blocked` («Целевая папка была подменена во время операции»).

## Используемые системные вызовы

| Действие | Вызов |
|---|---|
| Открытие корня | `open(O_RDONLY \| O_DIRECTORY \| O_NOFOLLOW \| O_CLOEXEC)` |
| Обход каталога | `openat(dir_fd, name, O_RDONLY \| O_DIRECTORY \| O_NOFOLLOW \| O_CLOEXEC)` |
| Создание каталога | `mkdirat(dir_fd, name, 0755)` + повторный `openat` с `O_NOFOLLOW` |
| Проверка имени | `statat(dir_fd, name, AT_SYMLINK_NOFOLLOW)` |
| Чтение существующего файла | `openat(dir_fd, name, O_RDONLY \| O_NOFOLLOW \| O_NONBLOCK \| O_NOCTTY \| O_CLOEXEC)` + `fstat` |
| Создание temp | `openat(dir_fd, ".kontur-<uuid>.tmp", O_RDWR \| O_CREAT \| O_EXCL \| O_NOFOLLOW \| O_CLOEXEC, 0644)` |
| Публикация | `renameat_with(dir_fd, temp, dir_fd, final, RENAME_NOREPLACE)`; на macOS rustix отображает флаг в `renameatx_np(RENAME_EXCL)` |
| Cleanup | `statat(dir_fd, name, AT_SYMLINK_NOFOLLOW)` + сравнение identity + `unlinkat(dir_fd, name, 0)` |
| Синхронизация | `fsync(temp_fd)`, `fsync(dir_fd)` |
| Чтение для SHA-256 | `pread` (`FileExt::read_at`) по открытому fd |

Все создаваемые fd имеют `CLOEXEC`. Мутации внутри корней не выполняются через абсолютные строковые пути. Path-based вызовы остались только там, где путь ещё не привязан к fd: проверка политики корня с его однократным открытием и однократное открытие source.

## Source: один inode

`SourceFile` хранит открытый `File` и identity `(st_dev, st_ino)`. `revalidate()` делает `fstat` того же fd и требует regular file с той же identity. Хэш (`hash_source`), размер и копирование читают **один открытый inode** через `pread` от смещения 0. Копирование выполняется одним проходом: каждые скопированные байты одновременно хэшируются, поэтому receipt SHA-256 описывает именно записанное содержимое. Размер `fstat` до копирования, после копирования и число скопированных байт обязаны совпасть. Подмена пути source после открытия не влияет на операцию (`source_is_bound_to_the_inode_opened_once`).

## SHA-256 и планирование

SHA-256 вычисляется блоками по 64 KiB через `pread` и возвращается lowercase hex; файл целиком в память не загружается.

`plan_copy` read-only:

1. `fstat` source;
2. fd-relative обход существующих родителей без создания; отсутствующий родитель → `Copy`;
3. `openat(... O_NOFOLLOW)` конечного имени: нет файла → `Copy`; symlink/каталог/специальный файл → `blocked`;
4. сравнение SHA-256 открытого source и открытого destination: совпало → `SameContent` (копия не создаётся);
5. иначе `NameConflict { suggestedName }`: имена `Файл (2).pdf`, `Файл (3).pdf`… проверяются `statat` в том же fd каталога; расширение и кириллица сохраняются, длина ограничена;
6. перед возвратом решения — `verify_parent_identity`. Подмена родителя во время planning даёт `blocked`, а содержимое за пределами корня никогда не читается как destination.

Planner не создаёт каталогов и файлов.

## Atomic copy

1. `fstat` source, запоминается размер.
2. `ensure_parent`: fd-relative обход с `mkdirat`.
3. `verify_parent_identity`, проверка отсутствия final через `statat`.
4. `openat(O_CREAT | O_EXCL | O_NOFOLLOW)` temp в fd родителя; запоминается identity temp. Создаётся guard.
5. Однопроходное копирование + SHA-256, `fsync(temp)`, сверка размера, сверка с ожидаемым хэшем (если задан), повторный хэш temp через тот же fd.
6. `verify_parent_identity`, повторная проверка отсутствия final.
7. `renameat_with(..., RENAME_NOREPLACE)` внутри одного dir fd. Конкурентно появившийся final даёт `conflict`; overwrite невозможен.
8. `verify_parent_identity`; `statat(final, AT_SYMLINK_NOFOLLOW)` обязан вернуть regular file с identity temp и ожидаемым размером; повторный хэш через fd созданного inode.
9. `fsync(dir_fd)`; guard подтверждается.

### Guards и cleanup

`TempGuard` и `FinalGuard` хранят `Arc` fd подтверждённого каталога, имя и identity созданного inode. При ошибке они **не** разрешают абсолютный путь: выполняется `statat(dir_fd, name, AT_SYMLINK_NOFOLLOW)`, и `unlinkat(dir_fd, name, 0)` вызывается только если имя всё ещё указывает на regular file с той же `(st_dev, st_ino)`. Если конкурент заменил temp другим файлом, чужой файл не удаляется (`cleanup_only_removes_inode_created_by_this_operation`). Если каталог был заменён symlink, удаление происходит в исходном inode каталога, а файл с тем же именем снаружи не трогается.

Остаточное окно: POSIX не даёт атомарного «unlink, если inode совпадает». Между `statat` и `unlinkat` процесс с правами записи в тот же подтверждённый каталог может переименовать другой файл на это же имя. Такой процесс и так может удалить этот файл сам; выйти за пределы каталога через это окно нельзя.

## Backup

`create_backup` открывает оригинал fd-relative внутри vault (`SourceFile::open_in`: обход без symlink, `openat(O_NOFOLLOW)`, проверка identity родителя), строит путь `<operationId>/<relative>` в `BackupRoot`, выполняет `plan_copy` (существующий backup → `conflict`), считает SHA-256 открытого inode и копирует через `atomic_copy_expected`. Изменение содержимого между хэшем и копированием отклоняется. `BackupManifest` в camelCase содержит только operation ID, относительные source/backup paths, SHA-256, размер и UTC creation time.

## Undo foundation

`UndoManifest` в camelCase содержит operation ID, созданные файлы с ожидаемыми SHA-256 и backup entries с относительными original/backup paths и ожидаемым backup SHA-256.

`validate_undo` требует `undoable`, отклоняет уже отменённую операцию и submitted Homework. Для каждого файла: fd-relative обход родителей (symlink в промежуточном компоненте → `blocked`), `openat(O_NOFOLLOW)` конечного имени (symlink или не regular → `not_undoable`, отсутствие → `not_undoable`), SHA-256 открытого inode, затем `verify_parent_identity` (подмена → `blocked`). Для backup дополнительно проверяется родитель original. Изменённый созданный файл или повреждённый backup → `not_undoable`.

Применение undo, связь с SQLite Operation и Tauri command намеренно не реализованы.

## Тестовые точки и race-тесты

`file_service/hooks.rs` объявляет точки `BeforeTempCreate`, `BeforeRename`, `BeforeFinalVerify`, `BeforeCleanup { fileName }`, `PlanningParentOpened`, `UndoParentOpened`. В production-сборке `hooks::fire` — пустая `#[inline(always)]` функция, замыкание не вызывается. В `cfg(test)` хук thread-local и вызывается синхронно, поэтому подмена происходит строго между известными системными вызовами без sleep. Один тест дополнительно использует `std::sync::Barrier`: атакующий поток подменяет каталог, пока операция остановлена перед `rename`.

`race_tests.rs` (22 теста) проверяет подмену родителя на symlink до создания temp, перед rename, перед финальной проверкой, перед cleanup, во время planning и во время undo (created file и каталог backup); symlink в промежуточном компоненте, созданный после проверки destination; symlink leaf с тем же содержимым; regular file как промежуточный компонент; подмену source после открытия; FIFO как source; подмену самого vault root после открытия; symlink-компонент оригинала backup; появление destination перед rename (overwrite невозможен); сохранение `(2)`/`(3)` нумерации с кириллицей и отсутствие копии при `SameContent`. Каждый race-тест сравнивает побайтный снимок каталога `outside` до и после операции и проверяет, что в исходном каталоге не осталось temp/final файлов.

## Платформенные замечания

- Целевая платформа — macOS (APFS). `renameat_with(NOREPLACE)` использует `renameatx_np(RENAME_EXCL)` (macOS 10.12+); на Linux — `renameat2(RENAME_NOREPLACE)`.
- Если vault root переименован и на его место положен symlink уже после открытия, операции продолжаются в исходном inode корня, а не по новому пути (`vault_root_swapped_after_open_keeps_operations_in_original_inode`). Повторная проверка выбранного пользователем корня — задача слоя настроек при следующем открытии.
- Hardlink на regular file не является escape: запись идёт только в новые inode, существующие файлы не открываются на запись.

## Импорт во «Входящие» — этап 5

### Кто может назвать source

WebView не может попросить backend прочитать произвольный файл. `import_inbox_files` принимает только пути, которые backend сам получил: из системного диалога (`pick_import_files`, официальный `tauri-plugin-dialog` из Rust) или из native drag-and-drop окна (`WindowEvent::DragDrop`). Такие пути записываются в `IntakeGrants` (память процесса, TTL 30 минут, ≤ 4096 записей, только абсолютные regular files); путь вне списка → `blocked`. Повтор с тем же путём в пределах TTL разрешён — это нужно для идемпотентного повтора команды.

### `SourceFile::open_external`

Строже `SourceFile::new`:

1. абсолютный путь без `.`/`..`;
2. `lstat` каждого предка: symlink в родителе отклоняется (`blocked`), кроме системных алиасов macOS `/var`, `/tmp`, `/etc` → `/private/...` (иначе нельзя открыть ни один файл из `$TMPDIR`);
3. канонический родитель открывается покомпонентно от `/` через `openat(O_DIRECTORY | O_NOFOLLOW)`;
4. файл — `openat(O_NOFOLLOW | O_NONBLOCK)` + `fstat`: symlink, каталог, FIFO и устройства отклоняются;
5. `stat(path)` после открытия обязан указывать на тот же `(st_dev, st_ino)` — подмена между проверкой и открытием даёт `blocked`.

Остаточное окно: между `lstat` предков и `openat` канонического родителя локальный процесс может переставить каталоги; файл будет открыт из реального (не symlink) каталога, а шаг 5 отклонит несовпадение с указанным пользователем путём. Источник всё равно выбран самим пользователем.

### Порядок записи и компенсация

1. SHA-256, размер и копия читают один открытый inode; размер ≤ 4 ГБ.
2. Дубль по SHA-256 среди неразложенных записей → `duplicate` без копии (если файла записи на диске нет — он восстанавливается копированием; если файл на диске изменён — `conflict`, ничего не трогается).
3. Имя `Предметы/00 Входящие/<safe name>`; имя, занятое на диске (`plan_copy` → `NameConflict`) или в БД, заменяется на `<stem> (N).<ext>`. Файл с тем же содержимым на диске без записи (прерванный импорт) принимается без копии.
4. `atomic_copy_committed` (temp → fsync → `RENAME_NOREPLACE` → проверка inode и SHA-256 → fsync каталога) возвращает `CommittedCopy`.
5. Только затем — `INSERT INTO inbox_files`. Если вставка не удалась, `CommittedCopy::compensate` удаляет имя через `unlinkat(dir_fd)` **только** если оно указывает на созданный inode; чужой файл на том же имени не удаляется. Неудачная компенсация отражается в сообщении, а повторный импорт примет оставшийся файл без дубля.

### Безопасное имя

Разделители `/`, `\`, `:` и управляющие символы → `-`; bidi-override и zero-width символы удаляются (защита от визуальной подмены расширения); ведущие точки снимаются (скрытые файлы и префикс `.kontur-`); длина ≤ 200 байт UTF-8 с сохранением расширения до 16 байт; пустое имя → `Файл`.

### Действия ОС

`open_inbox_file`, `reveal_inbox_file`, `reveal_in_finder` строят абсолютный путь из канонического корня хранилища только после fd-relative проверки относительного пути (без symlink, конечный элемент существует). Исполняемые и скриптовые расширения (`.command`, `.sh`, `.app`, `.pkg`, `.webloc`, `.py` и др.) не открываются — только показ в Finder. Окно не имеет `dialog:*`, `opener:*` и `fs:*` permissions; доступны лишь 20 `allow-<command>` «Контура».

## Не подключено к production

Watcher загрузок, ZIP из LMS, классификация, раскладка по предметам и применение backup/undo. `createBridge()` по умолчанию использует `MockDesktopBridge`.
