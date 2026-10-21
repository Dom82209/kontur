INSERT INTO schema_migrations(version) VALUES (2);

ALTER TABLE subjects ADD COLUMN short_name TEXT NOT NULL DEFAULT '';
UPDATE subjects SET short_name = name WHERE short_name = '';

ALTER TABLE weeks ADD COLUMN starts_at TEXT NOT NULL DEFAULT '';
ALTER TABLE weeks ADD COLUMN ends_at TEXT NOT NULL DEFAULT '';
ALTER TABLE weeks ADD COLUMN topic TEXT;
UPDATE weeks SET starts_at = created_at, ends_at = created_at WHERE starts_at = '' OR ends_at = '';

ALTER TABLE homeworks ADD COLUMN final_file_name TEXT;
ALTER TABLE homeworks ADD COLUMN assignment_url TEXT;

ALTER TABLE homework_versions ADD COLUMN file_name TEXT NOT NULL DEFAULT '';
ALTER TABLE homework_versions ADD COLUMN source_file_id TEXT NOT NULL DEFAULT '';
ALTER TABLE homework_versions ADD COLUMN note TEXT;
ALTER TABLE homework_versions ADD COLUMN sections_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(sections_json));
ALTER TABLE homework_versions ADD COLUMN prompt_id TEXT;
ALTER TABLE homework_versions ADD COLUMN source_bundle_id TEXT;
ALTER TABLE homework_versions ADD COLUMN files_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(files_json));
UPDATE homework_versions
SET file_name = CASE
    WHEN instr(file_path, '/') = 0 THEN file_path
    ELSE replace(file_path, rtrim(file_path, replace(file_path, '/', '')), '')
END
WHERE file_name = '';

CREATE TABLE source_bundles_v2 (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    purpose TEXT NOT NULL CHECK (purpose IN ('analyze_materials', 'review_version', 'revision_prompt', 'review_teacher')),
    version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    complete INTEGER NOT NULL DEFAULT 0 CHECK (complete IN (0, 1)),
    missing_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(missing_json)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

INSERT INTO source_bundles_v2(id, homework_id, purpose, version_id, complete, missing_json, created_at, updated_at)
SELECT id, homework_id, 'review_version', version_id,
       CASE status WHEN 'ready' THEN 1 ELSE 0 END,
       '[]', created_at, updated_at
FROM source_bundles;

CREATE TABLE source_bundle_items_v2 (
    id TEXT PRIMARY KEY NOT NULL,
    source_bundle_id TEXT NOT NULL REFERENCES source_bundles_v2(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES files(id) ON DELETE RESTRICT,
    source_kind TEXT NOT NULL DEFAULT 'extra' CHECK (source_kind IN ('lecture', 'seminar', 'task', 'data', 'extra', 'style', 'previous_work', 'current_version', 'previous_version', 'teacher_comments')),
    required INTEGER NOT NULL DEFAULT 0 CHECK (required IN (0, 1)),
    included INTEGER NOT NULL DEFAULT 1 CHECK (included IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(source_bundle_id, file_id)
) STRICT;

INSERT INTO source_bundle_items_v2(id, source_bundle_id, file_id, required, included, created_at)
SELECT id, source_bundle_id, file_id, required, included, created_at FROM source_bundle_items;

DROP TABLE source_bundle_items;
DROP TABLE source_bundles;
ALTER TABLE source_bundles_v2 RENAME TO source_bundles;
ALTER TABLE source_bundle_items_v2 RENAME TO source_bundle_items;

CREATE INDEX idx_source_bundles_homework_id ON source_bundles(homework_id);
CREATE INDEX idx_source_bundles_version_id ON source_bundles(version_id);
CREATE INDEX idx_source_bundle_items_bundle_id ON source_bundle_items(source_bundle_id);

CREATE TABLE review_runs_v2 (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('analyze_materials', 'review_version', 'revision_prompt', 'review_teacher')),
    role TEXT NOT NULL CHECK (role IN ('analyst', 'author', 'reviewer')),
    provider TEXT NOT NULL CHECK (provider IN ('codex', 'claude')),
    version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    source_bundle_id TEXT REFERENCES source_bundles(id) ON DELETE RESTRICT,
    sources_used_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(sources_used_json)),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    progress INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    step TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at TEXT,
    saved_as_iteration INTEGER NOT NULL DEFAULT 0 CHECK (saved_as_iteration IN (0, 1)),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    error_message TEXT,
    style_score INTEGER CHECK (style_score BETWEEN 0 AND 10),
    style_threshold INTEGER NOT NULL DEFAULT 4 CHECK (style_threshold BETWEEN 0 AND 10),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

INSERT INTO review_runs_v2(
    id, homework_id, kind, role, provider, version_id, status, started_at, finished_at,
    result_json, style_score, style_threshold, created_at
)
SELECT id, homework_id, 'review_version', 'reviewer', provider, version_id, status,
       coalesce(started_at, created_at), completed_at, result_json, style_score, style_threshold, created_at
FROM review_runs;

CREATE TABLE revision_prompts_v2 (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('initial', 'revision', 'teacher')),
    text TEXT NOT NULL,
    created_by TEXT NOT NULL CHECK (created_by IN ('codex', 'claude', 'user')),
    based_on_version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    review_run_id TEXT REFERENCES review_runs_v2(id) ON DELETE RESTRICT,
    addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(addresses_json)),
    sources_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(sources_json)),
    sent_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

INSERT INTO revision_prompts_v2(id, homework_id, kind, text, created_by, review_run_id, sent_at, created_at, updated_at)
SELECT id, homework_id, 'revision', content, 'user', review_run_id, delivered_at, created_at, updated_at
FROM revision_prompts;

DROP TABLE revision_prompts;
DROP TABLE review_runs;
ALTER TABLE review_runs_v2 RENAME TO review_runs;
ALTER TABLE revision_prompts_v2 RENAME TO revision_prompts;
CREATE INDEX idx_review_runs_homework_id ON review_runs(homework_id, created_at);
CREATE INDEX idx_review_runs_version_id ON review_runs(version_id);
CREATE INDEX idx_revision_prompts_homework_id ON revision_prompts(homework_id);

ALTER TABLE teacher_comments ADD COLUMN date TEXT;
ALTER TABLE teacher_comments ADD COLUMN added_at TEXT;
UPDATE teacher_comments SET date = coalesce(source_date, created_at), added_at = created_at;

CREATE TABLE teacher_requirements_v2 (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    comment_id TEXT REFERENCES teacher_comments(id) ON DELETE SET NULL,
    text TEXT NOT NULL,
    origin TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK (status IN ('done', 'not_done', 'manual', 'na')),
    note TEXT NOT NULL DEFAULT '',
    checked_version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

INSERT INTO teacher_requirements_v2(
    id, homework_id, comment_id, text, status, checked_version_id, created_at, updated_at
)
SELECT tr.id, tr.homework_id, tr.comment_id, tr.text,
       CASE tr.status
           WHEN 'met' THEN 'done'
           WHEN 'unmet' THEN 'not_done'
           WHEN 'manual_check' THEN 'manual'
           ELSE 'na'
       END,
       (SELECT hv.id FROM homework_versions hv WHERE hv.homework_id = tr.homework_id ORDER BY hv.number DESC LIMIT 1),
       tr.created_at, tr.updated_at
FROM teacher_requirements tr
;

DROP TABLE teacher_requirements;
ALTER TABLE teacher_requirements_v2 RENAME TO teacher_requirements;
CREATE INDEX idx_teacher_requirements_homework_id ON teacher_requirements(homework_id);

ALTER TABLE submission_packages ADD COLUMN items_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(items_json));
ALTER TABLE submission_packages ADD COLUMN links_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(links_json));
ALTER TABLE submission_packages ADD COLUMN warnings_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(warnings_json));
ALTER TABLE submission_packages ADD COLUMN upload_list_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(upload_list_json));
ALTER TABLE submission_packages ADD COLUMN built_at TEXT;
UPDATE submission_packages SET built_at = created_at WHERE built_at IS NULL;

ALTER TABLE submission_receipts ADD COLUMN submitted_at TEXT;
ALTER TABLE submission_receipts ADD COLUMN assignment_url TEXT NOT NULL DEFAULT '';
ALTER TABLE submission_receipts ADD COLUMN proof_json TEXT CHECK (proof_json IS NULL OR json_valid(proof_json));
ALTER TABLE submission_receipts ADD COLUMN files_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(files_json));
UPDATE submission_receipts SET submitted_at = confirmed_at WHERE submitted_at IS NULL;

ALTER TABLE operations ADD COLUMN title TEXT NOT NULL DEFAULT '';
ALTER TABLE operations ADD COLUMN detail TEXT;
ALTER TABLE operations ADD COLUMN paths_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(paths_json));
ALTER TABLE operations ADD COLUMN backup_path TEXT;
ALTER TABLE operations ADD COLUMN homework_id TEXT REFERENCES homeworks(id) ON DELETE RESTRICT;

CREATE TABLE inbox_files (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('pdf', 'doc', 'sheet', 'slides', 'image', 'notebook', 'text', 'archive', 'other')),
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    added_at TEXT NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('downloads', 'drop', 'lms')),
    status TEXT NOT NULL CHECK (status IN ('new', 'classifying', 'classified', 'ambiguous', 'routed')),
    relative_path TEXT NOT NULL UNIQUE CHECK (
        relative_path LIKE 'Предметы/00 Входящие/%'
        AND relative_path NOT LIKE '%/../%'
        AND instr(relative_path, ':') = 0
    ),
    classification_json TEXT CHECK (classification_json IS NULL OR json_valid(classification_json)),
    duplicate_json TEXT CHECK (duplicate_json IS NULL OR json_valid(duplicate_json)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE INDEX idx_inbox_files_status ON inbox_files(status, added_at);

CREATE TRIGGER subjects_v2_required_fields
BEFORE INSERT ON subjects
WHEN length(trim(NEW.short_name)) = 0
BEGIN
    SELECT RAISE(ABORT, 'subject short_name is required');
END;

CREATE TRIGGER weeks_v2_required_fields
BEFORE INSERT ON weeks
WHEN length(trim(NEW.starts_at)) = 0 OR length(trim(NEW.ends_at)) = 0
BEGIN
    SELECT RAISE(ABORT, 'week dates are required');
END;

CREATE TRIGGER homeworks_v2_required_fields
BEFORE INSERT ON homeworks
WHEN NEW.deadline IS NULL OR length(trim(NEW.deadline)) = 0
BEGIN
    SELECT RAISE(ABORT, 'homework deadline is required');
END;

CREATE TRIGGER homework_versions_v2_required_fields
BEFORE INSERT ON homework_versions
WHEN length(trim(NEW.file_name)) = 0 OR length(trim(NEW.source_file_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'homework version file_name and source_file_id are required');
END;

CREATE TRIGGER review_runs_v2_required_fields
BEFORE INSERT ON review_runs
WHEN NEW.source_bundle_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'review run source_bundle_id is required');
END;

CREATE TRIGGER teacher_comments_v2_required_fields
BEFORE INSERT ON teacher_comments
WHEN NEW.source NOT IN ('chat', 'email', 'lms', 'in_person', 'other')
  OR NEW.date IS NULL OR NEW.added_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'teacher comment source, date, and added_at are required');
END;

CREATE TRIGGER teacher_requirements_v2_required_fields
BEFORE INSERT ON teacher_requirements
WHEN NEW.comment_id IS NULL OR NEW.checked_version_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'teacher requirement comment_id and checked_version_id are required');
END;

CREATE TRIGGER submission_packages_v2_required_fields
BEFORE INSERT ON submission_packages
WHEN NEW.built_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'submission package built_at is required');
END;

CREATE TRIGGER submission_receipts_v2_required_fields
BEFORE INSERT ON submission_receipts
WHEN NEW.submitted_at IS NULL OR length(trim(NEW.assignment_url)) = 0
BEGIN
    SELECT RAISE(ABORT, 'submission receipt submitted_at and assignment_url are required');
END;

CREATE TRIGGER operations_relative_paths_insert
BEFORE INSERT ON operations
WHEN (NEW.backup_path IS NOT NULL AND (
        NEW.backup_path LIKE '/%' OR NEW.backup_path LIKE '\%' OR
        instr(NEW.backup_path, ':') > 0 OR NEW.backup_path LIKE '%/../%'
    ))
    OR EXISTS (
        SELECT 1 FROM json_each(NEW.paths_json)
        WHERE type = 'text' AND (
            value LIKE '/%' OR value LIKE '\%' OR instr(value, ':') > 0 OR value LIKE '%/../%'
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'operation paths must be relative');
END;

CREATE TRIGGER operations_relative_paths_update
BEFORE UPDATE OF paths_json, backup_path ON operations
WHEN (NEW.backup_path IS NOT NULL AND (
        NEW.backup_path LIKE '/%' OR NEW.backup_path LIKE '\%' OR
        instr(NEW.backup_path, ':') > 0 OR NEW.backup_path LIKE '%/../%'
    ))
    OR EXISTS (
        SELECT 1 FROM json_each(NEW.paths_json)
        WHERE type = 'text' AND (
            value LIKE '/%' OR value LIKE '\%' OR instr(value, ':') > 0 OR value LIKE '%/../%'
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'operation paths must be relative');
END;

CREATE TRIGGER homework_versions_no_update
BEFORE UPDATE ON homework_versions
BEGIN
    SELECT RAISE(ABORT, 'homework_version is immutable');
END;

CREATE TRIGGER homework_versions_no_delete
BEFORE DELETE ON homework_versions
BEGIN
    SELECT RAISE(ABORT, 'homework_version is immutable');
END;

CREATE TRIGGER submission_receipts_no_update
BEFORE UPDATE ON submission_receipts
BEGIN
    SELECT RAISE(ABORT, 'submission_receipt is immutable');
END;

CREATE TRIGGER submission_receipts_no_delete
BEFORE DELETE ON submission_receipts
BEGIN
    SELECT RAISE(ABORT, 'submission_receipt is immutable');
END;

CREATE TRIGGER submitted_homeworks_no_update
BEFORE UPDATE ON homeworks
WHEN OLD.stage = 'submitted'
BEGIN
    SELECT RAISE(ABORT, 'submitted homework is immutable');
END;

CREATE TRIGGER submitted_homeworks_no_delete
BEFORE DELETE ON homeworks
WHEN OLD.stage = 'submitted'
BEGIN
    SELECT RAISE(ABORT, 'submitted homework is immutable');
END;

CREATE TRIGGER submitted_homeworks_no_new_version
BEFORE INSERT ON homework_versions
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive versions');
END;

CREATE TRIGGER submitted_homeworks_no_new_file
BEFORE INSERT ON files
WHEN NEW.homework_id IS NOT NULL
 AND EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive files');
END;

CREATE TRIGGER submitted_homeworks_no_new_workflow_data
BEFORE INSERT ON source_bundles
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive workflow data');
END;

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

CREATE TRIGGER submitted_homeworks_no_new_review
BEFORE INSERT ON review_runs
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive reviews');
END;

CREATE TRIGGER submitted_homeworks_no_new_prompt
BEFORE INSERT ON revision_prompts
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive prompts');
END;

CREATE TRIGGER submitted_homeworks_no_new_comment
BEFORE INSERT ON teacher_comments
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive teacher comments');
END;

CREATE TRIGGER submitted_homeworks_no_new_requirement
BEFORE INSERT ON teacher_requirements
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive teacher requirements');
END;

CREATE TRIGGER submitted_homeworks_no_new_package
BEFORE INSERT ON submission_packages
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive submission packages');
END;
