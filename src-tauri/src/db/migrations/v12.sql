INSERT INTO schema_migrations(version) VALUES (12);

-- Production workflow, stage 3: subjects are managed entities.
-- archived_at: hidden from automatic and manual classification, data kept.
-- merged_into_id: the subject was merged into another one (always archived).
ALTER TABLE subjects ADD COLUMN archived_at TEXT;
ALTER TABLE subjects ADD COLUMN merged_into_id TEXT REFERENCES subjects(id) ON DELETE RESTRICT;
ALTER TABLE subjects ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0);

CREATE TRIGGER subjects_v12_merged_is_archived
BEFORE UPDATE OF merged_into_id, archived_at ON subjects
WHEN NEW.merged_into_id IS NOT NULL AND NEW.archived_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'merged subject must stay archived');
END;

CREATE TRIGGER subjects_v12_merge_is_final
BEFORE UPDATE OF merged_into_id ON subjects
WHEN OLD.merged_into_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'subject merge cannot be changed');
END;

CREATE TRIGGER subjects_v12_no_self_merge
BEFORE UPDATE OF merged_into_id ON subjects
WHEN NEW.merged_into_id = NEW.id
BEGIN
    SELECT RAISE(ABORT, 'subject cannot be merged into itself');
END;

-- Keywords belong to the subject (single source of truth). AutomationRules.subjectRules
-- is now derived from this table.
CREATE TABLE subject_keywords (
    subject_id TEXT NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    keyword TEXT NOT NULL CHECK (length(trim(keyword)) >= 2 AND keyword = trim(keyword)),
    PRIMARY KEY (subject_id, keyword)
) STRICT;

INSERT OR IGNORE INTO subject_keywords(subject_id, keyword)
SELECT json_extract(rule.value, '$.subjectId'), trim(keyword.value)
FROM app_settings settings,
     json_each(settings.value_json, '$.subjectRules') rule,
     json_each(rule.value, '$.keywords') keyword
WHERE settings.key = 'automation_rules'
  AND json_valid(settings.value_json)
  AND keyword.type = 'text'
  AND length(trim(keyword.value)) >= 2
  AND EXISTS (SELECT 1 FROM subjects s WHERE s.id = json_extract(rule.value, '$.subjectId'));

-- Append-only merge journal.
CREATE TABLE subject_merges (
    id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL REFERENCES subjects(id) ON DELETE RESTRICT,
    target_id TEXT NOT NULL REFERENCES subjects(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL,
    moved_weeks INTEGER NOT NULL CHECK (moved_weeks >= 0),
    retargeted_inbox INTEGER NOT NULL CHECK (retargeted_inbox >= 0),
    created_at TEXT NOT NULL,
    CHECK (source_id <> target_id)
) STRICT;

CREATE TRIGGER subject_merges_immutable
BEFORE UPDATE ON subject_merges
BEGIN
    SELECT RAISE(ABORT, 'subject merges are append-only');
END;

CREATE TRIGGER subject_merges_no_delete
BEFORE DELETE ON subject_merges
BEGIN
    SELECT RAISE(ABORT, 'subject merges are append-only');
END;
