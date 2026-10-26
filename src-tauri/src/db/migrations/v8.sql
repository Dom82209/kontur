INSERT INTO schema_migrations(version) VALUES (8);

CREATE TABLE managed_notes (
    id TEXT PRIMARY KEY NOT NULL,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN ('subject', 'week', 'homework')),
    entity_id TEXT NOT NULL,
    relative_path TEXT NOT NULL UNIQUE CHECK (
        relative_path NOT LIKE '/%' AND relative_path NOT LIKE '\%' AND
        relative_path NOT LIKE '%/../%' AND instr(relative_path, ':') = 0
    ),
    managed_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(entity_kind, entity_id)
) STRICT;

CREATE TABLE style_profile_sources (
    id TEXT PRIMARY KEY NOT NULL,
    file_id TEXT REFERENCES files(id) ON DELETE RESTRICT,
    relative_path TEXT NOT NULL CHECK (
        relative_path NOT LIKE '/%' AND relative_path NOT LIKE '\%' AND
        relative_path NOT LIKE '%/../%' AND instr(relative_path, ':') = 0
    ),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('previous_homework', 'style_markdown', 'confirmed_final')),
    traits_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(traits_json)),
    explicitly_selected INTEGER NOT NULL CHECK (explicitly_selected = 1),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(relative_path)
) STRICT;

CREATE TABLE route_operation_items (
    operation_id TEXT NOT NULL REFERENCES operations(id) ON DELETE RESTRICT,
    inbox_file_id TEXT NOT NULL REFERENCES inbox_files(id) ON DELETE RESTRICT,
    target_path TEXT NOT NULL CHECK (target_path NOT LIKE '/%' AND target_path NOT LIKE '%/../%'),
    action TEXT NOT NULL CHECK (action IN ('copy', 'skip', 'rename')),
    PRIMARY KEY(operation_id, inbox_file_id)
) STRICT;

ALTER TABLE submission_packages ADD COLUMN manifest_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(manifest_json));
ALTER TABLE submission_receipts ADD COLUMN manifest_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(manifest_json));

CREATE TRIGGER submitted_snapshot_manifest_no_update
BEFORE UPDATE OF manifest_json, snapshot_path, snapshot_hash, files_json ON submission_receipts
BEGIN
    SELECT RAISE(ABORT, 'submitted snapshot is immutable');
END;
