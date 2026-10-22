INSERT INTO schema_migrations(version) VALUES (5);

-- Этап 7: локальное извлечение текста и структуры из учебных материалов.
-- Оригиналы не меняются: хранятся только нормализованные результаты и
-- диагностика, привязанные к SHA-256 содержимого источника.

-- Cache результатов. Ключ — содержимое (SHA-256), версия правил извлечения и
-- вариант интерпретации (формат по расширению, OCR для изображений).
CREATE TABLE extractions (
    id TEXT PRIMARY KEY NOT NULL,
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    extractor_version INTEGER NOT NULL CHECK (extractor_version > 0),
    options_key TEXT NOT NULL CHECK (length(options_key) BETWEEN 1 AND 32),
    format TEXT NOT NULL CHECK (format IN (
        'pdf', 'docx', 'xlsx', 'pptx', 'text', 'markdown', 'csv', 'tsv', 'notebook', 'image', 'unknown'
    )),
    status TEXT NOT NULL CHECK (status IN (
        'complete', 'partial', 'needs_ocr', 'unsupported', 'encrypted', 'corrupt',
        'rejected', 'too_large', 'timeout', 'failed'
    )),
    status_message TEXT CHECK (status_message IS NULL OR length(status_message) <= 2000),
    metadata_json TEXT NOT NULL CHECK (json_valid(metadata_json)),
    structure_json TEXT NOT NULL CHECK (json_valid(structure_json)),
    blocks_truncated INTEGER NOT NULL CHECK (blocks_truncated IN (0, 1)),
    text TEXT NOT NULL,
    text_chars INTEGER NOT NULL CHECK (text_chars >= 0),
    truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
    warnings_json TEXT NOT NULL CHECK (json_valid(warnings_json)),
    ocr_used INTEGER NOT NULL CHECK (ocr_used IN (0, 1)),
    source_size_bytes INTEGER NOT NULL CHECK (source_size_bytes >= 0),
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    created_at TEXT NOT NULL,
    UNIQUE (sha256, extractor_version, options_key)
) STRICT;

-- Какой результат относится к материалу. sha256 — содержимое на момент анализа:
-- если файл изменится, результат считается устаревшим.
CREATE TABLE material_extractions (
    material_kind TEXT NOT NULL CHECK (material_kind IN ('inbox', 'file')),
    material_id TEXT NOT NULL CHECK (length(material_id) BETWEEN 1 AND 128),
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    extraction_id TEXT REFERENCES extractions(id) ON DELETE SET NULL,
    analyzed_at TEXT NOT NULL,
    PRIMARY KEY (material_kind, material_id)
) STRICT;

CREATE INDEX idx_material_extractions_extraction ON material_extractions(extraction_id);

-- Итог пакетного анализа для идемпотентного повтора команды и журнала.
CREATE TABLE extraction_batches (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
    report_json TEXT NOT NULL CHECK (json_valid(report_json)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TRIGGER extraction_batches_relative_only
BEFORE INSERT ON extraction_batches
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.report_json)
    WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
        OR lower(value) LIKE 'file:%' OR value LIKE '%/../%'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'extraction report must not contain absolute paths');
END;

CREATE TRIGGER extraction_batches_no_update
BEFORE UPDATE ON extraction_batches
BEGIN
    SELECT RAISE(ABORT, 'extraction report is immutable');
END;

-- Явно выбранный пользователем набор материалов: только он позже передаётся
-- в анализ (SourceBundle). Подборка ДЗ или одна общая подборка без ДЗ.
CREATE TABLE corpus_selections (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT REFERENCES homeworks(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX idx_corpus_selections_homework
    ON corpus_selections(homework_id) WHERE homework_id IS NOT NULL;
CREATE UNIQUE INDEX idx_corpus_selections_general
    ON corpus_selections((homework_id IS NULL)) WHERE homework_id IS NULL;

CREATE TABLE corpus_selection_items (
    selection_id TEXT NOT NULL REFERENCES corpus_selections(id) ON DELETE CASCADE,
    material_kind TEXT NOT NULL CHECK (material_kind IN ('inbox', 'file')),
    material_id TEXT NOT NULL CHECK (length(material_id) BETWEEN 1 AND 128),
    -- Содержимое, которое пользователь выбрал; изменённый файл требует повторного выбора.
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    position INTEGER NOT NULL CHECK (position >= 0),
    added_at TEXT NOT NULL,
    PRIMARY KEY (selection_id, material_kind, material_id)
) STRICT;

CREATE TRIGGER submitted_homeworks_no_corpus_selection
BEFORE INSERT ON corpus_selections
WHEN NEW.homework_id IS NOT NULL
 AND EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive a corpus selection');
END;

CREATE TRIGGER submitted_homeworks_no_corpus_item_insert
BEFORE INSERT ON corpus_selection_items
WHEN EXISTS (
    SELECT 1 FROM corpus_selections s JOIN homeworks h ON h.id = s.homework_id
    WHERE s.id = NEW.selection_id AND h.stage = 'submitted'
)
BEGIN
    SELECT RAISE(ABORT, 'submitted homework corpus selection is immutable');
END;

CREATE TRIGGER submitted_homeworks_no_corpus_item_delete
BEFORE DELETE ON corpus_selection_items
WHEN EXISTS (
    SELECT 1 FROM corpus_selections s JOIN homeworks h ON h.id = s.homework_id
    WHERE s.id = OLD.selection_id AND h.stage = 'submitted'
)
BEGIN
    SELECT RAISE(ABORT, 'submitted homework corpus selection is immutable');
END;

-- Материал удалён — связь с результатом и выбор в подборках снимаются.
CREATE TRIGGER inbox_files_drop_extraction_links
AFTER DELETE ON inbox_files
BEGIN
    DELETE FROM material_extractions WHERE material_kind = 'inbox' AND material_id = OLD.id;
    DELETE FROM corpus_selection_items WHERE material_kind = 'inbox' AND material_id = OLD.id;
END;

CREATE TRIGGER files_drop_extraction_links
AFTER DELETE ON files
BEGIN
    DELETE FROM material_extractions WHERE material_kind = 'file' AND material_id = OLD.id;
    DELETE FROM corpus_selection_items WHERE material_kind = 'file' AND material_id = OLD.id;
END;
