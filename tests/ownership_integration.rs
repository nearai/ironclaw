//! Integration tests for the ownership model.
//!
//! Tests get_or_create_user, migrate_default_owner, tenant isolation, and ChannelPairingStore.
//! Uses libSQL file-backed tempdir — no PostgreSQL required.
//!
//! Note: `new_memory()` does NOT share schema across separate `connect()` calls
//! in libsql (each call gets its own connection and a new in-memory DB). All
//! tests here use `new_local` with a `tempfile::TempDir` so all connections
//! within the same test share the migrated schema.

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;

    use ironclaw::db::libsql::LibSqlBackend;
    use ironclaw::db::{ChannelPairingStore, Database, UserRecord, UserStore};
    use ironclaw::ownership::{Identity, OwnerId, UserRole};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Create a file-backed test DB with migrations applied.
    async fn setup_db() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("ownership_test.db");
        let db = LibSqlBackend::new_local(&db_path)
            .await
            .expect("test DB creation failed");
        db.run_migrations().await.expect("run migrations");
        (db, dir)
    }

    async fn create_user(db: &LibSqlBackend, id: &str, role: &str) {
        db.get_or_create_user(UserRecord {
            id: id.to_string(),
            role: role.to_string(),
            display_name: id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("user creation failed");
    }

    // -----------------------------------------------------------------------
    // Bootstrap tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_bootstrap_creates_owner_user() {
        let (db, _dir) = setup_db().await;

        // Owner does not exist yet
        assert!(db.get_user("henry").await.unwrap().is_none());

        // Create the owner via get_or_create_user (atomic upsert)
        create_user(&db, "henry", "admin").await;

        let user = db
            .get_user("henry")
            .await
            .unwrap()
            .expect("owner should exist");
        assert_eq!(user.id, "henry");
        assert_eq!(user.role, "admin");
        assert_eq!(user.status, "active");
    }

    #[tokio::test]
    async fn test_bootstrap_get_or_create_is_idempotent() {
        let (db, _dir) = setup_db().await;

        // Call twice — should not error or duplicate
        create_user(&db, "henry", "admin").await;
        create_user(&db, "henry", "admin").await;

        // Exactly one row
        let user = db
            .get_user("henry")
            .await
            .unwrap()
            .expect("owner should exist");
        assert_eq!(user.id, "henry");
    }

    #[tokio::test]
    async fn test_bootstrap_rewrites_default_user_id() {
        let (db, _dir) = setup_db().await;

        // Create a 'default' user (the pre-ownership placeholder)
        create_user(&db, "default", "member").await;

        // Insert a settings row with user_id = 'default'
        {
            let conn = db.connect().await.unwrap();
            conn.execute(
                "INSERT INTO settings (user_id, key, value, updated_at) \
                 VALUES ('default', 'test_migration_key', '\"test_value\"', \
                         strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                (),
            )
            .await
            .expect("insert settings row");
        }

        // Create the real owner
        create_user(&db, "henry", "admin").await;

        // Run migrate_default_owner
        db.migrate_default_owner("henry").await.unwrap();

        // The settings row should now be under 'henry'
        let conn = db.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT user_id FROM settings WHERE key = 'test_migration_key'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("row should exist");
        let user_id: String = row.get(0).unwrap();
        assert_eq!(
            user_id, "henry",
            "migrate_default_owner should rewrite 'default' to real owner"
        );
    }

    #[tokio::test]
    async fn test_migrate_default_owner_is_idempotent() {
        let (db, _dir) = setup_db().await;
        create_user(&db, "henry", "admin").await;

        // Run twice — should not error
        db.migrate_default_owner("owner-bootstrap-test")
            .await
            .unwrap();
        db.migrate_default_owner("henry").await.unwrap();

        // Still exactly one henry row
        let user = db.get_user("henry").await.unwrap();
        assert!(user.is_some());
    }

    #[tokio::test]
    async fn test_migrate_default_owner_no_default_rows() {
        let (db, _dir) = setup_db().await;
        create_user(&db, "henry", "admin").await;

        // No 'default' rows to migrate — should succeed without error
        db.migrate_default_owner("henry").await.unwrap();
    }

    #[tokio::test]
    async fn test_migrate_default_owner_succeeds_on_fresh_migrated_db() {
        let (db, _dir) = setup_db().await;

        // Fresh installs include ownerless tables like `dynamic_tools`; the
        // bootstrap rewrite should still succeed without assuming every table
        // in the schema carries a `user_id` column.
        db.migrate_default_owner("henry").await.unwrap();
    }

    // -----------------------------------------------------------------------
    // TenantScope isolation tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_tenant_scope_isolates_settings_by_user() {
        let (backend, _dir) = setup_db().await;
        let db: Arc<dyn Database> = Arc::new(backend);

        // Use TenantScope::new (the legacy bridge, avoids needing LibSqlBackend directly)
        // libSQL FK enforcement is off by default, so users table rows are not required here.
        let alice_scope = ironclaw::tenant::TenantScope::new("alice", Arc::clone(&db));
        alice_scope
            .set_setting("theme", &serde_json::json!("dark"))
            .await
            .unwrap();

        // Bob's scope should not see Alice's setting
        let bob_scope = ironclaw::tenant::TenantScope::new("bob", Arc::clone(&db));
        let bobs_theme = bob_scope.get_setting("theme").await.unwrap();
        assert!(
            bobs_theme.is_none(),
            "Bob should not see Alice's settings, got: {bobs_theme:?}"
        );

        // Alice can read her own setting
        let alices_theme = alice_scope.get_setting("theme").await.unwrap();
        assert_eq!(alices_theme, Some(serde_json::json!("dark")));
    }

    #[tokio::test]
    async fn test_tenant_scope_with_identity() {
        let (backend, _dir) = setup_db().await;
        let db: Arc<dyn Database> = Arc::new(backend);

        let alice_identity = Identity::new(OwnerId::from("alice"), UserRole::Member);
        let alice_scope =
            ironclaw::tenant::TenantScope::with_identity(alice_identity, Arc::clone(&db));
        alice_scope
            .set_setting("lang", &serde_json::json!("en"))
            .await
            .unwrap();

        // Bob uses Identity with Admin role — still cannot see Alice's setting
        let bob_identity = Identity::new(OwnerId::from("bob"), UserRole::Admin);
        let bob_scope = ironclaw::tenant::TenantScope::with_identity(bob_identity, Arc::clone(&db));
        let result = bob_scope.get_setting("lang").await.unwrap();
        assert!(
            result.is_none(),
            "Admin role must not bypass per-user setting isolation"
        );

        // Alice sees her own setting
        let alice_identity2 = Identity::new(OwnerId::from("alice"), UserRole::Member);
        let alice_scope2 =
            ironclaw::tenant::TenantScope::with_identity(alice_identity2, Arc::clone(&db));
        let alices = alice_scope2.get_setting("lang").await.unwrap();
        assert_eq!(alices, Some(serde_json::json!("en")));
    }

    #[tokio::test]
    async fn test_tenant_scope_user_id_accessor() {
        let (backend, _dir) = setup_db().await;
        let db: Arc<dyn Database> = Arc::new(backend);

        let identity = Identity::new(OwnerId::from("henry"), UserRole::Admin);
        let scope = ironclaw::tenant::TenantScope::with_identity(identity, db);
        assert_eq!(scope.user_id(), "henry");
        assert_eq!(scope.identity().owner_id.as_str(), "henry");
        assert_eq!(scope.identity().role, UserRole::Admin);
    }

    // -----------------------------------------------------------------------
    // ChannelPairingStore tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_pairing_different_users_are_independent() {
        let (db, _dir) = setup_db().await;
        create_user(&db, "alice", "member").await;
        create_user(&db, "bob", "member").await;

        let req_a = db
            .upsert_pairing_request("telegram", "tg-alice", None)
            .await
            .unwrap();
        let req_b = db
            .upsert_pairing_request("telegram", "tg-bob", None)
            .await
            .unwrap();

        db.approve_pairing("telegram", &req_a.code, "alice")
            .await
            .unwrap();
        db.approve_pairing("telegram", &req_b.code, "bob")
            .await
            .unwrap();

        let alice_id = db
            .resolve_channel_identity("telegram", "tg-alice")
            .await
            .unwrap()
            .expect("alice should be linked");
        let bob_id = db
            .resolve_channel_identity("telegram", "tg-bob")
            .await
            .unwrap()
            .expect("bob should be linked");

        assert_eq!(alice_id.owner_id.as_str(), "alice");
        assert_eq!(bob_id.owner_id.as_str(), "bob");
        assert_ne!(alice_id.owner_id, bob_id.owner_id);
    }

    #[tokio::test]
    async fn test_pairing_channels_are_isolated() {
        let (db, _dir) = setup_db().await;
        create_user(&db, "alice", "member").await;

        // Same external_id across two different channels
        let req_telegram = db
            .upsert_pairing_request("telegram", "user-999", None)
            .await
            .unwrap();
        let _req_slack = db
            .upsert_pairing_request("slack", "user-999", None)
            .await
            .unwrap();

        // Approve only telegram
        db.approve_pairing("telegram", &req_telegram.code, "alice")
            .await
            .unwrap();

        // telegram resolves; slack does not
        assert!(
            db.resolve_channel_identity("telegram", "user-999")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.resolve_channel_identity("slack", "user-999")
                .await
                .unwrap()
                .is_none()
        );
    }

    // -----------------------------------------------------------------------
    // OwnerId / Identity unit-level sanity
    // -----------------------------------------------------------------------

    #[test]
    fn test_owner_id_equality_and_display() {
        let a = OwnerId::from("alice");
        let b = OwnerId::from("alice");
        let c = OwnerId::from("bob");

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.as_str(), "alice");
        assert_eq!(a.to_string(), "alice");
    }

    #[test]
    fn test_can_act_on_own_resource() {
        use ironclaw::ownership::can_act_on;
        let actor = Identity::new(OwnerId::from("alice"), UserRole::Member);
        assert!(can_act_on(&actor, &OwnerId::from("alice")));
        assert!(!can_act_on(&actor, &OwnerId::from("bob")));
    }

    #[test]
    fn test_admin_role_does_not_bypass_ownership() {
        use ironclaw::ownership::can_act_on;
        // Admin role has no special bypass in can_act_on
        let admin = Identity::new(OwnerId::from("admin-user"), UserRole::Admin);
        assert!(!can_act_on(&admin, &OwnerId::from("bob")));
        assert!(can_act_on(&admin, &OwnerId::from("admin-user")));
    }
}
