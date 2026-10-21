INSERT INTO schema_migrations(version) VALUES (3);

-- Этап 5: реальный импорт во «Входящие».
-- SHA-256 содержимого и MIME/расширение сохраняются для дедупликации и идемпотентности.
-- Legacy-строки v2 остаются с NULL: их содержимое не хэшировалось.
ALTER TABLE inbox_files ADD COLUMN sha256 TEXT CHECK (
    sha256 IS NULL OR (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*')
);
ALTER TABLE inbox_files ADD COLUMN mime_type TEXT CHECK (
    mime_type IS NULL OR (length(mime_type) BETWEEN 3 AND 127 AND instr(mime_type, '/') > 1)
);
ALTER TABLE inbox_files ADD COLUMN import_operation_id TEXT REFERENCES operations(id) ON DELETE SET NULL;

-- Одно и то же содержимое не может дважды лежать во «Входящих» (до раскладки).
CREATE UNIQUE INDEX idx_inbox_files_active_sha256
    ON inbox_files(sha256)
    WHERE sha256 IS NOT NULL AND status <> 'routed';

-- Итог файловой операции (например, отчёт импорта) для повторного чтения и
-- идемпотентного повтора команды с тем же operation id.
CREATE TABLE operation_results (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('inbox_import')),
    result_json TEXT NOT NULL CHECK (json_valid(result_json)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

-- В отчёте не должно быть абсолютных путей, file:// и home-путей.
CREATE TRIGGER operation_results_relative_only
BEFORE INSERT ON operation_results
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.result_json)
    WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
        OR lower(value) LIKE 'file:%' OR value LIKE '%/../%'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'operation result must not contain absolute paths');
END;

CREATE TRIGGER operation_results_no_update
BEFORE UPDATE ON operation_results
BEGIN
    SELECT RAISE(ABORT, 'operation result is immutable');
END;
