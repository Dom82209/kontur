use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use tempfile::TempDir;

use super::{
    read_identity, verify_vault_binding, TransitionRequest, TransitionState,
    WorkspaceActivationDto, WorkspaceCatalog, WorkspaceKind, MARKER_FILE,
};
use crate::{
    db::Database,
    error::ErrorCode,
    intake::{RootKind, RootState, RootsService},
};

fn setup() -> (TempDir, WorkspaceCatalog) {
    let temp = TempDir::new().unwrap();
    let (catalog, _) = WorkspaceCatalog::bootstrap(&temp.path().join("app"), None).unwrap();
    (temp, catalog)
}

fn app(temp: &TempDir) -> PathBuf {
    temp.path().join("app")
}

fn vault(temp: &TempDir, name: &str) -> PathBuf {
    let path = temp.path().join("vaults").join(name);
    fs::create_dir_all(&path).unwrap();
    fs::canonicalize(path).unwrap()
}

/// Повторный запуск процесса: новый каталог + база активного пространства.
fn restart(temp: &TempDir) -> (WorkspaceCatalog, Database) {
    let (catalog, path) = WorkspaceCatalog::bootstrap(&app(temp), None).unwrap();
    let database = Database::open_file(path).unwrap();
    database.migrate().unwrap();
    catalog.verify_active_database(&database).unwrap();
    (catalog, database)
}

fn apply(
    catalog: &WorkspaceCatalog,
    request: TransitionRequest,
    vault: Option<&Path>,
) -> WorkspaceActivationDto {
    let preview = catalog.preview_transition(&request, vault).unwrap();
    catalog.confirm_transition(&preview.id).unwrap()
}

fn create_real(catalog: &WorkspaceCatalog, path: &Path) -> WorkspaceActivationDto {
    apply(catalog, TransitionRequest::CreateReal, Some(path))
}

fn connect(catalog: &WorkspaceCatalog, path: &Path) -> WorkspaceActivationDto {
    apply(catalog, TransitionRequest::ConnectExisting, Some(path))
}

fn insert_subject(database: &Database, id: &str) {
    database
        .with_transaction(|tx| {
            tx.execute(
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES (?1,?1,?1,'Предметы/Одинаковый путь')",
                [id],
            )?;
            Ok(())
        })
        .unwrap();
}

fn count(database: &Database, table: &str) -> i64 {
    database
        .with_connection(|connection| {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .map_err(Into::into)
        })
        .unwrap()
}

fn files_under(path: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                result.extend(files_under(&path));
            }
            result.push(path);
        }
    }
    result.sort();
    result
}

fn transition_state(catalog: &WorkspaceCatalog, id: &str) -> TransitionState {
    catalog
        .recent_transitions(100)
        .unwrap()
        .into_iter()
        .find(|transition| transition.id == id)
        .unwrap()
        .state
}

// ----- изоляция данных -----------------------------------------------------------------

#[test]
fn two_vaults_with_same_relative_paths_do_not_share_rows() {
    let (temp, catalog) = setup();
    let first = vault(&temp, "vault-a");
    create_real(&catalog, &first);
    let (catalog, database) = restart(&temp);
    insert_subject(&database, "same-id");
    let second = vault(&temp, "vault-b");
    create_real(&catalog, &second);
    let (catalog, second_database) = restart(&temp);
    assert_eq!(count(&second_database, "subjects"), 0);
    insert_subject(&second_database, "same-id");
    connect(&catalog, &first);
    let (_, first_database) = restart(&temp);
    assert_eq!(count(&first_database, "subjects"), 1);
}

#[test]
fn demo_database_stays_isolated_from_real_workspace() {
    let (temp, catalog) = setup();
    apply(&catalog, TransitionRequest::CreateDemo, None);
    let (catalog, demo_database) = restart(&temp);
    assert_eq!(
        read_identity(&demo_database).unwrap().kind,
        WorkspaceKind::Demo
    );
    insert_subject(&demo_database, "demo-only");
    let real = vault(&temp, "real-vault");
    create_real(&catalog, &real);
    let (_, real_database) = restart(&temp);
    assert_eq!(
        read_identity(&real_database).unwrap().kind,
        WorkspaceKind::Real
    );
    assert_eq!(count(&real_database, "subjects"), 0);
}

#[test]
fn submitted_snapshot_row_never_appears_in_another_workspace() {
    let (temp, catalog) = setup();
    let first = vault(&temp, "vault-a");
    create_real(&catalog, &first);
    let (catalog, first_database) = restart(&temp);
    first_database
        .with_connection(|connection| {
            connection.pragma_update(None, "foreign_keys", false)?;
            connection.execute(
                "INSERT INTO submission_receipts(id,package_id,homework_id,submitted_version_id,snapshot_path,snapshot_hash,confirmed_at,submitted_at,assignment_url)
                 VALUES ('receipt','package','homework','version','Снимки/receipt','hash','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','https://example.edu/task')",
                [],
            )?;
            connection.pragma_update(None, "foreign_keys", true)?;
            Ok(())
        })
        .unwrap();
    let second = vault(&temp, "vault-b");
    create_real(&catalog, &second);
    let (catalog, second_database) = restart(&temp);
    assert_eq!(count(&second_database, "submission_receipts"), 0);
    connect(&catalog, &first);
    let (_, restored) = restart(&temp);
    assert_eq!(count(&restored, "submission_receipts"), 1);
}

#[test]
fn every_workspace_has_its_own_database_file() {
    let (temp, catalog) = setup();
    create_real(&catalog, &vault(&temp, "a"));
    let (catalog, _) = restart(&temp);
    apply(&catalog, TransitionRequest::CreateDemo, None);
    let (catalog, _) = restart(&temp);
    let relpaths: Vec<String> = catalog
        .lock()
        .unwrap()
        .prepare("SELECT database_relpath FROM workspaces")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // legacy + real + demo, все разные файлы.
    assert_eq!(relpaths.len(), 3);
    let mut unique = relpaths.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3);
}

// ----- vault ownership ------------------------------------------------------------------

#[test]
fn configured_workspace_vault_cannot_be_rebound() {
    let (temp, catalog) = setup();
    let first = vault(&temp, "vault-a");
    let second = vault(&temp, "vault-b");
    create_real(&catalog, &first);
    let (_, database) = restart(&temp);
    insert_subject(&database, "non-empty");
    let error = RootsService::new(&database, None)
        .set_root(RootKind::Vault, &second)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    // Повторный выбор того же vault разрешён.
    RootsService::new(&database, None)
        .set_root(RootKind::Vault, &first)
        .unwrap();
    assert_eq!(count(&database, "subjects"), 1);
}

#[test]
fn demo_cannot_become_real_by_changing_vault() {
    let (temp, catalog) = setup();
    apply(&catalog, TransitionRequest::CreateDemo, None);
    let (_, database) = restart(&temp);
    let real = vault(&temp, "real");
    let error = RootsService::new(&database, None)
        .set_root(RootKind::Vault, &real)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(!real.join(MARKER_FILE).exists());
}

#[test]
fn reconnecting_known_vault_restores_its_database() {
    let (temp, catalog) = setup();
    let first = vault(&temp, "vault-a");
    create_real(&catalog, &first);
    let (catalog, database) = restart(&temp);
    insert_subject(&database, "restored");
    create_real(&catalog, &vault(&temp, "vault-b"));
    connect(&catalog, &first);
    let (_, restored) = restart(&temp);
    assert_eq!(count(&restored, "subjects"), 1);
}

#[test]
fn moved_vault_is_reconnected_by_marker_and_path_is_updated() {
    let (temp, catalog) = setup();
    let first = vault(&temp, "vault-a");
    let id = create_real(&catalog, &first).workspace.id;
    let (catalog, _) = restart(&temp);
    create_real(&catalog, &vault(&temp, "other"));
    let moved = temp.path().join("vaults").join("moved");
    fs::rename(&first, &moved).unwrap();
    let moved = fs::canonicalize(moved).unwrap();
    let activation = connect(&catalog, &moved);
    assert_eq!(activation.workspace.id, id);
    let (_, database) = restart(&temp);
    let identity = read_identity(&database).unwrap();
    assert_eq!(identity.vault_canonical.as_deref(), moved.to_str());
    assert_eq!(
        RootsService::new(&database, None)
            .status()
            .unwrap()
            .vault
            .state,
        RootState::Ok
    );
}

#[test]
fn changed_vault_fingerprint_is_blocked() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    create_real(&catalog, &path);
    let (catalog, _) = restart(&temp);
    create_real(&catalog, &vault(&temp, "other"));
    fs::remove_file(path.join(MARKER_FILE)).unwrap();
    fs::write(
        path.join(MARKER_FILE),
        r#"{"workspaceId":"00000000-0000-4000-8000-000000000000","nonce":"00000000-0000-4000-8000-000000000001"}"#,
    )
    .unwrap();
    let error = catalog
        .preview_transition(&TransitionRequest::ConnectExisting, Some(&path))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
}

#[test]
fn copy_of_live_vault_cannot_be_connected() {
    let (temp, catalog) = setup();
    let original = vault(&temp, "original");
    create_real(&catalog, &original);
    let (catalog, _) = restart(&temp);
    create_real(&catalog, &vault(&temp, "second"));
    // Копия vault с тем же marker при живом исходном vault не подключается.
    let copy = vault(&temp, "copy");
    fs::copy(original.join(MARKER_FILE), copy.join(MARKER_FILE)).unwrap();
    assert_eq!(
        catalog
            .preview_transition(&TransitionRequest::ConnectExisting, Some(&copy))
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn replaced_vault_folder_blocks_file_operations() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    create_real(&catalog, &path);
    let (_, database) = restart(&temp);
    // Пользователь подменил папку: то же имя, другой vault без marker.
    fs::rename(&path, temp.path().join("vaults").join("old")).unwrap();
    fs::create_dir_all(&path).unwrap();
    let roots = RootsService::new(&database, None);
    assert_eq!(roots.open_vault().unwrap_err().code, ErrorCode::Blocked);
    assert_eq!(roots.status().unwrap().vault.state, RootState::Unsafe);
    assert!(!roots.status().unwrap().import_ready);
    assert!(verify_vault_binding(&database, &path).is_err());
}

#[test]
fn active_database_mismatch_stops_startup() {
    let (temp, catalog) = setup();
    let first = create_real(&catalog, &vault(&temp, "a")).workspace.id;
    let (catalog, _) = restart(&temp);
    create_real(&catalog, &vault(&temp, "b"));
    let (catalog, _) = restart(&temp);
    let first_record = catalog.require_record(&first).unwrap();
    let first_database = Database::open_file(
        catalog
            .database_path(&first_record.database_relpath)
            .unwrap(),
    )
    .unwrap();
    let error = catalog.verify_active_database(&first_database).unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
}

// ----- preview / confirm / cancel ---------------------------------------------------------

#[test]
fn preview_writes_nothing_to_vault_or_workspace_databases() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    fs::write(path.join("note.md"), "# заметка").unwrap();
    let vault_before = files_under(&path);
    let workspaces_before = files_under(&app(&temp).join("workspaces"));
    let listed_before = catalog.list().unwrap();
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&path))
        .unwrap();
    assert!(preview
        .effects
        .iter()
        .any(|effect| effect.contains(MARKER_FILE)));
    assert_eq!(files_under(&path), vault_before);
    assert_eq!(
        files_under(&app(&temp).join("workspaces")),
        workspaces_before
    );
    assert_eq!(catalog.list().unwrap(), listed_before);
    assert_eq!(
        fs::read_to_string(path.join("note.md")).unwrap(),
        "# заметка"
    );
    assert_eq!(
        transition_state(&catalog, &preview.id),
        TransitionState::Previewed
    );
}

#[test]
fn cancelled_preview_cannot_be_confirmed() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&path))
        .unwrap();
    catalog.cancel_transition(&preview.id).unwrap();
    assert_eq!(
        catalog.confirm_transition(&preview.id).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(!path.join(MARKER_FILE).exists());
    assert_eq!(
        transition_state(&catalog, &preview.id),
        TransitionState::Cancelled
    );
}

#[test]
fn preview_is_single_use() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&path))
        .unwrap();
    catalog.confirm_transition(&preview.id).unwrap();
    assert_eq!(
        catalog.confirm_transition(&preview.id).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        catalog.cancel_transition(&preview.id).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn expired_preview_requires_new_preview() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&path))
        .unwrap();
    catalog
        .lock()
        .unwrap()
        .execute(
            "UPDATE workspace_transitions SET expires_at='2000-01-01T00:00:00.000Z' WHERE id=?1",
            [&preview.id],
        )
        .unwrap();
    assert_eq!(
        catalog.confirm_transition(&preview.id).unwrap_err().code,
        ErrorCode::ConfirmationRequired
    );
    assert!(!path.join(MARKER_FILE).exists());
    assert_eq!(
        transition_state(&catalog, &preview.id),
        TransitionState::Failed
    );
}

#[test]
fn vault_changed_after_preview_is_blocked_without_side_effects() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&path))
        .unwrap();
    // Папку подменили между preview и confirm.
    fs::rename(&path, temp.path().join("vaults").join("old")).unwrap();
    fs::create_dir_all(&path).unwrap();
    let before = catalog.list().unwrap();
    assert_eq!(
        catalog.confirm_transition(&preview.id).unwrap_err().code,
        ErrorCode::Blocked
    );
    assert!(!path.join(MARKER_FILE).exists());
    assert_eq!(catalog.list().unwrap(), before);
    assert_eq!(
        transition_state(&catalog, &preview.id),
        TransitionState::Failed
    );
}

#[test]
fn active_workspace_change_invalidates_older_preview() {
    let (temp, catalog) = setup();
    let first = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&vault(&temp, "a")))
        .unwrap();
    let second = catalog
        .preview_transition(&TransitionRequest::CreateDemo, None)
        .unwrap();
    catalog.confirm_transition(&second.id).unwrap();
    assert_eq!(
        catalog.confirm_transition(&first.id).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn already_registered_vault_cannot_be_created_twice() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    create_real(&catalog, &path);
    let (catalog, _) = restart(&temp);
    assert_eq!(
        catalog
            .preview_transition(&TransitionRequest::CreateReal, Some(&path))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn activating_current_workspace_is_rejected() {
    let (temp, catalog) = setup();
    let id = create_real(&catalog, &vault(&temp, "a")).workspace.id;
    let (catalog, _) = restart(&temp);
    assert_eq!(
        catalog
            .preview_transition(&TransitionRequest::Activate { workspace_id: id }, None)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn switching_back_to_demo_reverifies_its_vault() {
    let (temp, catalog) = setup();
    let demo = apply(&catalog, TransitionRequest::CreateDemo, None)
        .workspace
        .id;
    let (catalog, _) = restart(&temp);
    create_real(&catalog, &vault(&temp, "real"));
    let (catalog, _) = restart(&temp);
    apply(
        &catalog,
        TransitionRequest::Activate {
            workspace_id: demo.clone(),
        },
        None,
    );
    let (_, database) = restart(&temp);
    assert_eq!(read_identity(&database).unwrap().id, demo);
    assert_eq!(count(&database, "subjects"), 1);
}

#[test]
fn interrupted_transition_is_marked_failed_on_restart() {
    let (temp, catalog) = setup();
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateDemo, None)
        .unwrap();
    catalog
        .lock()
        .unwrap()
        .execute(
            "UPDATE workspace_transitions SET state='applying' WHERE id=?1",
            [&preview.id],
        )
        .unwrap();
    drop(catalog);
    let (catalog, _) = restart(&temp);
    assert_eq!(
        transition_state(&catalog, &preview.id),
        TransitionState::Failed
    );
}

// ----- rollback ------------------------------------------------------------------------------

#[test]
fn failed_real_creation_rolls_back_marker_and_catalog() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&path))
        .unwrap();
    let before = catalog.list().unwrap();
    // Сбой создания базы: каталог баз заменён обычным файлом.
    let workspaces = app(&temp).join("workspaces");
    fs::remove_dir_all(&workspaces).unwrap();
    fs::write(&workspaces, "not a directory").unwrap();
    assert!(catalog.confirm_transition(&preview.id).is_err());
    assert!(
        !path.join(MARKER_FILE).exists(),
        "marker должен быть удалён"
    );
    assert_eq!(catalog.list().unwrap(), before);
    assert_eq!(
        transition_state(&catalog, &preview.id),
        TransitionState::Failed
    );
}

#[test]
fn failed_demo_creation_leaves_no_directories() {
    let (temp, catalog) = setup();
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateDemo, None)
        .unwrap();
    let workspaces = app(&temp).join("workspaces");
    fs::remove_dir_all(&workspaces).unwrap();
    fs::write(&workspaces, "not a directory").unwrap();
    assert!(catalog.confirm_transition(&preview.id).is_err());
    let demo_root = app(&temp).join("demo-workspaces");
    assert!(fs::read_dir(&demo_root).map_or(true, |mut entries| entries.next().is_none()));
}

// ----- symlink / traversal ---------------------------------------------------------------------

#[test]
fn symlinked_marker_is_rejected() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let outside = temp.path().join("outside.json");
    fs::write(
        &outside,
        r#"{"workspaceId":"00000000-0000-4000-8000-000000000000","nonce":"00000000-0000-4000-8000-000000000001"}"#,
    )
    .unwrap();
    symlink(&outside, path.join(MARKER_FILE)).unwrap();
    for request in [
        TransitionRequest::CreateReal,
        TransitionRequest::ConnectExisting,
    ] {
        let error = catalog
            .preview_transition(&request, Some(&path))
            .unwrap_err();
        assert!(
            matches!(error.code, ErrorCode::Blocked | ErrorCode::Conflict),
            "{error:?}"
        );
    }
    let preview_count = catalog.recent_transitions(10).unwrap().len();
    assert_eq!(preview_count, 0);
}

#[test]
fn dangling_marker_symlink_is_not_followed_on_create() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let target = temp.path().join("escaped.json");
    symlink(&target, path.join(MARKER_FILE)).unwrap();
    assert!(catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&path))
        .is_err());
    assert!(!target.exists());
}

#[test]
fn symlinked_vault_root_is_rejected() {
    let (temp, catalog) = setup();
    let real = vault(&temp, "real");
    let link = temp.path().join("vaults").join("link");
    symlink(&real, &link).unwrap();
    assert!(catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&link))
        .is_err());
}

#[test]
fn vault_inside_app_data_is_rejected() {
    let (temp, catalog) = setup();
    let inside = app(&temp).join("my-vault");
    fs::create_dir_all(&inside).unwrap();
    assert_eq!(
        catalog
            .preview_transition(&TransitionRequest::CreateReal, Some(&inside))
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn database_paths_cannot_escape_app_data() {
    let (_temp, catalog) = setup();
    for bad in [
        "../kontur.sqlite3",
        "/etc/passwd",
        "workspaces/../../x.sqlite3",
        "workspaces/not-a-uuid.sqlite3",
        "workspaces/00000000-0000-4000-8000-000000000000/../x.sqlite3",
        "workspaces/./00000000-0000-4000-8000-000000000000.sqlite3",
        "other.sqlite3",
    ] {
        assert!(catalog.database_path(bad).is_err(), "{bad}");
    }
    assert!(catalog
        .database_path("workspaces/00000000-0000-4000-8000-000000000000.sqlite3")
        .is_ok());
}

#[test]
fn symlinked_workspace_directory_is_rejected() {
    let (temp, catalog) = setup();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let workspaces = app(&temp).join("workspaces");
    fs::remove_dir_all(&workspaces).unwrap();
    symlink(&outside, &workspaces).unwrap();
    assert!(catalog
        .database_path("workspaces/00000000-0000-4000-8000-000000000000.sqlite3")
        .is_err());
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateDemo, None)
        .unwrap();
    assert!(catalog.confirm_transition(&preview.id).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn catalog_records_are_append_only_and_identity_immutable() {
    let (temp, catalog) = setup();
    create_real(&catalog, &vault(&temp, "a"));
    let connection = catalog.lock().unwrap();
    assert!(connection.execute("DELETE FROM workspaces", []).is_err());
    assert!(connection
        .execute("UPDATE workspaces SET kind='demo'", [])
        .is_err());
    assert!(connection
        .execute(
            "UPDATE workspaces SET database_relpath='kontur.sqlite3'",
            []
        )
        .is_err());
    drop(connection);
    let (_, database) = restart(&temp);
    let error = database
        .with_connection(|connection| {
            connection
                .execute("UPDATE workspace_identity SET kind='demo'", [])
                .map_err(Into::into)
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Database);
}

// ----- restart / migration ----------------------------------------------------------------------

#[test]
fn restart_keeps_active_workspace() {
    let (temp, catalog) = setup();
    let path = vault(&temp, "vault");
    let expected = create_real(&catalog, &path).workspace.id;
    drop(catalog);
    let (_, database) = restart(&temp);
    assert_eq!(read_identity(&database).unwrap().id, expected);
}

fn legacy_v8(app_dir: &Path, vault: Option<&Path>) {
    fs::create_dir_all(app_dir).unwrap();
    let mut connection = Connection::open(app_dir.join("kontur.sqlite3")).unwrap();
    for (version, sql) in [
        (1, include_str!("db/migrations/v1.sql")),
        (2, include_str!("db/migrations/v2.sql")),
        (3, include_str!("db/migrations/v3.sql")),
        (4, include_str!("db/migrations/v4.sql")),
        (5, include_str!("db/migrations/v5.sql")),
        (6, include_str!("db/migrations/v6.sql")),
        (7, include_str!("db/migrations/v7.sql")),
        (8, include_str!("db/migrations/v8.sql")),
    ] {
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch(sql).unwrap();
        transaction
            .pragma_update(None, "user_version", version)
            .unwrap();
        transaction.commit().unwrap();
    }
    connection
        .execute(
            "INSERT INTO subjects(id,name,short_name,folder_path) VALUES ('legacy','Legacy','Legacy','Предметы/Legacy')",
            [],
        )
        .unwrap();
    if let Some(vault) = vault {
        let json = serde_json::json!({ "vaultPath": vault.to_str().unwrap() }).to_string();
        connection
            .execute(
                "INSERT INTO app_settings(key,value_json,created_at,updated_at)
                 VALUES ('managed_roots',?1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
                [json],
            )
            .unwrap();
    }
}

#[test]
fn schema_v8_without_vault_is_preserved_as_legacy_demo_workspace() {
    let temp = TempDir::new().unwrap();
    legacy_v8(&app(&temp), None);
    let (_, database) = restart(&temp);
    assert_eq!(database.schema_version().unwrap(), 9);
    assert_eq!(read_identity(&database).unwrap().kind, WorkspaceKind::Demo);
    assert_eq!(count(&database, "subjects"), 1);
}

#[test]
fn schema_v8_with_user_vault_becomes_real_workspace_without_touching_vault() {
    let temp = TempDir::new().unwrap();
    let path = vault(&temp, "user-vault");
    fs::write(path.join("note.md"), "x").unwrap();
    let before = files_under(&path);
    legacy_v8(&app(&temp), Some(&path));
    let (catalog, database) = restart(&temp);
    let identity = read_identity(&database).unwrap();
    assert_eq!(identity.kind, WorkspaceKind::Real);
    assert_eq!(identity.vault_canonical.as_deref(), path.to_str());
    assert!(identity.vault_fingerprint.is_some());
    assert_eq!(count(&database, "subjects"), 1);
    // Upgrade не пишет marker в пользовательский vault.
    assert_eq!(files_under(&path), before);
    assert_eq!(
        RootsService::new(&database, None)
            .status()
            .unwrap()
            .vault
            .state,
        RootState::Ok
    );
    // Legacy vault затем подключается по fingerprint (path + inode).
    create_real(&catalog, &vault(&temp, "other"));
    let activation = connect(&catalog, &path);
    assert_eq!(activation.workspace.id, identity.id);
}

#[test]
fn schema_v8_old_demo_workspace_stays_demo() {
    let temp = TempDir::new().unwrap();
    let demo_vault = app(&temp).join("demo-workspace").join("Учебное хранилище");
    fs::create_dir_all(&demo_vault).unwrap();
    legacy_v8(&app(&temp), Some(&fs::canonicalize(&demo_vault).unwrap()));
    let (_, database) = restart(&temp);
    assert_eq!(read_identity(&database).unwrap().kind, WorkspaceKind::Demo);
}

#[test]
fn legacy_vault_missing_at_upgrade_is_bound_later() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("vaults").join("usb");
    legacy_v8(&app(&temp), Some(&path));
    let (_, database) = restart(&temp);
    let identity = read_identity(&database).unwrap();
    assert_eq!(identity.kind, WorkspaceKind::Real);
    assert!(identity.vault_fingerprint.is_none());
    drop(database);
    fs::create_dir_all(&path).unwrap();
    let (_, database) = restart(&temp);
    assert!(read_identity(&database)
        .unwrap()
        .vault_fingerprint
        .is_some());
}

#[test]
fn all_workspace_files_stay_inside_tempdir() {
    let (temp, catalog) = setup();
    apply(&catalog, TransitionRequest::CreateDemo, None);
    let canonical_temp = fs::canonicalize(temp.path()).unwrap();
    for workspace in catalog.list().unwrap() {
        if let Some(path) = workspace.vault_canonical {
            assert!(Path::new(&path).starts_with(&canonical_temp));
        }
    }
}
