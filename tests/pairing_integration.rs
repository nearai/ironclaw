//! Integration tests for the DB-backed DM pairing flow.
//!
//! Verifies the full pairing lifecycle: upsert → list → approve → resolve_identity.
//! Uses libSQL file-backed tempdir for isolation.

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;

    use ironclaw::cli::{PairingCommand, run_pairing_command_with_store};
    use ironclaw::db::libsql::LibSqlBackend;
    use ironclaw::db::{Database, UserRecord};
    use ironclaw::ownership::{OwnerId, OwnershipCache};
    use ironclaw::pairing::PairingStore;

    async fn setup_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("pairing_test.db");
        let db = LibSqlBackend::new_local(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();
        (Arc::new(db), dir)
    }

    async fn setup_db_with_user(user_id: &str) -> (Arc<dyn Database>, tempfile::TempDir) {
        let (db, dir) = setup_db().await;
        db.get_or_create_user(UserRecord {
            id: user_id.to_string(),
            role: "member".to_string(),
            display_name: user_id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
            max_agents: None,
            max_tokens: None,
            tokens_used: 0,
        })
        .await
        .unwrap();
        (db, dir)
    }

    fn make_store(db: Arc<dyn Database>) -> PairingStore {
        let cache = Arc::new(OwnershipCache::new());
        PairingStore::new(db, cache)
    }

    #[tokio::test]
    async fn test_pairing_flow_unknown_user_to_approved() {
        let (db, _dir) = setup_db_with_user("owner_1").await;
        let store = make_store(Arc::clone(&db));
        let channel = "telegram";
        let owner_id = OwnerId::from("owner_1");

        // 1. Unknown user sends first message -> upsert creates request
        let r1 = store
            .upsert_request(
                channel,
                "user_12345",
                Some(serde_json::json!({
                    "chat_id": 999,
                    "username": "alice"
                })),
            )
            .await
            .unwrap();
        assert!(!r1.code.is_empty());
        assert_eq!(r1.code.len(), 8);

        // 2. List pending shows the request
        let pending = store.list_pending(channel).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].external_id, "user_12345");
        assert_eq!(pending[0].code, r1.code);

        // 3. User identity is not resolved yet
        assert!(
            store
                .resolve_identity(channel, "user_12345")
                .await
                .unwrap()
                .is_none()
        );

        // 4. Approve via code
        store.approve(channel, &r1.code, &owner_id).await.unwrap();

        // 5. User identity now resolves
        let identity = store.resolve_identity(channel, "user_12345").await.unwrap();
        assert!(identity.is_some());
        assert_eq!(identity.unwrap().owner_id.as_str(), "owner_1");

        // 6. Pending list is empty
        let pending_after = store.list_pending(channel).await.unwrap();
        assert!(pending_after.is_empty());
    }

    #[tokio::test]
    async fn test_pairing_flow_cli_approve() {
        let (db, _dir) = setup_db_with_user("owner_cli").await;
        let store = make_store(Arc::clone(&db));
        let owner_id = OwnerId::from("owner_cli");

        store
            .upsert_request("telegram", "user_999", None)
            .await
            .unwrap();
        let pending = store.list_pending("telegram").await.unwrap();
        let code = pending[0].code.clone();

        let result = run_pairing_command_with_store(
            &store,
            &owner_id,
            PairingCommand::Approve {
                channel: "telegram".to_string(),
                code,
            },
        )
        .await;
        assert!(result.is_ok());

        let identity = store
            .resolve_identity("telegram", "user_999")
            .await
            .unwrap();
        assert!(identity.is_some());
    }

    #[tokio::test]
    async fn test_pairing_reject_invalid_code() {
        let (db, _dir) = setup_db_with_user("owner_reject").await;
        let store = make_store(Arc::clone(&db));
        let owner_id = OwnerId::from("owner_reject");

        store
            .upsert_request("telegram", "user_1", None)
            .await
            .unwrap();

        let result = store.approve("telegram", "INVALID1", &owner_id).await;
        assert!(result.is_err());

        let result = run_pairing_command_with_store(
            &store,
            &owner_id,
            PairingCommand::Approve {
                channel: "telegram".to_string(),
                code: "BADCODE1".to_string(),
            },
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pairing_multiple_channels_isolated() {
        let (db, _dir) = setup_db_with_user("owner_multi").await;
        let store = make_store(Arc::clone(&db));
        let owner_id = OwnerId::from("owner_multi");

        let r_telegram = store
            .upsert_request("telegram", "user_a", None)
            .await
            .unwrap();
        let r_slack = store.upsert_request("slack", "user_b", None).await.unwrap();

        // Each channel has its own pending
        assert_eq!(store.list_pending("telegram").await.unwrap().len(), 1);
        assert_eq!(store.list_pending("slack").await.unwrap().len(), 1);

        // Approve in one channel doesn't affect the other
        store
            .approve("telegram", &r_telegram.code, &owner_id)
            .await
            .unwrap();
        assert!(
            store
                .resolve_identity("telegram", "user_a")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .resolve_identity("slack", "user_a")
                .await
                .unwrap()
                .is_none()
        );

        store
            .approve("slack", &r_slack.code, &owner_id)
            .await
            .unwrap();
        assert!(
            store
                .resolve_identity("slack", "user_b")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn test_pairing_store_normalizes_channel_keys_for_cache_and_db() {
        let (db, _dir) = setup_db_with_user("owner_case").await;
        let store = make_store(Arc::clone(&db));
        let owner_id = OwnerId::from("owner_case");

        let req = store
            .upsert_request("TeleGram", "user_case", None)
            .await
            .unwrap();
        assert_eq!(req.channel, "telegram");

        store
            .approve("telegram", &req.code, &owner_id)
            .await
            .unwrap();

        let first = store
            .resolve_identity("TELEGRAM", "user_case")
            .await
            .unwrap()
            .expect("identity should resolve");
        let second = store
            .resolve_identity("telegram", "user_case")
            .await
            .unwrap()
            .expect("identity should resolve on normalized cache key");

        assert_eq!(first.owner_id.as_str(), "owner_case");
        assert_eq!(second.owner_id.as_str(), "owner_case");
    }
}
