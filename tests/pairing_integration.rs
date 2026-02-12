//! Integration tests for the DM pairing flow.
//!
//! Verifies the full pairing lifecycle: upsert → list → approve → allowFrom → is_sender_allowed.
//! Uses temp directory for isolation.

use ironclaw::cli::{PairingCommand, run_pairing_command_with_store};
use ironclaw::pairing::PairingStore;
use tempfile::TempDir;

fn test_store() -> (PairingStore, TempDir) {
    let dir = TempDir::new().unwrap();
    let store = PairingStore::with_base_dir(dir.path().to_path_buf());
    (store, dir)
}

#[test]
fn test_pairing_flow_unknown_user_to_approved() {
    let (store, _) = test_store();
    let channel = "telegram";

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
        .unwrap();
    assert!(r1.created);
    assert!(!r1.code.is_empty());
    assert_eq!(r1.code.len(), 8);

    // 2. List pending shows the request
    let pending = store.list_pending(channel).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "user_12345");
    assert_eq!(pending[0].code, r1.code);

    // 3. User is not allowed yet
    assert!(
        !store
            .is_sender_allowed(channel, "user_12345", Some("alice"))
            .unwrap()
    );

    // 4. Approve via code
    let approved = store.approve(channel, &r1.code).unwrap();
    assert!(approved.is_some());
    assert_eq!(approved.unwrap().id, "user_12345");

    // 5. User is now allowed
    assert!(
        store
            .is_sender_allowed(channel, "user_12345", None)
            .unwrap()
    );
    assert!(
        store
            .is_sender_allowed(channel, "user_12345", Some("alice"))
            .unwrap()
    );

    // 6. Pending list is empty
    let pending_after = store.list_pending(channel).unwrap();
    assert!(pending_after.is_empty());

    // 7. allowFrom contains the user
    let allow = store.read_allow_from(channel).unwrap();
    assert_eq!(allow, vec!["user_12345"]);
}

#[test]
fn test_pairing_flow_cli_approve() {
    let (store, _) = test_store();
    store.upsert_request("telegram", "user_999", None).unwrap();
    let pending = store.list_pending("telegram").unwrap();
    let code = pending[0].code.clone();

    let result = run_pairing_command_with_store(
        &store,
        PairingCommand::Approve {
            channel: "telegram".to_string(),
            code,
        },
    );
    assert!(result.is_ok());
    assert!(
        store
            .is_sender_allowed("telegram", "user_999", None)
            .unwrap()
    );
}

#[test]
fn test_pairing_reject_invalid_code() {
    let (store, _) = test_store();
    store.upsert_request("telegram", "user_1", None).unwrap();

    let result = store.approve("telegram", "INVALID1");
    assert!(result.unwrap().is_none());

    let result = run_pairing_command_with_store(
        &store,
        PairingCommand::Approve {
            channel: "telegram".to_string(),
            code: "BADCODE1".to_string(),
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_pairing_multiple_channels_isolated() {
    let (store, _) = test_store();

    let r_telegram = store.upsert_request("telegram", "user_a", None).unwrap();
    let r_slack = store.upsert_request("slack", "user_b", None).unwrap();

    // Each channel has its own pending
    assert_eq!(store.list_pending("telegram").unwrap().len(), 1);
    assert_eq!(store.list_pending("slack").unwrap().len(), 1);

    // Approve in one channel doesn't affect the other
    store.approve("telegram", &r_telegram.code).unwrap();
    assert!(store.is_sender_allowed("telegram", "user_a", None).unwrap());
    assert!(!store.is_sender_allowed("slack", "user_a", None).unwrap());

    store.approve("slack", &r_slack.code).unwrap();
    assert!(store.is_sender_allowed("slack", "user_b", None).unwrap());
}
