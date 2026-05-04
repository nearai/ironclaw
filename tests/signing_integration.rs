//! Integration tests for the signet signing service.
//!
//! These test the full `SigningService` lifecycle: key generation, tool call
//! signing, audit chain verification, and skiplist behavior — all through
//! the public crate API with filesystem isolation via `SIGNET_HOME`.

use std::collections::HashSet;
use std::sync::Mutex;

use ironclaw::signing::SigningService;

/// Serialise env-var mutations across tests in this file.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn signing_service_full_lifecycle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _guard = ENV_MUTEX.lock().unwrap();
    // SAFETY: serialised via ENV_MUTEX — no concurrent readers/writers.
    unsafe { std::env::set_var("SIGNET_HOME", dir.path().as_os_str()) };

    // 1. Init auto-generates key on first run
    let service = SigningService::init(HashSet::new()).expect("first init should succeed");

    // 2. Sign several tool calls
    let tools = [
        (
            "shell",
            serde_json::json!({"command": "ls -la"}),
            "file listing",
            true,
        ),
        (
            "http_fetch",
            serde_json::json!({"url": "https://example.com"}),
            "200 OK",
            true,
        ),
        (
            "web_search",
            serde_json::json!({"query": "rust async"}),
            "timeout",
            false,
        ),
    ];

    for (tool, params, output, success) in &tools {
        let receipt = service.sign_action(tool, params, output, *success, "test-user");
        assert!(receipt.is_some(), "tool '{tool}' should produce a receipt");
    }

    // 3. Verify chain integrity
    let status = service
        .verify_chain()
        .expect("chain verification should succeed");
    assert!(status.valid, "chain must be valid");
    assert_eq!(
        status.total_records, 3,
        "should have exactly 3 audit records"
    );

    // 4. Second init on same directory loads existing key (no re-generate)
    let service2 = SigningService::init(HashSet::new()).expect("second init should load key");

    // Sign one more action with the reloaded key
    let receipt = service2.sign_action(
        "echo",
        &serde_json::json!({"text": "hello"}),
        "hello",
        true,
        "test-user",
    );
    assert!(receipt.is_some());

    // Chain should now have 4 records
    let status = service2.verify_chain().expect("verify after reload");
    assert!(status.valid, "chain must remain valid after key reload");
    assert_eq!(status.total_records, 4);

    // SAFETY: serialised via ENV_MUTEX
    unsafe { std::env::remove_var("SIGNET_HOME") };
}

#[test]
fn signing_service_skiplist_integration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _guard = ENV_MUTEX.lock().unwrap();
    // SAFETY: serialised via ENV_MUTEX
    unsafe { std::env::set_var("SIGNET_HOME", dir.path().as_os_str()) };

    let skip = HashSet::from(["echo".to_string(), "time".to_string()]);
    let service = SigningService::init(skip).expect("init with skiplist");

    // Skipped tools return None and do not appear in audit
    assert!(
        service
            .sign_action("echo", &serde_json::json!({}), "", true, "u")
            .is_none()
    );
    assert!(
        service
            .sign_action("time", &serde_json::json!({}), "", true, "u")
            .is_none()
    );

    // Non-skipped tool produces receipt
    assert!(
        service
            .sign_action("shell", &serde_json::json!({}), "ok", true, "u")
            .is_some()
    );

    let status = service.verify_chain().expect("verify");
    assert!(status.valid);
    assert_eq!(
        status.total_records, 1,
        "only non-skipped tool should be in audit"
    );

    // SAFETY: serialised via ENV_MUTEX
    unsafe { std::env::remove_var("SIGNET_HOME") };
}
