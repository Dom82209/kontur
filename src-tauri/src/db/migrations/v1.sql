CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

INSERT INTO schema_migrations(version) VALUES (1);

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE subjects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE CHECK (length(trim(name)) > 0),
    folder_path TEXT NOT NULL UNIQUE CHECK (folder_path NOT LIKE '/%' AND folder_path NOT LIKE '\\%'),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE weeks (
    id TEXT PRIMARY KEY NOT NULL,
    subject_id TEXT NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    number INTEGER NOT NULL CHECK (number > 0),
    folder_path TEXT NOT NULL UNIQUE CHECK (folder_path NOT LIKE '/%' AND folder_path NOT LIKE '\\%'),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(subject_id, number)
) STRICT;

CREATE TABLE homeworks (
    id TEXT PRIMARY KEY NOT NULL,
    week_id TEXT NOT NULL UNIQUE REFERENCES weeks(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    deadline TEXT,
    stage TEXT NOT NULL CHECK (stage IN ('task', 'in_progress', 'ready_to_submit', 'submitted')),
    workflow_stage TEXT NOT NULL CHECK (workflow_stage IN ('materials', 'analysis', 'prompt', 'solution', 'review', 'revision', 'teacher', 'ready_to_submit', 'submitted')),
    final_version_id TEXT REFERENCES homework_versions(id) DEFERRABLE INITIALLY DEFERRED,
    submitted_at TEXT,
    folder_path TEXT NOT NULL UNIQUE CHECK (folder_path NOT LIKE '/%' AND folder_path NOT LIKE '\\%'),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((stage = 'submitted') = (submitted_at IS NOT NULL))
) STRICT;

CREATE TABLE files (
    id TEXT PRIMARY KEY NOT NULL,
    week_id TEXT REFERENCES weeks(id) ON DELETE CASCADE,
    homework_id TEXT REFERENCES homeworks(id) ON DELETE CASCADE,
    category TEXT NOT NULL CHECK (category IN ('lecture', 'seminar', 'material', 'homework_task', 'homework_data', 'homework_draft', 'homework_final', 'submission_attachment')),
    status TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('available', 'missing', 'excluded', 'archived')),
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    modified_at TEXT,
    relative_path TEXT NOT NULL UNIQUE CHECK (relative_path NOT LIKE '/%' AND relative_path NOT LIKE '\\%'),
    content_hash TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (week_id IS NOT NULL OR homework_id IS NOT NULL)
) STRICT;

CREATE TABLE homework_versions (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE RESTRICT,
    number INTEGER NOT NULL CHECK (number > 0),
    label TEXT NOT NULL CHECK (length(trim(label)) > 0),
    file_path TEXT NOT NULL UNIQUE CHECK (file_path NOT LIKE '/%' AND file_path NOT LIKE '\\%'),
    size_bytes INTEGER NOT NULL DEFAULT 0 CHECK (size_bytes >= 0),
    author TEXT NOT NULL CHECK (length(trim(author)) > 0),
    metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
    frozen INTEGER NOT NULL DEFAULT 1 CHECK (frozen = 1),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(homework_id, number),
    UNIQUE(homework_id, label)
) STRICT;

CREATE TABLE source_bundles (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('draft', 'ready', 'invalid')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE source_bundle_items (
    id TEXT PRIMARY KEY NOT NULL,
    source_bundle_id TEXT NOT NULL REFERENCES source_bundles(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES files(id) ON DELETE RESTRICT,
    required INTEGER NOT NULL DEFAULT 0 CHECK (required IN (0, 1)),
    included INTEGER NOT NULL DEFAULT 1 CHECK (included IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(source_bundle_id, file_id)
) STRICT;

CREATE TABLE review_runs (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    provider TEXT NOT NULL CHECK (provider IN ('codex', 'claude')),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    verdict TEXT,
    style_score INTEGER CHECK (style_score BETWEEN 0 AND 10),
    style_threshold INTEGER NOT NULL CHECK (style_threshold BETWEEN 0 AND 10),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE revision_prompts (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    review_run_id TEXT NOT NULL REFERENCES review_runs(id) ON DELETE RESTRICT,
    content TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('draft', 'locked', 'delivered')),
    delivered_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE teacher_comments (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    source TEXT,
    source_date TEXT,
    attachment_file_id TEXT REFERENCES files(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE teacher_requirements (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    comment_id TEXT REFERENCES teacher_comments(id) ON DELETE SET NULL,
    text TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('met', 'unmet', 'manual_check', 'not_applicable')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE submission_packages (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL UNIQUE REFERENCES homeworks(id) ON DELETE RESTRICT,
    final_version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('draft', 'ready', 'submitted')),
    package_path TEXT NOT NULL UNIQUE CHECK (package_path NOT LIKE '/%' AND package_path NOT LIKE '\\%'),
    checklist_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(checklist_json)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE submission_receipts (
    id TEXT PRIMARY KEY NOT NULL,
    package_id TEXT NOT NULL UNIQUE REFERENCES submission_packages(id) ON DELETE RESTRICT,
    homework_id TEXT NOT NULL UNIQUE REFERENCES homeworks(id) ON DELETE RESTRICT,
    submitted_version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    snapshot_path TEXT NOT NULL UNIQUE CHECK (snapshot_path NOT LIKE '/%' AND snapshot_path NOT LIKE '\\%'),
    snapshot_hash TEXT NOT NULL CHECK (length(trim(snapshot_hash)) > 0),
    confirmed_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE operations (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('started', 'running', 'completed', 'failed', 'cancelled', 'undone')),
    progress INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    undoable INTEGER NOT NULL DEFAULT 0 CHECK (undoable IN (0, 1)),
    undo_payload_json TEXT CHECK (undo_payload_json IS NULL OR json_valid(undo_payload_json)),
    error_json TEXT CHECK (error_json IS NULL OR json_valid(error_json)),
    started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TEXT
) STRICT;

CREATE INDEX idx_weeks_subject_id ON weeks(subject_id);
CREATE INDEX idx_homeworks_week_id ON homeworks(week_id);
CREATE INDEX idx_files_week_id ON files(week_id);
CREATE INDEX idx_files_homework_id ON files(homework_id);
CREATE INDEX idx_homework_versions_homework_id ON homework_versions(homework_id, number);
CREATE INDEX idx_source_bundles_homework_id ON source_bundles(homework_id);
CREATE INDEX idx_source_bundles_version_id ON source_bundles(version_id);
CREATE INDEX idx_source_bundle_items_bundle_id ON source_bundle_items(source_bundle_id);
CREATE INDEX idx_review_runs_homework_id ON review_runs(homework_id, created_at);
CREATE INDEX idx_revision_prompts_homework_id ON revision_prompts(homework_id);
CREATE INDEX idx_teacher_comments_homework_id ON teacher_comments(homework_id);
CREATE INDEX idx_teacher_requirements_homework_id ON teacher_requirements(homework_id);
CREATE INDEX idx_operations_status ON operations(status, started_at);
