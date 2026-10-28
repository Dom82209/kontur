INSERT INTO schema_migrations(version) VALUES (11);

-- Unknown dates are stored honestly instead of invented ones: a week either has both
-- dates or none (''), a homework deadline is a timestamp or NULL (unknown).
DROP TRIGGER weeks_v2_required_fields;
CREATE TRIGGER weeks_v11_dates_pair
BEFORE INSERT ON weeks
WHEN (length(trim(NEW.starts_at)) = 0) <> (length(trim(NEW.ends_at)) = 0)
BEGIN
    SELECT RAISE(ABORT, 'week dates must be both known or both unknown');
END;
CREATE TRIGGER weeks_v11_dates_pair_update
BEFORE UPDATE OF starts_at, ends_at ON weeks
WHEN (length(trim(NEW.starts_at)) = 0) <> (length(trim(NEW.ends_at)) = 0)
BEGIN
    SELECT RAISE(ABORT, 'week dates must be both known or both unknown');
END;

DROP TRIGGER homeworks_v2_required_fields;
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

-- Production workflow, stage 2: adoption of an existing Obsidian vault.
-- A plan is produced by a read-only scan. It is immutable; only its status moves
-- previewed -> applied | cancelled | failed. Apply re-hashes every planned file.
CREATE TABLE vault_adoption_plans (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL,
    vault_fingerprint TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('previewed', 'applied', 'cancelled', 'failed')),
    plan_json TEXT NOT NULL,
    plan_sha256 TEXT NOT NULL CHECK (length(plan_sha256) = 64),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    finished_at TEXT,
    message TEXT,
    report_json TEXT,
    CHECK ((status = 'previewed') = (finished_at IS NULL))
) STRICT;

CREATE INDEX idx_vault_adoption_plans_created ON vault_adoption_plans(created_at);

CREATE TRIGGER vault_adoption_plans_immutable
BEFORE UPDATE OF id, workspace_id, vault_fingerprint, plan_json, plan_sha256, created_at, expires_at
ON vault_adoption_plans
BEGIN
    SELECT RAISE(ABORT, 'vault adoption plan is immutable');
END;

CREATE TRIGGER vault_adoption_plans_final
BEFORE UPDATE OF status ON vault_adoption_plans
WHEN OLD.status <> 'previewed'
BEGIN
    SELECT RAISE(ABORT, 'vault adoption plan is already finished');
END;

CREATE TRIGGER vault_adoption_plans_no_delete
BEFORE DELETE ON vault_adoption_plans
BEGIN
    SELECT RAISE(ABORT, 'vault adoption plans are append-only');
END;

-- Provenance: which rows were created by which applied plan (stable import ids).
CREATE TABLE vault_adoption_items (
    plan_id TEXT NOT NULL REFERENCES vault_adoption_plans(id) ON DELETE RESTRICT,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN ('subject', 'week', 'homework', 'file')),
    entity_id TEXT NOT NULL,
    relative_path TEXT NOT NULL CHECK (relative_path NOT LIKE '/%' AND relative_path NOT LIKE '\\%'),
    content_sha256 TEXT,
    PRIMARY KEY (entity_kind, entity_id)
) STRICT;

CREATE TRIGGER vault_adoption_items_immutable
BEFORE UPDATE ON vault_adoption_items
BEGIN
    SELECT RAISE(ABORT, 'vault adoption items are immutable');
END;
