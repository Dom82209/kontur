-- Этап 9: профиль личного стиля с явно выбранными источниками и ревизиями.
--
-- Источники выбирает пользователь (никакого автоматического сканирования).
-- Профиль — неизменяемые ревизии с отпечатком источников, признаками,
-- ограничениями и статусом. Проверка версии — неизменяемая запись с
-- эвристической оценкой; это не «детектор ИИ» и не определение авторства.

CREATE TABLE style_sources (
    id TEXT PRIMARY KEY NOT NULL,
    relative_path TEXT NOT NULL CHECK (
        length(trim(relative_path)) > 0 AND
        relative_path NOT LIKE '/%' AND relative_path NOT LIKE '\%' AND
        relative_path NOT LIKE '%/../%' AND relative_path NOT LIKE '../%' AND
        instr(relative_path, ':') = 0
    ),
    file_id TEXT REFERENCES files(id) ON DELETE SET NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('style_markdown', 'previous_homework', 'confirmed_sample')),
    language TEXT NOT NULL CHECK (language IN ('ru', 'en')),
    document_type TEXT NOT NULL CHECK (document_type IN ('markdown', 'text', 'docx', 'pdf', 'other')),
    sha256 TEXT CHECK (sha256 IS NULL OR length(sha256) = 64),
    size_bytes INTEGER CHECK (size_bytes IS NULL OR size_bytes >= 0),
    confirmation TEXT NOT NULL DEFAULT 'confirmed' CHECK (confirmation IN ('confirmed', 'unconfirmed')),
    read_status TEXT NOT NULL DEFAULT 'unread'
        CHECK (read_status IN ('unread', 'ok', 'missing', 'unsupported', 'empty')),
    added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_read_at TEXT,
    removed_at TEXT
) STRICT;

CREATE UNIQUE INDEX style_sources_active_path ON style_sources(relative_path) WHERE removed_at IS NULL;

-- Перенос прежних явно выбранных источников (до этапа 9 они не читались).
INSERT INTO style_sources(id, relative_path, file_id, source_kind, language, document_type, added_at)
SELECT id, relative_path, file_id,
       CASE source_kind WHEN 'confirmed_final' THEN 'confirmed_sample' ELSE source_kind END,
       'ru',
       CASE
           WHEN lower(relative_path) LIKE '%.md' THEN 'markdown'
           WHEN lower(relative_path) LIKE '%.txt' THEN 'text'
           WHEN lower(relative_path) LIKE '%.docx' THEN 'docx'
           WHEN lower(relative_path) LIKE '%.pdf' THEN 'pdf'
           ELSE 'other'
       END,
       created_at
FROM style_profile_sources;

CREATE TRIGGER style_sources_identity_stable
BEFORE UPDATE OF id, relative_path, source_kind, added_at ON style_sources
BEGIN
    SELECT RAISE(ABORT, 'style source identity is immutable');
END;

CREATE TRIGGER style_sources_removed_final
BEFORE UPDATE OF removed_at ON style_sources
WHEN OLD.removed_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'removed style source stays removed');
END;

CREATE TRIGGER style_sources_no_delete
BEFORE DELETE ON style_sources
BEGIN
    SELECT RAISE(ABORT, 'style sources are kept for history');
END;

CREATE TABLE style_profile_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL DEFAULT 'personal',
    revision INTEGER NOT NULL CHECK (revision >= 1),
    language TEXT NOT NULL CHECK (language IN ('ru', 'en')),
    source_fingerprint TEXT NOT NULL CHECK (length(source_fingerprint) = 64),
    sources_json TEXT NOT NULL CHECK (json_valid(sources_json)),
    features_json TEXT CHECK (features_json IS NULL OR json_valid(features_json)),
    limitations_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(limitations_json)),
    status TEXT NOT NULL CHECK (status IN ('ready', 'insufficient_sources', 'unsupported')),
    unusable_reasons_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(unusable_reasons_json)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (profile_id, revision),
    CHECK ((status = 'ready') = (features_json IS NOT NULL))
) STRICT;

CREATE TRIGGER style_profile_revisions_sequence
BEFORE INSERT ON style_profile_revisions
WHEN NEW.revision <> coalesce(
    (SELECT max(revision) FROM style_profile_revisions WHERE profile_id = NEW.profile_id), 0) + 1
BEGIN
    SELECT RAISE(ABORT, 'style profile revisions are sequential');
END;

CREATE TRIGGER style_profile_revisions_immutable
BEFORE UPDATE ON style_profile_revisions
BEGIN
    SELECT RAISE(ABORT, 'style profile revision is immutable');
END;

CREATE TRIGGER style_profile_revisions_no_delete
BEFORE DELETE ON style_profile_revisions
BEGIN
    SELECT RAISE(ABORT, 'style profile revisions are kept for history');
END;

CREATE TABLE style_checks (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    profile_revision_id TEXT REFERENCES style_profile_revisions(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('checked', 'not_checked', 'insufficient_sources', 'unsupported', 'manual_required')),
    score INTEGER CHECK (score IS NULL OR score BETWEEN 0 AND 10),
    threshold INTEGER NOT NULL CHECK (threshold BETWEEN 0 AND 10),
    result_json TEXT NOT NULL CHECK (json_valid(result_json)),
    version_fingerprint TEXT NOT NULL,
    inputs_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((status = 'checked') = (score IS NOT NULL))
) STRICT;

CREATE INDEX style_checks_version ON style_checks(version_id, created_at);

CREATE TRIGGER style_checks_immutable
BEFORE UPDATE ON style_checks
BEGIN
    SELECT RAISE(ABORT, 'style check is immutable');
END;

CREATE TRIGGER style_checks_submitted_homework
BEFORE INSERT ON style_checks
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework is immutable');
END;

-- Ручное подтверждение стиля: расхождение выше порога или проверка невозможна.
CREATE TABLE style_check_confirmations (
    id TEXT PRIMARY KEY NOT NULL,
    style_check_id TEXT NOT NULL UNIQUE REFERENCES style_checks(id) ON DELETE CASCADE,
    note TEXT NOT NULL DEFAULT '',
    confirmed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TRIGGER style_check_confirmations_immutable
BEFORE UPDATE ON style_check_confirmations
BEGIN
    SELECT RAISE(ABORT, 'style confirmation is immutable');
END;
