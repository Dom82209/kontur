# План production homework workflow

Дата фиксации: 27 октября 2026 года.

Этот документ фиксирует порядок изменений и границы контрактов до реализации. Этапы выполняются последовательно; следующий этап не начинается при красных проверках предыдущего.

## Карта контрактов

| Этап | Rust / SQLite | IPC / DesktopBridge | Frontend | Проверки безопасности |
|---|---|---|---|---|
| 1. Workspace boundary | глобальный каталог workspace, отдельная SQLite на workspace, identity/fingerprint, журнал переходов | preview/confirm/cancel перехода, список и активное workspace | выбор demo/real, явный preview и подтверждение | TempDir, traversal, symlink, rollback, отсутствие смешивания |
| 2. Vault adoption | read-only scanner, import plan, stable identity, относительные пути | scan/preview/apply/dry-run, отчёт | мастер существующего vault | повторный hash перед apply, no-follow, идемпотентность |
| 3. Subjects | CRUD/archive/merge/rules/settings threshold | subject DTO и команды | управление предметами и ручной override | приоритет ручного решения, preview merge |
| 4. Homework 0..N | миграция unique week relation, stable import id | обновлённые query/mutation DTO | списки недели/ДЗ/AI | три ДЗ одной недели, upgrade без потерь |
| 5. Semester | semester/calendar/deadline metadata | calendar и assignment mutations | настройка семестра и точных сроков | timezone/year boundaries, unknown deadline |
| 6. Automation | persisted effective settings | единый settings contract | только реально работающие controls | hydration/restart и behavioral tests |
| 7. Authority/stale | comment authority, dependency revisions | invalidation-aware mutations | authority и stale blockers | все invalidation transitions |
| 8. Package v2 | immutable revisioned package/snapshot | build/check/confirm v2 | revisions/manual checks | negative tests, revalidation on confirm |
| 9. Style | explicit sources, traits, revision | build/read profile | evidence and manual-required UI | source consent, invalidation |
| 10. Versions | immutable lineage, typed diff/correction plan | lineage/diff DTO | сравнение и решения пользователя | unsupported => manual_required |
| 11. UX | без новой бизнес-логики | существующие production contracts | все состояния workflow | themes, sizes, keyboard, reduced motion |
| 12. Providers | существующая account isolation | status/consent/manual handoff | честный account/manual/unauthorized | sentinel tests, без реального AI |
| 13. Audit | upgrade/new DB audit | contract traceability | manual TempDir E2E | secrets scan, full build, read-only real-vault preview |

## Архитектурное решение workspace

Учебные сущности изолируются отдельной SQLite на workspace. Глобальная база-каталог хранит только identity, тип, fingerprint, относительный путь к workspace SQLite, активный выбор и журнал переходов. Это делает невозможным SQL-запрос, случайно смешивающий предметы, ДЗ, AI jobs, packages или receipts двух vault.

Путь vault нельзя заменить у активного workspace. Новый vault создаёт новый identity; ранее зарегистрированный vault подключается только после совпадения marker/fingerprint. Preview перехода не пишет в vault или workspace SQLite. Apply требует отдельного подтверждения и повторной валидации.

## Статус этапов

| Этап | Статус | Коммит |
|---|---|---|
| 1. Workspace boundary | готов: preview/confirm/cancel, журнал, restart, legacy demo/real, symlink/traversal/rollback, 42 теста; ревью-фиксы (вложенные vault, symlink demo, неизменяемый fingerprint) | `feat(workspaces): isolate demo and real data`, `fix(workspaces): harden stage 1 workspace boundary` |
| 2. Vault adoption | готов: read-only scan, неизменяемый план, preview/dry-run/apply/cancel, стабильные id, повторный SHA-256 перед apply, ручные категории, UI-мастер; 14 Rust-тестов + mock/IPC/render-тесты; dry-run на зеркале структуры настоящего vault | `wip(adoption): …` ×2, `feat(adoption): …` ×2, `feat(ui): show unknown deadlines…` |

## Этап 2: подключение существующего vault

Состав:

- `file_service/scan.rs` — read-only fd-relative обход vault (`O_NOFOLLOW`; скрытые записи не открываются; symlink, спецфайлы и имена не в UTF-8 попадают только в отчёт; лимиты 50 000 записей / глубина 12) и потоковый SHA-256.
- `adoption.rs` + миграция v11 — неизменяемый план `vault_adoption_plans` (preview → apply / dry-run / cancel, TTL 30 мин, SHA-256 плана, триггеры неизменяемости), стабильные id из identity пространства и относительного пути (идемпотентность), повторное хэширование каждого нового файла перед apply, одна транзакция, provenance в `vault_adoption_items`. Изменённые после импорта файлы и конфликты с базой не применяются, а показываются.
- Классификация: папка недели задаёт категорию; внутри `03 ДЗ` — данные (имя), своя работа (фамилия из настроек, кириллица и латиница; версии `_v01`; `.tex`; решение/черновик), условие (LMS-хэш-суффикс `_xxxxxxxx`; PDF с «ДЗ»). Неуверенные случаи (< 0.7) помечены «проверьте категорию»; ручная категория имеет приоритет, категории ДЗ доступны только файлам из `03 ДЗ`.
- Неизвестные сроки ДЗ и даты недель хранятся честно (`deadline = NULL`, даты `''`; DTO — `Option`/`null`), UI пишет «срок не указан» / «даты недели не заданы» (часть этапа 5, перенесена раньше).
- Tauri-команды `preview_vault_adoption`, `apply_vault_adoption(planId, overrides, dryRun)`, `cancel_vault_adoption`; apply берёт общий замок файловых мутаций. UI: «Настройки → Управляемые каталоги → Подключить материалы…».

Проверка на настоящем vault (только чтение): из Obsidian взят лишь список путей (570 файлов, без содержимого); зеркало структуры с файлами-заглушками прогнано через preview и dry-run (`adoption::tests::mirror_of_real_vault_preview`, `#[ignore]`, путь списка в `KONTUR_MIRROR_PATHS`). Итог: 11 предметов, 43 недели, 39 ДЗ, 291 файл к подключению; пропущено 279 (139 заметок Obsidian, 136 файлов `00 Общие материалы`, 3 файла `Неделя 00`, 1 служебная папка); 63 файла с неуверенной категорией; dry-run прошёл без ошибок. Apply на настоящем vault не выполнялся.

## Известные ограничения

- Файлы `00 Общие материалы` и `Неделя 00` не подключаются: схема требует неделю с номером ≥ 1. Они перечислены в отчёте с причиной.
- Изменённые после подключения файлы не обновляются планом (статус `changed`); обновление версий — этап 10.
- Сроки и даты недель после подключения неизвестны до настройки семестра (этап 5).

- Legacy real-пространство (schema ≤ 8) без marker опознаётся по canonical path + dev/inode. Перенос такого vault на другой Mac или восстановление из копии меняет inode, и «Подключить существующее» будет заблокировано. Явное действие «записать marker в legacy vault» пока не реализовано (подключение материалов его не требует); вынесено в этап 13.
- Проверки `cargo test` в Linux-окружении: 20 тестов `ai::*` требуют macOS `sandbox-exec`; окончательный прогон — на Mac (`logs/run-checks.command`, логи в `logs/`).

## Ограничения до финального GO

- Настоящий Obsidian допускается только для read-only preview; apply/import запрещён до production-аудита.
- Все filesystem-тесты используют `TempDir` или отдельную синтетическую копию.
- Codex/Claude не получают настоящий запрос во время тестов.
- Readiness, package и submitted snapshot не считаются основанием для реальной сдачи, пока P0/P1 из аудита не закрыты.
