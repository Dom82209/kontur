-- Этап 10: родословная версий, решения пользователя, сравнения и план исправлений.
--
-- Версии по-прежнему неизменяемы. Родословная записывается при создании;
-- решения пользователя — отдельным журналом (только добавление). Прежние версии
-- получают понятный источник по автору и пустого родителя.

ALTER TABLE homework_versions ADD COLUMN parent_version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT;
ALTER TABLE homework_versions ADD COLUMN origin TEXT NOT NULL DEFAULT 'other'
    CHECK (origin IN ('user_import', 'claude_result', 'codex_result', 'manual_edit', 'restored', 'other'));
ALTER TABLE homework_versions ADD COLUMN created_by TEXT NOT NULL DEFAULT '';
ALTER TABLE homework_versions ADD COLUMN creation_reason TEXT NOT NULL DEFAULT '';
ALTER TABLE homework_versions ADD COLUMN review_run_id TEXT;
ALTER TABLE homework_versions ADD COLUMN inputs_fingerprint TEXT;

-- Однократная разметка прежних версий: триггер неизменяемости снимается
-- только на время этого UPDATE и создаётся заново тем же определением.
DROP TRIGGER homework_versions_no_update;
UPDATE homework_versions
   SET origin = CASE author WHEN 'claude' THEN 'claude_result' WHEN 'codex' THEN 'codex_result' ELSE 'user_import' END,
       created_by = author,
       creation_reason = 'Создана до учёта родословной (этап 10): родитель неизвестен';
CREATE TRIGGER homework_versions_no_update
BEFORE UPDATE ON homework_versions
BEGIN
    SELECT RAISE(ABORT, 'homework_version is immutable');
END;

CREATE TRIGGER homework_versions_parent_same_homework
BEFORE INSERT ON homework_versions
WHEN NEW.parent_version_id IS NOT NULL AND (
    NEW.parent_version_id = NEW.id OR NOT EXISTS (
        SELECT 1 FROM homework_versions
         WHERE id = NEW.parent_version_id AND homework_id = NEW.homework_id
    )
)
BEGIN
    SELECT RAISE(ABORT, 'parent version must be an existing version of the same homework');
END;

CREATE TABLE version_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    decision TEXT NOT NULL CHECK (decision IN ('accepted', 'rejected', 'superseded', 'final')),
    reason TEXT NOT NULL DEFAULT '',
    superseded_by_version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    decided_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (decision <> 'rejected' OR length(trim(reason)) > 0),
    CHECK ((decision = 'superseded') = (superseded_by_version_id IS NOT NULL))
) STRICT;

CREATE INDEX version_decisions_version ON version_decisions(version_id, decided_at);

CREATE TRIGGER version_decisions_append_only
BEFORE UPDATE ON version_decisions
BEGIN
    SELECT RAISE(ABORT, 'version decisions are append-only');
END;

CREATE TRIGGER version_decisions_no_delete
BEFORE DELETE ON version_decisions
BEGIN
    SELECT RAISE(ABORT, 'version decisions are append-only');
END;

CREATE TRIGGER version_decisions_same_homework
BEFORE INSERT ON version_decisions
WHEN NOT EXISTS (SELECT 1 FROM homework_versions WHERE id = NEW.version_id AND homework_id = NEW.homework_id)
  OR (NEW.superseded_by_version_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM homework_versions WHERE id = NEW.superseded_by_version_id AND homework_id = NEW.homework_id))
  OR EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'version decision must stay within one open homework');
END;

-- Структурированный результат сравнения двух версий (без полных текстов).
CREATE TABLE version_comparisons (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    base_version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    target_version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    base_fingerprint TEXT NOT NULL,
    target_fingerprint TEXT NOT NULL,
    extractor_version INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('identical', 'complete', 'partial', 'manual_required')),
    summary TEXT NOT NULL,
    changes_json TEXT NOT NULL CHECK (json_valid(changes_json)),
    warnings_json TEXT NOT NULL CHECK (json_valid(warnings_json)),
    manual_checks_json TEXT NOT NULL CHECK (json_valid(manual_checks_json)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (base_version_id <> target_version_id)
) STRICT;

CREATE INDEX version_comparisons_pair ON version_comparisons(base_version_id, target_version_id, created_at);

CREATE TRIGGER version_comparisons_immutable
BEFORE UPDATE ON version_comparisons
BEGIN
    SELECT RAISE(ABORT, 'version comparison is immutable');
END;

-- План исправлений: строится по актуальной проверке и требованиям.
CREATE TABLE correction_plans (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE CASCADE,
    version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    review_run_id TEXT REFERENCES review_runs(id) ON DELETE RESTRICT,
    requirements_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TRIGGER correction_plans_immutable
BEFORE UPDATE ON correction_plans
BEGIN
    SELECT RAISE(ABORT, 'correction plan is immutable');
END;

CREATE TABLE correction_items (
    id TEXT PRIMARY KEY NOT NULL,
    plan_id TEXT NOT NULL REFERENCES correction_plans(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    category TEXT NOT NULL,
    issue TEXT NOT NULL CHECK (length(trim(issue)) > 0),
    source_requirement TEXT NOT NULL DEFAULT '',
    evidence TEXT NOT NULL DEFAULT '',
    file TEXT,
    location TEXT,
    expected_change TEXT NOT NULL,
    acceptance_criterion TEXT NOT NULL,
    priority TEXT NOT NULL CHECK (priority IN ('high', 'medium', 'low')),
    origin TEXT NOT NULL CHECK (origin IN ('review', 'requirement')),
    status TEXT NOT NULL CHECK (status IN ('proposed', 'open', 'resolved', 'dismissed')),
    linked_review_run_id TEXT REFERENCES review_runs(id) ON DELETE RESTRICT,
    linked_comment_id TEXT,
    linked_requirement_id TEXT REFERENCES teacher_requirements(id) ON DELETE SET NULL,
    resolved_in_version_id TEXT REFERENCES homework_versions(id) ON DELETE RESTRICT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (plan_id, position),
    CHECK ((status = 'resolved') = (resolved_in_version_id IS NOT NULL))
) STRICT;

CREATE TRIGGER correction_items_content_immutable
BEFORE UPDATE OF id, plan_id, position, category, issue, source_requirement, evidence, file, location,
    expected_change, acceptance_criterion, priority, origin, linked_review_run_id, linked_comment_id
ON correction_items
BEGIN
    SELECT RAISE(ABORT, 'correction item content is immutable');
END;

CREATE TRIGGER correction_items_resolved_same_homework
BEFORE UPDATE OF resolved_in_version_id ON correction_items
WHEN NEW.resolved_in_version_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM homework_versions v JOIN correction_plans p ON p.id = NEW.plan_id
     WHERE v.id = NEW.resolved_in_version_id AND v.homework_id = p.homework_id
)
BEGIN
    SELECT RAISE(ABORT, 'resolved version must belong to the same homework');
END;
