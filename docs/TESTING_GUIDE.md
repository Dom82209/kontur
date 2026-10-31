# Проверка «Контура»

Все автоматические файловые сценарии используют `tempfile::TempDir`. Они не должны указывать на домашнюю папку, Desktop, настоящий Obsidian vault или реальные учебные материалы.

## Workspace isolation (этап 14A)

Тесты `workspace::tests` (`src-tauri/src/workspace_tests.rs`) создают только временные vault: два пространства с одинаковыми относительными путями, отдельный demo, submitted receipt, запрет rebind и demo→real, перемещённый vault, копия vault, подменённая папка, повреждённый fingerprint, preview без записи, cancel/повтор/expiry, изменение vault между preview и confirm, rollback marker/БД/demo-каталога, symlink marker/vault/каталога баз, traversal путей БД, append-only каталог, restart и legacy v8 (без vault, с vault, старое демо, vault недоступен при upgrade).

Ручной smoke: «Настройки → Управляемые каталоги → Новое демо» показывает предпросмотр; «Отмена» ничего не меняет; «Подтвердить и перезапустить» перезапускает приложение в новом пространстве. Прямой замены активного vault в UI быть не должно.

На Linux 20 тестов `ai::*` с fake CLI через `sandbox-exec` падают по причине окружения (Seatbelt есть только в macOS); полный `cargo test` обязателен на Mac.

## Подключение vault (этап 2)

- `cargo test adoption` — 14 тестов на синтетическом vault в `TempDir`: read-only preview, идемпотентность, dry-run, изменение файла/базы после preview, symlink, ручные категории, конфликты, неизменяемость плана, эвристики на реальных именах.
- Проверка на структуре настоящего vault без чтения содержимого: сохранить список путей (`find "Предметы" -type f -printf '%y\t%p\n' > paths.tsv` внутри vault) и выполнить `KONTUR_MIRROR_PATHS=paths.tsv cargo test --lib mirror_of_real_vault -- --ignored --nocapture`. Тест строит зеркало с заглушками в `TempDir`, делает preview и dry-run; настоящий vault не открывается.

## Предметы (этап 3)

- `cargo test subjects::` — 11 тестов: нормализация и уникальность имён, ключевые слова без пересечений, ревизии, архив (классификация, ручной выбор, правила), слияние (предпросмотр без записи, перенос недель/ключевых слов/«Входящих», блокировка по одинаковым неделям и по изменениям после предпросмотра), перенос ключевых слов миграцией v12 и единая классификация «Входящих».
- `src/lib/bridge/mock/subjects.test.ts`, `src/components/domain/subjects-render.test.ts` — тот же контракт в mock и честный предпросмотр слияния.

## Несколько ДЗ (этап 4)

- `cargo test homework_registry` — три ДЗ одной недели, валидация, неизменяемость идентичности, срок в UTC (граница года), обновление схемы v12→v13 без потерь.
- `cargo test routing::` — раскладка в выбранное ДЗ, создание первого ДЗ с привязкой файла, ручной выбор ДЗ в классификации.
- `src/lib/bridge/mock/homeworks.test.ts` — тот же контракт в mock.

## Семестр и сроки (этап 5)

- `cargo test semester` — границы недель по местной полуночи (Москва, UTC−5), граница года, до/после семестра, неверный ввод, пересчёт дат недель с ревизией, точный/неизвестный срок, неизменяемость сданного ДЗ, «приблизительные» старые сроки, честная неделя на главной.
- `src/lib/bridge/mock/semester.test.ts`, `src/components/domain/semester-ui.test.ts` — контракт mock и преобразования времени в UI.

## Полный набор

```bash
cd src-tauri
cargo fmt --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test

cd ..
npm run typecheck
npm run test
npm run build
npm run tauri:build -- --debug --bundles app
```

Security-тесты с вложенным `/usr/bin/sandbox-exec` на macOS нужно запускать без дополнительной внешней sandbox-обёртки. Они используют только фиктивные локальные CLI и не делают AI-запросов.

## Матрица этапа 12

| Сценарий | Автоматические проверки |
|---|---|
| A — первый запуск и перезапуск | `intake_commands::tests::demo_workspace_is_created_only_under_temp_app_data`; `query_service::tests::scenario_a_file_backed_state_survives_restart`; `ai::tests::cancelling_the_run_stops_and_closes_its_jobs_and_restart_recovers_running_jobs` |
| B — импорт, классификация, preview, раскладка, Finder/Obsidian | `intake::tests::valid_import_copies_atomically_records_hash_and_reports_progress`; `routing::tests::manual_classification_has_priority_and_ambiguous_is_not_routed`; `routing::tests::preview_route_and_undo_are_real_and_notes_preserve_user_text` |
| C — версии, проверка, исправления | `workflow::service_tests::iteration_chain_links_versions_prompts_and_reviews_without_changing_old_iterations`; `workflow::service_tests::full_manual_cycle_visits_every_reachable_stage_through_backend_only` |
| D — комментарии и требования | `submission_commands::tests::scenario_d_transcript_preview_extracts_pairs_without_writing`; `workflow::service_tests::readiness_is_blocked_by_each_required_condition_separately`; `workflow::tests::full_hydration_of_existing_homework` |
| E — комплект и submitted snapshot | `submission::tests::package_and_submitted_snapshot_are_real_and_immutable`; repository-тесты неизменяемости receipt и submitted Homework |
| F — undo и повреждения | `file_service::tests::undo_rejects_changed_created_file`; `file_service::tests::repeated_undo_is_rejected`; `file_service::tests::undo_validates_backup_hash_and_relative_roots`; `file_service::tests::submitted_homework_blocks_undo`; race-тесты symlink и подмены каталогов |

## Крайние случаи

- Кириллица, пробелы, длинные имена и конфликтующие имена: `intake::tests::unicode_and_long_names_are_preserved_safely`, `file_service::race_tests::planning_keeps_conflict_numbering_and_cyrillic_extension`.
- Одинаковое содержимое: `intake::tests::duplicate_content_is_detected_and_not_copied_twice`, `file_service::race_tests::same_content_plan_does_not_create_copy`.
- Пустые, удалённые, повреждённые и слишком большие файлы: группы `extraction::tests`, `intake::tests::vanished_source_fails_without_side_effects`.
- Symlink/traversal/race: все `file_service::race_tests`, ZIP Slip и symlink-тесты `lms::tests`.
- Неизвестные enum, повреждённый JSON и старая схема: `workflow::tests::corrupted_json_and_unknown_enum_are_database_errors_without_panic`, `db::migrations::tests`.
- Повторные действия: operation-id replay, optimistic revision и backend mutex для route/undo/build/confirm; UI дополнительно использует синхронный in-flight guard.

## AI-безопасность

- Пользовательский `config.toml` недоступен из изоляции; монтируется только account-сессия Codex с `auth_mode=chatgpt` и без `OPENAI_API_KEY`.
- `ai::isolation::tests::api_key_credentials_are_never_mounted_as_account_subscription` запрещает подмену подписки API-ключом.
- `ai::tests::codex_automatic_mode_requires_credentials_proven_inside_isolation` запрещает ложный автоматический режим без доступной авторизации.
- Claude остаётся manual-only, пока не доказана изоляция settings/hooks/MCP.
- Ответ AI остаётся черновиком до явного подтверждения: `ai::tests::cli_response_waits_for_confirmation_and_then_applies_with_provenance`.
- Secret-like поля блокируются в настройках, а redaction-тесты проверяют логи. Настоящий AI-запрос в тестах запрещён.

## Ручная проверка окна

Используйте только созданное приложением demo workspace. Проверьте размеры 1180×740 и 1100×700, обе темы, reduced motion, клавиатурную навигацию, focus-visible, отсутствие горизонтального переполнения, состояния loading/empty/error/recovery и невозможность повторного запуска опасной операции. Реальные учебные папки не выбирайте.
