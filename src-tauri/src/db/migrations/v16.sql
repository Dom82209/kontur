-- Этап 8: неизменяемые ревизии комплекта на сдачу.
--
-- Каждая сборка — новая строка с номером ревизии; прежние ревизии остаются в
-- истории (устаревшими их делает отпечаток входных данных или более новая
-- ревизия). Содержимое ревизии не меняется; меняется только статус на
-- `submitted` при подтверждении отправки. Ручные подтверждения хранятся
-- отдельным журналом, а не правкой checklist.
--
-- Таблица перестраивается (снимается UNIQUE(homework_id)); раннер выключает
-- внешние ключи и проверяет foreign_key_check до коммита.

CREATE TABLE submission_packages_v16 (
    id TEXT PRIMARY KEY NOT NULL,
    homework_id TEXT NOT NULL REFERENCES homeworks(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    final_version_id TEXT NOT NULL REFERENCES homework_versions(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('draft', 'ready', 'submitted')),
    package_path TEXT NOT NULL UNIQUE CHECK (package_path NOT LIKE '/%' AND package_path NOT LIKE '\%'),
    checklist_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(checklist_json)),
    items_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(items_json)),
    links_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(links_json)),
    warnings_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(warnings_json)),
    upload_list_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(upload_list_json)),
    manifest_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(manifest_json)),
    requirements_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(requirements_json)),
    inputs_json TEXT CHECK (inputs_json IS NULL OR json_valid(inputs_json)),
    inputs_fingerprint TEXT,
    built_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (homework_id, revision)
) STRICT;

INSERT INTO submission_packages_v16(
    id, homework_id, revision, final_version_id, status, package_path, checklist_json, items_json,
    links_json, warnings_json, upload_list_json, manifest_json, inputs_fingerprint, built_at,
    created_at, updated_at
)
SELECT id, homework_id, 1, final_version_id, status, package_path, checklist_json, items_json,
       links_json, warnings_json, upload_list_json, manifest_json, inputs_fingerprint, built_at,
       created_at, updated_at
FROM submission_packages;

DROP TABLE submission_packages;
ALTER TABLE submission_packages_v16 RENAME TO submission_packages;

CREATE INDEX submission_packages_homework ON submission_packages(homework_id, revision);

CREATE TRIGGER submission_packages_v2_required_fields
BEFORE INSERT ON submission_packages
WHEN NEW.built_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'submission package built_at is required');
END;

CREATE TRIGGER submitted_homeworks_no_new_package
BEFORE INSERT ON submission_packages
WHEN EXISTS (SELECT 1 FROM homeworks WHERE id = NEW.homework_id AND stage = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted homework cannot receive submission packages');
END;

CREATE TRIGGER submission_packages_revision_sequence
BEFORE INSERT ON submission_packages
WHEN NEW.revision <> coalesce(
    (SELECT max(revision) FROM submission_packages WHERE homework_id = NEW.homework_id), 0) + 1
BEGIN
    SELECT RAISE(ABORT, 'submission package revisions are sequential');
END;

CREATE TRIGGER submission_packages_content_immutable
BEFORE UPDATE OF id, homework_id, revision, final_version_id, package_path, checklist_json,
    items_json, links_json, warnings_json, upload_list_json, manifest_json, requirements_json,
    inputs_json, inputs_fingerprint, built_at, created_at ON submission_packages
BEGIN
    SELECT RAISE(ABORT, 'submission package revision is immutable');
END;

CREATE TRIGGER submission_packages_status_only_to_submitted
BEFORE UPDATE OF status ON submission_packages
WHEN OLD.status = 'submitted' OR NEW.status <> 'submitted'
BEGIN
    SELECT RAISE(ABORT, 'submission package status can only become submitted, once');
END;

CREATE TRIGGER submission_packages_no_delete
BEFORE DELETE ON submission_packages
BEGIN
    SELECT RAISE(ABORT, 'submission package revisions are kept for history');
END;

-- Ручные подтверждения пунктов ревизии (только добавление; действует последнее).
CREATE TABLE package_manual_checks (
    id TEXT PRIMARY KEY NOT NULL,
    package_id TEXT NOT NULL REFERENCES submission_packages(id) ON DELETE RESTRICT,
    check_id TEXT NOT NULL CHECK (check_id IN ('links', 'manual_review')),
    confirmed INTEGER NOT NULL CHECK (confirmed IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX package_manual_checks_package ON package_manual_checks(package_id, check_id, created_at);

CREATE TRIGGER package_manual_checks_append_only
BEFORE UPDATE ON package_manual_checks
BEGIN
    SELECT RAISE(ABORT, 'manual package checks are append-only');
END;

CREATE TRIGGER package_manual_checks_no_delete
BEFORE DELETE ON package_manual_checks
BEGIN
    SELECT RAISE(ABORT, 'manual package checks are append-only');
END;

CREATE TRIGGER package_manual_checks_open_revision
BEFORE INSERT ON package_manual_checks
WHEN EXISTS (SELECT 1 FROM submission_packages WHERE id = NEW.package_id AND status = 'submitted')
BEGIN
    SELECT RAISE(ABORT, 'submitted package checks are immutable');
END;
