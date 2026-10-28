INSERT INTO schema_migrations(version) VALUES (9);

-- Each workspace database has exactly one immutable identity. The application-wide
-- catalog lives in a separate SQLite file and only selects which database opens.
CREATE TABLE workspace_identity (
    singleton INTEGER PRIMARY KEY NOT NULL DEFAULT 1 CHECK (singleton = 1),
    workspace_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('demo', 'real')),
    vault_canonical TEXT,
    vault_fingerprint TEXT,
    created_at TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'archived')),
    CHECK ((vault_canonical IS NULL) = (vault_fingerprint IS NULL))
) STRICT;

CREATE TRIGGER workspace_identity_stable
BEFORE UPDATE OF workspace_id, kind, created_at ON workspace_identity
BEGIN
    SELECT RAISE(ABORT, 'workspace identity is immutable');
END;

CREATE TRIGGER workspace_identity_no_delete
BEFORE DELETE ON workspace_identity
BEGIN
    SELECT RAISE(ABORT, 'workspace identity is immutable');
END;
