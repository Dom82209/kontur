INSERT INTO schema_migrations(version) VALUES (10);

-- Vault fingerprint of a workspace database may be bound once (legacy workspaces are
-- bound on first access); afterwards it can never point to a different vault.
CREATE TRIGGER workspace_identity_fingerprint_bound_once
BEFORE UPDATE OF vault_fingerprint ON workspace_identity
WHEN OLD.vault_fingerprint IS NOT NULL
  AND NEW.vault_fingerprint IS NOT OLD.vault_fingerprint
BEGIN
    SELECT RAISE(ABORT, 'workspace vault fingerprint is immutable');
END;
