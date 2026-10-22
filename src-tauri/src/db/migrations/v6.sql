INSERT INTO schema_migrations(version) VALUES (6);

-- Этап 8: постоянный рабочий цикл домашней работы.
-- Новых таблиц с назначением существующих не создаётся: source_bundles,
-- source_bundle_items, review_runs, revision_prompts и operation_results
-- дополняются или перестраиваются с переносом всех строк v1–v5.
-- Единственная новая таблица — style_confirmations (ручное подтверждение
-- расхождения с личным стилем), у неё нет аналога в старой схеме.

-- ───────────── Homework: optimistic revision агрегата ─────────────

ALTER TABLE homeworks ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0);

-- ───────────── SourceBundle: неизменяемый снимок подборки ─────────────

ALTER TABLE source_bundles ADD COLUMN origin TEXT NOT NULL DEFAULT 'legacy'
    CHECK (origin IN ('legacy', 'corpus_selection'));
ALTER TABLE source_bundles ADD COLUMN selection_id TEXT;
ALTER TABLE source_bundles ADD COLUMN extractor_version INTEGER
    CHECK (extractor_version IS NULL OR extractor_version > 0);
ALTER TABLE source_bundles ADD COLUMN warnings_json TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(warnings_json));
ALTER TABLE source_bundles ADD COLUMN total_chars INTEGER NOT NULL DEFAULT 0 CHECK (total_chars >= 0);
ALTER TABLE source_bundles ADD COLUMN limit_chars INTEGER NOT NULL DEFAULT 0 CHECK (limit_chars >= 0);

CREATE TRIGGER source_bundles_v6_insert_rules
BEFORE INSERT ON source_bundles
WHEN NEW.origin <> 'corpus_selection'
  OR NEW.selection_id IS NULL
  OR NEW.extractor_version IS NULL
  OR (NEW.version_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM homework_versions v WHERE v.id = NEW.version_id AND v.homework_id = NEW.homework_id
     ))
  OR EXISTS (
        SELECT 1 FROM json_tree(NEW.warnings_json) WHERE type = 'text' AND (
            value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
            OR lower(value) LIKE 'file:%' OR value LIKE '%/../%' OR value LIKE '../%'
        )
     )
  OR EXISTS (
        SELECT 1 FROM json_tree(NEW.missing_json) WHERE type = 'text' AND (
            value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
            OR lower(value) LIKE 'file:%' OR value LIKE '%/../%' OR value LIKE '../%'
        )
     )
BEGIN
    SELECT RAISE(ABORT, 'source bundle must be an immutable corpus snapshot of its homework');
END;

CREATE TRIGGER source_bundles_no_update
BEFORE UPDATE ON source_bundles
BEGIN
    SELECT RAISE(ABORT, 'source bundle is immutable');
END;

-- Элементы снимка не ссылаются на материал внешним ключом: материал можно
-- удалить или изменить, а снимок обязан остаться прежним. Имя, путь и SHA-256
-- фиксируются в самой строке.
CREATE TABLE source_bundle_items_v6 (
    id TEXT PRIMARY KEY NOT NULL,
    source_bundle_id TEXT NOT NULL REFERENCES source_bundles(id) ON DELETE CASCADE,
    material_kind TEXT NOT NULL CHECK (material_kind IN ('inbox', 'file')),
    material_id TEXT NOT NULL CHECK (length(material_id) BETWEEN 1 AND 128),
    source_kind TEXT NOT NULL DEFAULT 'extra' CHECK (source_kind IN ('lecture', 'seminar', 'task', 'data', 'extra', 'style', 'previous_work', 'current_version', 'previous_version', 'teacher_comments')),
    required INTEGER NOT NULL DEFAULT 0 CHECK (required IN (0, 1)),
    included INTEGER NOT NULL DEFAULT 1 CHECK (included IN (0, 1)),
    position INTEGER NOT NULL CHECK (position >= 0),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    file_kind TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    modified_at TEXT NOT NULL,
    -- Базовое ограничение совпадает с files.relative_path v1, чтобы перенос
    -- legacy-строк не терял данные; новые строки дополнительно проверяет триггер.
    relative_path TEXT NOT NULL CHECK (relative_path NOT LIKE '/%' AND relative_path NOT LIKE '\%'),
    sha256 TEXT CHECK (sha256 IS NULL OR (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*')),
    extraction_id TEXT,
    extractor_version INTEGER CHECK (extractor_version IS NULL OR extractor_version > 0),
    extraction_status TEXT,
    text_chars INTEGER NOT NULL DEFAULT 0 CHECK (text_chars >= 0),
    warnings_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(warnings_json)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(source_bundle_id, material_kind, material_id),
    UNIQUE(source_bundle_id, position)
) STRICT;

INSERT INTO source_bundle_items_v6(
    id, source_bundle_id, material_kind, material_id, source_kind, required, included, position,
    name, file_kind, size_bytes, modified_at, relative_path, sha256, created_at
)
SELECT i.id, i.source_bundle_id, 'file', i.file_id, i.source_kind, i.required, i.included,
       (SELECT count(*) FROM source_bundle_items x
        WHERE x.source_bundle_id = i.source_bundle_id
          AND (x.created_at < i.created_at OR (x.created_at = i.created_at AND x.id < i.id))),
       f.name, f.kind, f.size_bytes, coalesce(f.modified_at, f.updated_at, f.created_at),
       f.relative_path,
       CASE WHEN length(f.content_hash) = 64 AND f.content_hash NOT GLOB '*[^0-9a-f]*'
            THEN f.content_hash ELSE NULL END,
       i.created_at
FROM source_bundle_items i JOIN files f ON f.id = i.file_id;

DROP TABLE source_bundle_items;
ALTER TABLE source_bundle_items_v6 RENAME TO source_bundle_items;

CREATE INDEX idx_source_bundle_items_bundle_position ON source_bundle_items(source_bundle_id, position);

CREATE TRIGGER submitted_homeworks_no_new_source_item
BEFORE INSERT ON source_bundle_items
WHEN EXISTS (
    SELECT 1 FROM source_bundles sb
    JOIN homeworks h ON h.id = sb.homework_id
    WHERE sb.id = NEW.source_bundle_id AND h.stage = 'submitted'
)
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive source items');
END;

CREATE TRIGGER source_bundle_items_no_update
BEFORE UPDATE ON source_bundle_items
BEGIN
    SELECT RAISE(ABORT, 'source bundle item is immutable');
END;

-- ───────────── ReviewRun: честные состояния и отдельные критерии ─────────────

CREATE TABLE review_runs_v6 (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('analyze_materials', 'review_version', 'revision_prompt', 'review_teacher')),
    role TEXT NOT NULL CHECK (role IN ('analyst', 'author', 'reviewer')),
    provider TEXT NOT NULL CHECK (provider IN ('codex', 'claude')),
    version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    source_bundle_id TEXT REFERENCES source_bundles(id) ON DELETE RESTRICT,
    sources_used_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(sources_used_json)),
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'waiting_for_provider', 'manual_handoff', 'running', 'completed', 'failed', 'cancelled'
    )),
    progress INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    step TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at TEXT,
    saved_as_iteration INTEGER NOT NULL DEFAULT 0 CHECK (saved_as_iteration IN (0, 1)),
    provenance TEXT NOT NULL DEFAULT 'legacy' CHECK (provenance IN ('legacy', 'manual', 'provider')),
    verdict TEXT CHECK (verdict IS NULL OR verdict IN ('pass', 'needs_fixes', 'needs_teacher_review', 'blocked')),
    correctness TEXT CHECK (correctness IS NULL OR correctness IN ('pass', 'warn', 'fail', 'manual', 'na')),
    task_fit TEXT CHECK (task_fit IS NULL OR task_fit IN ('pass', 'warn', 'fail', 'manual', 'na')),
    course_grounding TEXT CHECK (course_grounding IS NULL OR course_grounding IN ('pass', 'warn', 'fail', 'manual', 'na')),
    teacher_status TEXT CHECK (teacher_status IS NULL OR teacher_status IN ('pass', 'warn', 'fail', 'manual', 'na')),
    missing_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(missing_json)),
    critical_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(critical_json)),
    unsupported_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(unsupported_json)),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    error_message TEXT,
    style_score INTEGER CHECK (style_score BETWEEN 0 AND 10),
    style_threshold INTEGER NOT NULL DEFAULT 4 CHECK (style_threshold BETWEEN 0 AND 10),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

INSERT INTO review_runs_v6(
    id, homework_id, kind, role, provider, version_id, source_bundle_id, sources_used_json,
    status, progress, step, started_at, finished_at, saved_as_iteration, provenance,
    result_json, error_message, style_score, style_threshold, created_at
)
SELECT id, homework_id, kind, role, provider, version_id, source_bundle_id, sources_used_json,
       status, progress, step, started_at, finished_at, saved_as_iteration, 'legacy',
       result_json, error_message, style_score, style_threshold, created_at
FROM review_runs;

CREATE TABLE revision_prompts_v6 (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('initial', 'revision', 'teacher')),
    text TEXT NOT NULL,
    created_by TEXT NOT NULL CHECK (created_by IN ('codex', 'claude', 'user')),
    provenance TEXT NOT NULL DEFAULT 'legacy' CHECK (provenance IN ('legacy', 'manual', 'provider')),
    status TEXT NOT NULL DEFAULT 'committed' CHECK (status IN ('draft', 'committed')),
    based_on_version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    review_run_id TEXT REFERENCES review_runs_v6(id) ON DELETE RESTRICT,
    addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(addresses_json)),
    sources_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(sources_json)),
    sent_at TEXT,
    committed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((status = 'committed') = (committed_at IS NOT NULL)),
    CHECK (sent_at IS NULL OR status = 'committed')
) STRICT;

-- Промты v1–v5 фиксировались сразу (draft-статуса не было): переносятся как committed.
INSERT INTO revision_prompts_v6(
    id, homework_id, kind, text, created_by, provenance, status, based_on_version_id, review_run_id,
    addresses_json, sources_json, sent_at, committed_at, created_at, updated_at
)
SELECT id, homework_id, kind, text, created_by, 'legacy', 'committed', based_on_version_id, review_run_id,
       addresses_json, sources_json, sent_at, created_at, created_at, updated_at
FROM revision_prompts;

DROP TABLE revision_prompts;
DROP TABLE review_runs;
ALTER TABLE review_runs_v6 RENAME TO review_runs;
ALTER TABLE revision_prompts_v6 RENAME TO revision_prompts;

-- Триггер элементов снимка ссылается на review_runs, поэтому создаётся после перестройки.
CREATE TRIGGER source_bundle_items_v6_insert_rules
BEFORE INSERT ON source_bundle_items
WHEN EXISTS (SELECT 1 FROM review_runs r WHERE r.source_bundle_id = NEW.source_bundle_id)
  OR NEW.sha256 IS NULL
  OR NEW.extractor_version IS NULL
  OR length(trim(NEW.relative_path)) = 0
  OR NEW.relative_path LIKE '~%' OR NEW.relative_path LIKE '../%'
  OR NEW.relative_path LIKE '%/../%' OR instr(NEW.relative_path, ':') > 0
  OR EXISTS (
        SELECT 1 FROM json_tree(NEW.warnings_json) WHERE type = 'text' AND (
            value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
            OR lower(value) LIKE 'file:%' OR value LIKE '%/../%'
        )
     )
BEGIN
    SELECT RAISE(ABORT, 'source bundle item must be a hashed snapshot of an unused bundle');
END;

CREATE INDEX idx_review_runs_homework_id ON review_runs(homework_id, created_at);
CREATE INDEX idx_review_runs_version_id ON review_runs(version_id);
CREATE INDEX idx_review_runs_bundle_id ON review_runs(source_bundle_id);
CREATE INDEX idx_revision_prompts_homework_id ON revision_prompts(homework_id, created_at);
CREATE INDEX idx_revision_prompts_review_run_id ON revision_prompts(review_run_id);
CREATE UNIQUE INDEX idx_revision_prompts_one_draft ON revision_prompts(homework_id) WHERE status = 'draft';

CREATE TRIGGER review_runs_v2_required_fields
BEFORE INSERT ON review_runs
WHEN NEW.source_bundle_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'review run source_bundle_id is required');
END;

CREATE TRIGGER review_runs_v6_insert_rules
BEFORE INSERT ON review_runs
WHEN NEW.provenance = 'legacy'
  OR NOT EXISTS (SELECT 1 FROM source_bundles b WHERE b.id = NEW.source_bundle_id AND b.homework_id = NEW.homework_id)
  OR (NEW.version_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM homework_versions v WHERE v.id = NEW.version_id AND v.homework_id = NEW.homework_id
     ))
BEGIN
    SELECT RAISE(ABORT, 'review run must reference a bundle and version of its homework');
END;

CREATE TRIGGER submitted_homeworks_no_new_review
BEFORE INSERT ON review_runs
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive reviews');
END;

-- Завершённый запуск неизменяем; разрешено только закрепить его в истории итераций.
CREATE TRIGGER review_runs_terminal_immutable
BEFORE UPDATE OF id, homework_id, kind, role, provider, version_id, source_bundle_id,
    sources_used_json, status, progress, step, started_at, finished_at, provenance, verdict,
    correctness, task_fit, course_grounding, teacher_status, missing_json, critical_json,
    unsupported_json, result_json, error_message, style_score, style_threshold, created_at
ON review_runs
WHEN OLD.status IN ('completed', 'failed', 'cancelled')
BEGIN
    SELECT RAISE(ABORT, 'finished review run is immutable');
END;

-- Применённый порог и ссылки запуска не меняются никогда.
CREATE TRIGGER review_runs_links_immutable
BEFORE UPDATE OF homework_id, kind, version_id, source_bundle_id, style_threshold, created_at ON review_runs
WHEN NEW.homework_id IS NOT OLD.homework_id OR NEW.kind IS NOT OLD.kind
  OR NEW.version_id IS NOT OLD.version_id OR NEW.source_bundle_id IS NOT OLD.source_bundle_id
  OR NEW.style_threshold IS NOT OLD.style_threshold OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'review run links and applied threshold are immutable');
END;

CREATE TRIGGER review_runs_saved_only_once
BEFORE UPDATE OF saved_as_iteration ON review_runs
WHEN OLD.saved_as_iteration = 1 AND NEW.saved_as_iteration = 0
BEGIN
    SELECT RAISE(ABORT, 'saved iteration cannot be unpinned');
END;

CREATE TRIGGER review_runs_no_delete_finished
BEFORE DELETE ON review_runs
WHEN OLD.status = 'completed'
 AND EXISTS (SELECT 1 FROM homeworks h WHERE h.id = OLD.homework_id)
BEGIN
    SELECT RAISE(ABORT, 'completed review run is immutable');
END;

CREATE TRIGGER review_runs_json_relative_only
BEFORE INSERT ON review_runs
WHEN EXISTS (
    SELECT 1 FROM json_tree(coalesce(NEW.result_json, '{}')) WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
        OR lower(value) LIKE 'file:%' OR value LIKE '%/../%'
    )
) OR EXISTS (
    SELECT 1 FROM json_tree(NEW.missing_json) WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%' OR lower(value) LIKE 'file:%'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'review run must not contain absolute paths');
END;

CREATE TRIGGER review_runs_json_relative_only_update
BEFORE UPDATE OF result_json, missing_json ON review_runs
WHEN EXISTS (
    SELECT 1 FROM json_tree(coalesce(NEW.result_json, '{}')) WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%'
        OR lower(value) LIKE 'file:%' OR value LIKE '%/../%'
    )
) OR EXISTS (
    SELECT 1 FROM json_tree(NEW.missing_json) WHERE type = 'text' AND (
        value LIKE '/%' OR value LIKE '\%' OR value LIKE '~/%' OR lower(value) LIKE 'file:%'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'review run must not contain absolute paths');
END;

-- ───────────── RevisionPrompt: draft → committed, потом неизменяем ─────────────

CREATE TRIGGER submitted_homeworks_no_new_prompt
BEFORE INSERT ON revision_prompts
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive prompts');
END;

CREATE TRIGGER revision_prompts_v6_insert_rules
BEFORE INSERT ON revision_prompts
WHEN NEW.provenance = 'legacy'
  OR NEW.status <> 'draft'
  OR (NEW.kind = 'revision' AND (NEW.review_run_id IS NULL OR NEW.based_on_version_id IS NULL))
  OR (NEW.review_run_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM review_runs r
        WHERE r.id = NEW.review_run_id AND r.homework_id = NEW.homework_id AND r.status = 'completed'
          AND (NEW.based_on_version_id IS NULL OR r.version_id IS NEW.based_on_version_id)
     ))
  OR (NEW.based_on_version_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM homework_versions v WHERE v.id = NEW.based_on_version_id AND v.homework_id = NEW.homework_id
     ))
BEGIN
    SELECT RAISE(ABORT, 'new prompt must start as a draft linked to its homework review and version');
END;

CREATE TRIGGER revision_prompts_committed_immutable
BEFORE UPDATE OF id, homework_id, kind, text, created_by, provenance, status, based_on_version_id,
    review_run_id, addresses_json, sources_json, committed_at, created_at
ON revision_prompts
WHEN OLD.status = 'committed'
BEGIN
    SELECT RAISE(ABORT, 'committed prompt is immutable');
END;

CREATE TRIGGER revision_prompts_links_immutable
BEFORE UPDATE OF homework_id, kind, based_on_version_id, review_run_id, created_at ON revision_prompts
WHEN NEW.homework_id IS NOT OLD.homework_id OR NEW.kind IS NOT OLD.kind
  OR NEW.based_on_version_id IS NOT OLD.based_on_version_id
  OR NEW.review_run_id IS NOT OLD.review_run_id OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'prompt links are immutable');
END;

CREATE TRIGGER revision_prompts_sent_once
BEFORE UPDATE OF sent_at ON revision_prompts
WHEN OLD.sent_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'prompt hand-off is recorded only once');
END;

CREATE TRIGGER revision_prompts_no_delete_committed
BEFORE DELETE ON revision_prompts
WHEN OLD.status = 'committed'
 AND EXISTS (SELECT 1 FROM homeworks h WHERE h.id = OLD.homework_id)
BEGIN
    SELECT RAISE(ABORT, 'committed prompt is immutable');
END;

-- ───────────── Ручное подтверждение расхождения с личным стилем ─────────────

CREATE TABLE style_confirmations (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    review_run_id TEXT NOT NULL UNIQUE REFERENCES review_runs(id) ON DELETE RESTRICT,
    score INTEGER NOT NULL CHECK (score BETWEEN 0 AND 10),
    threshold INTEGER NOT NULL CHECK (threshold BETWEEN 0 AND 10),
    confirmed_by TEXT NOT NULL CHECK (confirmed_by IN ('user')),
    confirmed_at TEXT NOT NULL,
    CHECK (score > threshold)
) STRICT;

CREATE INDEX idx_style_confirmations_homework_id ON style_confirmations(homework_id);

CREATE TRIGGER style_confirmations_insert_rules
BEFORE INSERT ON style_confirmations
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
  OR NOT EXISTS (
        SELECT 1 FROM review_runs r
        WHERE r.id = NEW.review_run_id AND r.homework_id = NEW.homework_id
          AND r.version_id = NEW.version_id AND r.status = 'completed'
          AND r.style_score = NEW.score AND r.style_threshold = NEW.threshold
     )
BEGIN
    SELECT RAISE(ABORT, 'style confirmation must match a completed review of the same version');
END;

CREATE TRIGGER style_confirmations_no_update
BEFORE UPDATE ON style_confirmations
BEGIN
    SELECT RAISE(ABORT, 'style confirmation is immutable');
END;

-- ───────────── Итоги операций: идемпотентный повтор команд рабочего цикла ─────────────

CREATE TABLE operation_results_v6 (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('inbox_import', 'workflow')),
    request_hash TEXT CHECK (request_hash IS NULL OR (length(request_hash) = 64 AND request_hash NOT GLOB '*[^0-9a-f]*')),
    result_json TEXT NOT NULL CHECK (json_valid(result_json)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (kind <> 'workflow' OR request_hash IS NOT NULL)
) STRICT;

INSERT INTO operation_results_v6(operation_id, kind, request_hash, result_json, created_at)
SELECT operation_id, kind, NULL, result_json, created_at FROM operation_results;

DROP TABLE operation_results;
ALTER TABLE operation_results_v6 RENAME TO operation_results;

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

-- ───────────── Индексы реальных запросов агрегата ─────────────

CREATE INDEX idx_operations_homework_id ON operations(homework_id, started_at);
CREATE INDEX idx_teacher_requirements_comment_id ON teacher_requirements(comment_id);
CREATE INDEX idx_files_homework_category ON files(homework_id, category);
