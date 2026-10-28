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
| 2. Vault adoption | **в работе (WIP)**: backend готов и покрыт 13 тестами; IPC, DesktopBridge, mock и UI ещё не сделаны | `wip(adoption): …` |

## Checkpoint этапа 2 (WIP)

Сделано (`src-tauri`):

- `file_service/scan.rs` — read-only fd-relative обход vault (`O_NOFOLLOW`, скрытые записи не открываются, symlink/спецфайлы/не-UTF-8 только в отчёт, лимиты) и потоковый SHA-256.
- `adoption.rs` + миграция v11 — неизменяемый план `vault_adoption_plans` (preview → apply/dry-run/cancel, TTL 30 мин, SHA-256 плана), стабильные id из identity workspace и относительного пути, повторное хэширование каждого нового файла перед apply, одна транзакция, provenance в `vault_adoption_items`, ручные категории с приоритетом. `Неделя 00`, `00 Общие материалы`, служебные папки и заметки — в отчёт пропущенных.
- Часть этапа 5 перенесена раньше, потому что у импортированных ДЗ нет известных дат: v11 разрешает `homeworks.deadline = NULL` и пустые даты недели; `HomeworkDto.deadline` и `WeekDto.startsAt/endsAt` стали `Option`. Лента сроков пропускает ДЗ без срока.

Осталось для этапа 2:

1. Frontend-типы: `deadline: ISODate | null`, `startsAt/endsAt: ISODate | null` и все места отображения/сортировки («срок не указан»). **До этого backend-изменение DTO не согласовано с TS-типами.**
2. Tauri-команды `preview_vault_adoption`, `apply_vault_adoption(planId, overrides, dryRun)`, `cancel_vault_adoption`; регистрация в `lib.rs` и capabilities.
3. `DesktopBridge` (types, ipc, TauriDesktopBridge, Mock + тесты).
4. UI-мастер «Подключить материалы из vault» в настройках: сводка, пропущенные с причинами, ручные категории, dry-run, apply.
5. Read-only preview на копии настоящего vault и полный прогон проверок на Mac (`logs/run-checks.command`).

## Известные ограничения

- Legacy real-пространство (schema ≤ 8) без marker опознаётся по canonical path + dev/inode. Перенос такого vault на другой Mac или восстановление из копии меняет inode, и «Подключить существующее» будет заблокировано. Marker для legacy vault будет предлагаться явным действием (этап 2, vault adoption).
- Проверки `cargo test` в Linux-окружении: 20 тестов `ai::*` требуют macOS `sandbox-exec`; окончательный прогон — на Mac.

## Ограничения до финального GO

- Настоящий Obsidian допускается только для read-only preview; apply/import запрещён до production-аудита.
- Все filesystem-тесты используют `TempDir` или отдельную синтетическую копию.
- Codex/Claude не получают настоящий запрос во время тестов.
- Readiness, package и submitted snapshot не считаются основанием для реальной сдачи, пока P0/P1 из аудита не закрыты.
