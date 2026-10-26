# Release checklist

## Граница релиза

- [x] Ветка не `main`; история не переписывалась.
- [x] Schema version и миграции задокументированы.
- [x] Production runtime всегда использует `TauriDesktopBridge`; browser mock включается только `VITE_KONTUR_BRIDGE=demo`.
- [x] API-key поля и API billing отсутствуют.
- [x] Claude automatic mode выключен до доказанной изоляции settings/hooks/MCP.
- [x] Отправка в LMS остаётся ручной.

## Автоматические проверки

- [x] `cargo fmt --check`
- [x] `cargo check`
- [x] `cargo clippy --all-targets --all-features -- -D warnings`
- [x] `cargo test` — 382 passed, 1 ignored (все файловые fixtures — TempDir; security-тесты macOS запущены вне внешней вложенной песочницы)
- [x] `npm run typecheck`
- [x] `npm run test` — 124 passed
- [x] `npm run build`
- [x] `npm run tauri:build -- --debug --bundles app`
- [x] `git diff --check`
- [x] Рабочее дерево чистое после release-коммита (проверяется после создания коммита).

## Ручная macOS-проверка — только demo workspace

- [x] Первый запуск и создание demo workspace.
- [x] Диагностика выбранных папок.
- [x] Импорт тестовых файлов, классификация, preview и раскладка.
- [x] Управляемые блоки Obsidian не меняют пользовательский текст (проверено тестом и содержимым demo-заметок).
- [x] Создание версии ДЗ и выбор итоговой.
- [x] Сборка комплекта; ручная отправка намеренно не выполнялась, receipt не создан.
- [x] Перезапуск сохраняет состояние, версию, проверку и собранный комплект.
- [x] Экспорт диагностики не содержит содержимое, пути или secrets (Rust regression-тест).
- [ ] 1180×740 и 1100×700; light/dark; reduced motion; keyboard/focus-visible.
- [ ] Нет horizontal overflow, console errors или terminal errors.

Фактический smoke-прогон 27 октября 2026 выполнен только в app-created demo workspace. Кнопка Obsidian проходит backend-проверку зарегистрированной managed note и формирует URI; отдельный demo vault не регистрировался в пользовательском Obsidian, поэтому визуальное открытие именно этого vault не отмечено как пройденное. Настоящий AI-запрос и подтверждение отправки не выполнялись.

## Не входит без отдельного разрешения

- DMG;
- signing и notarization;
- публикация;
- auto-update;
- настоящий AI-запрос;
- реальные учебные материалы пользователя.
