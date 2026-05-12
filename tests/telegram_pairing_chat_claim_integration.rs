//! Regression coverage for #3317 — chat-surface pairing claim.
//!
//! Drives the full chain: submission parser → agent loop dispatch →
//! `bridge::handle_pairing_claim` → `PairingStore::approve`. The unit
//! tests in `src/bridge/router.rs` cover the no-extension-manager and
//! invalid-channel branches; this integration test exercises the
//! happy path through a real `Agent` + `ExtensionManager` +
//! `PairingStore`.
//!
//! Why this lives at the integration tier (per
//! `.claude/rules/testing.md` "Test Through the Caller"): the parser
//! and handler are correct individually, but the wiring between them
//! — `agent_loop.rs` calling `crate::bridge::handle_pairing_claim` —
//! is exactly where #3317 would silently regress if the new arm is
//! ever dropped from the dispatch match.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod pairing_chat_claim_tests {
    use std::sync::OnceLock;
    use std::time::Duration;

    use tokio::sync::Mutex;

    use crate::support::test_rig::TestRigBuilder;
    use ironclaw::db::UserRecord;

    /// Seed the test-channel user into the users table so the FK on
    /// `channel_identities.owner_id` is satisfied during pairing approval.
    /// The TestRig defaults its channel `user_id` to `"test-user"`, but
    /// only the owner row is created automatically — pairing-claim flows
    /// reach the DB constraint that other rig consumers don't.
    async fn seed_test_user(rig: &crate::support::test_rig::TestRig, user_id: &str) {
        rig.database()
            .get_or_create_user(UserRecord {
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
            })
            .await
            .expect("seed test user must succeed");
    }

    const TIMEOUT: Duration = Duration::from_secs(15);

    /// Engine v2 stores its state in a process-global `OnceLock`.
    /// Serialize tests in this file so one test's state doesn't bleed
    /// into the next instance.
    fn engine_v2_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[tokio::test]
    async fn chat_approve_telegram_code_completes_pairing() {
        let _guard = engine_v2_test_lock().lock().await;

        let rig = TestRigBuilder::new().with_engine_v2().build().await;
        rig.clear().await;

        // Pairing approval writes `channel_identities.owner_id` which
        // FKs to `users.id`. Seed the channel's user row so the
        // constraint is satisfied. (The owner row is auto-seeded but
        // the channel's user_id "test-user" is not.)
        seed_test_user(&rig, "test-user").await;

        let ext_mgr = rig
            .extension_manager()
            .cloned()
            .expect("test rig must wire an ExtensionManager");
        let pairing_store = ext_mgr
            .pairing_store()
            .cloned()
            .expect("ExtensionManager must wire a PairingStore for the chat-claim flow");

        // Mint a pairing code the user can claim. The bot's pairing
        // reply is what tells real users this code in the matching
        // production path; here we shortcut by inserting the request
        // directly so the handler under test exercises the full
        // approve → propagate → respond chain.
        let pairing = pairing_store
            .upsert_request("telegram", "tg-test-user-9001", None)
            .await
            .expect("pairing request upsert must succeed");
        assert!(
            !pairing.code.is_empty(),
            "pairing store must mint a non-empty code"
        );

        // Type the pairing claim into the chat surface — this is the
        // same path users naturally try after seeing the bot's reply
        // ("type `approve telegram CODE` in any IronClaw chat").
        rig.send_message(&format!("approve telegram {}", pairing.code))
            .await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert!(
            !responses.is_empty(),
            "agent must respond to the pairing claim within {TIMEOUT:?}"
        );
        let response_text = responses[0].content.clone();
        assert!(
            response_text.contains("Pairing approved") && response_text.contains("telegram"),
            "expected 'Pairing approved … telegram' response, got: {response_text}"
        );

        rig.shutdown();
    }

    #[tokio::test]
    async fn chat_approve_invalid_code_responds_clearly() {
        let _guard = engine_v2_test_lock().lock().await;

        let rig = TestRigBuilder::new().with_engine_v2().build().await;
        rig.clear().await;

        // No pairing request was minted, so any code is "invalid". The
        // user must see a clear rejection — not have the LLM improvise
        // an unhelpful answer like the original #3317 report.
        rig.send_message("approve telegram NEVERMINTED99").await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert!(
            !responses.is_empty(),
            "agent must respond to invalid pairing claim within {TIMEOUT:?}"
        );
        let response_text = responses[0].content.clone();
        assert!(
            response_text.contains("Invalid or expired pairing code"),
            "expected explicit invalid-code rejection, got: {response_text}"
        );

        rig.shutdown();
    }
}
