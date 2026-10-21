INSERT INTO schema_migrations(version) VALUES (4);

-- Этап 6: импорт экспорта CU LMS Enhancer и наблюдение за папкой загрузок.

-- Происхождение файла «Входящих»: ручной импорт, watcher загрузок или экспорт ЦУ.
-- Только имена и относительные пути внутри архива; абсолютных путей нет.
ALTER TABLE inbox_files ADD COLUMN provenance_json TEXT CHECK (
    provenance_json IS NULL OR json_valid(provenance_json)
);

CREATE TRIGGER inbox_files_provenance_relative_only_insert
BEFORE INSERT ON inbox_files
WHEN NEW.provenance_json IS NOT NULL AND EXISTS (
    SELECT 1 FROM json_tree(NEW.provenance_json)
    WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
        OR lower(value) LIKE 'file:%' OR value LIKE '%/../%' OR value LIKE '../%'
        OR value GLOB '[A-Za-z]:*'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'inbox provenance must not contain absolute paths');
END;

CREATE TRIGGER inbox_files_provenance_relative_only_update
BEFORE UPDATE OF provenance_json ON inbox_files
WHEN NEW.provenance_json IS NOT NULL AND EXISTS (
    SELECT 1 FROM json_tree(NEW.provenance_json)
    WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
        OR lower(value) LIKE 'file:%' OR value LIKE '%/../%' OR value LIKE '../%'
        OR value GLOB '[A-Za-z]:*'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'inbox provenance must not contain absolute paths');
END;

-- Необязательная пользовательская ссылка на страницу курса в LMS.
-- Allowlist домена проверяет backend; схема дополнительно требует HTTPS.
ALTER TABLE subjects ADD COLUMN course_url TEXT CHECK (
    course_url IS NULL OR (course_url LIKE 'https://%' AND length(course_url) <= 2048)
);

-- Файлы загрузок, уже импортированные или скрытые пользователем: watcher не
-- предлагает их повторно. Хранится только имя файла (без каталога) и SHA-256.
CREATE TABLE download_seen (
    sha256 TEXT PRIMARY KEY NOT NULL CHECK (
        length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    name TEXT NOT NULL CHECK (
        length(name) BETWEEN 1 AND 255 AND instr(name, '/') = 0 AND instr(name, '\') = 0
    ),
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('imported', 'dismissed')),
    operation_id TEXT REFERENCES operations(id) ON DELETE SET NULL,
    first_seen_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
