INSERT INTO schema_migrations(version) VALUES (13);

-- Production workflow, stage 4: a week has 0..N homeworks.
-- homeworks.week_id loses UNIQUE; (week_id, ordinal) is unique instead. import_key is
-- a stable external identity (vault folder, LMS assignment) for idempotent imports.
-- This migration runs with foreign_keys=OFF (12-step table rebuild): DROP TABLE under
-- foreign_keys=ON would cascade-delete files, reviews, comments and bundles.
-- The runner checks PRAGMA foreign_key_check before commit.

CREATE TABLE homeworks_v13 (
    id TEXT PRIMARY KEY NOT NULL,
    week_id TEXT NOT NULL REFERENCES weeks(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL DEFAULT 1 CHECK (ordinal >= 1),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    deadline TEXT,
    stage TEXT NOT NULL CHECK (stage IN ('task', 'in_progress', 'ready_to_submit', 'submitted')),
    workflow_stage TEXT NOT NULL CHECK (workflow_stage IN ('materials', 'analysis', 'prompt', 'solution', 'review', 'revision', 'teacher', 'ready_to_submit', 'submitted')),
    final_version_id TEXT REFERENCES homework_versions(id) DEFERRABLE INITIALLY DEFERRED,
    submitted_at TEXT,
    folder_path TEXT NOT NULL UNIQUE CHECK (folder_path NOT LIKE '/%' AND folder_path NOT LIKE '\\%'),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    final_file_name TEXT,
    assignment_url TEXT,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    import_key TEXT UNIQUE CHECK (import_key IS NULL OR length(trim(import_key)) BETWEEN 1 AND 300),
    CHECK ((stage = 'submitted') = (submitted_at IS NOT NULL)),
    UNIQUE (week_id, ordinal)
) STRICT;

-- Before v13 every week had at most one homework: it becomes ordinal 1.
INSERT INTO homeworks_v13(
    id, week_id, ordinal, title, deadline, stage, workflow_stage, final_version_id, submitted_at,
    folder_path, created_at, updated_at, final_file_name, assignment_url, revision, import_key
)
SELECT h.id, h.week_id, 1, h.title, h.deadline, h.stage, h.workflow_stage, h.final_version_id,
       h.submitted_at, h.folder_path, h.created_at, h.updated_at, h.final_file_name,
       h.assignment_url, h.revision,
       -- Homeworks created by vault adoption keep their vault folder as identity.
       CASE WHEN EXISTS (
           SELECT 1 FROM vault_adoption_items i
           WHERE i.entity_kind = 'homework' AND i.entity_id = h.id
       ) THEN 'vault:' || h.folder_path END
FROM homeworks h;

-- legacy_alter_table: the rename must not re-validate triggers of other tables that
-- reference homeworks while the old table is already dropped.
PRAGMA legacy_alter_table = ON;
DROP TABLE homeworks;
ALTER TABLE homeworks_v13 RENAME TO homeworks;
PRAGMA legacy_alter_table = OFF;

CREATE INDEX idx_homeworks_week_id ON homeworks(week_id);

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

CREATE TRIGGER homeworks_v11_deadline_not_blank
BEFORE INSERT ON homeworks
WHEN NEW.deadline IS NOT NULL AND length(trim(NEW.deadline)) = 0
BEGIN
    SELECT RAISE(ABORT, 'homework deadline must be a timestamp or NULL (unknown)');
END;

CREATE TRIGGER homeworks_v11_deadline_not_blank_update
BEFORE UPDATE OF deadline ON homeworks
WHEN NEW.deadline IS NOT NULL AND length(trim(NEW.deadline)) = 0
BEGIN
    SELECT RAISE(ABORT, 'homework deadline must be a timestamp or NULL (unknown)');
END;

CREATE TRIGGER homeworks_v13_identity_stable
BEFORE UPDATE OF week_id, ordinal, import_key ON homeworks
WHEN NEW.week_id IS NOT OLD.week_id OR NEW.ordinal IS NOT OLD.ordinal
  OR (OLD.import_key IS NOT NULL AND NEW.import_key IS NOT OLD.import_key)
BEGIN
    SELECT RAISE(ABORT, 'homework week, ordinal and import key are immutable');
END;
