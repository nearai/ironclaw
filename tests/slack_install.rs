//! Integration tests for the `ironclaw channels install slack` workflow.
//!
//! Covers the four checks called out in the install-path spec:
//!   (a) manifest JSON has minimal scopes
//!   (b) OAuth callback for the bot-token shape parses the response
//!   (c) workspace identity write hits `channel_identities` with the team id
//!   (d) duplicate install of the same workspace id reuses the existing identity
//!
//! Uses libSQL file-backed tempdir for isolation, mirroring `tests/pairing_integration.rs`.

#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw::channels::slack::{
    MINIMAL_BOT_SCOPES, manifest::manifest_json, parse_oauth_v2_access,
};
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::{Database, UserRecord, UserStore};

async fn setup_db_with_owner(owner_id: &str) -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("slack_install_test.db");
    let db = LibSqlBackend::new_local(&db_path).await.unwrap();
    db.run_migrations().await.unwrap();
    db.get_or_create_user(UserRecord {
        id: owner_id.to_string(),
        role: "owner".to_string(),
        display_name: owner_id.to_string(),
        status: "active".to_string(),
        email: None,
        last_login_at: None,
        created_by: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        metadata: serde_json::Value::Null,
    })
    .await
    .unwrap();
    (Arc::new(db), dir)
}

// (a) manifest JSON has minimal scopes
#[test]
fn manifest_json_carries_minimal_bot_scopes() {
    let m = manifest_json("https://ironclaw.example.com");
    let scopes: Vec<&str> = m["oauth_config"]["scopes"]["bot"]
        .as_array()
        .expect("scopes.bot is an array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    let expected: Vec<&str> = MINIMAL_BOT_SCOPES.to_vec();
    let mut got_sorted = scopes.clone();
    got_sorted.sort();
    let mut want_sorted = expected.clone();
    want_sorted.sort();
    assert_eq!(
        got_sorted, want_sorted,
        "manifest.oauth_config.scopes.bot must equal MINIMAL_BOT_SCOPES exactly; got {scopes:?}"
    );

    // Slash command must point at the deployment's install base URL.
    let slash_url = m["features"]["slash_commands"][0]["url"]
        .as_str()
        .expect("slash command url");
    assert_eq!(
        slash_url,
        "https://ironclaw.example.com/api/channels/slack/slash"
    );
}

// (b) OAuth callback for the bot-token shape parses the response
#[test]
fn parses_oauth_v2_access_bot_token_response() {
    // Real-shape fixture from Slack's docs (June 2024).
    let body = r#"{
        "ok": true,
        "access_token": "xoxb-FAKE-FIXTURE-FOR-INTEGRATION-TEST",
        "token_type": "bot",
        "scope": "chat:write,app_mentions:read,im:history,im:write,commands",
        "bot_user_id": "U0KRQLJ9H",
        "app_id": "A0KRD7HC3",
        "team": { "id": "T9TK3CUKW", "name": "Slack Pickleball Team" }
    }"#;

    let resp = parse_oauth_v2_access(body).expect("bot-token response parses");
    assert!(resp.ok);
    assert_eq!(resp.token_type, "bot");
    assert!(resp.access_token.starts_with("xoxb-"));
    assert_eq!(resp.team.as_ref().unwrap().id, "T9TK3CUKW");
    // The captured token must carry every minimal scope. The manifest is
    // declarative; the response is the proof Slack actually granted them.
    let granted: Vec<&str> = resp.scope.split(',').collect();
    for required in MINIMAL_BOT_SCOPES {
        assert!(
            granted.contains(required),
            "captured response is missing required scope {required}; got {granted:?}"
        );
    }
}

// (c) workspace identity write hits channel_identities with the team id
#[tokio::test]
async fn install_persists_workspace_identity_with_team_id() {
    let (db, _dir) = setup_db_with_owner("owner_alpha").await;

    let was_new = db
        .upsert_channel_identity("slack", "T9TK3CUKW", "owner_alpha")
        .await
        .expect("upsert succeeds");
    assert!(
        was_new,
        "first call must report inserted=true so the operator sees \"registered (new)\""
    );

    // Resolve must round-trip the deployment owner.
    let resolved = db
        .resolve_channel_identity("slack", "T9TK3CUKW")
        .await
        .expect("resolve succeeds")
        .expect("identity exists after install");
    assert_eq!(resolved.as_str(), "owner_alpha");

    // Channel name is normalised to lowercase per the table CHECK constraint.
    let resolved_lower = db
        .resolve_channel_identity("SLACK", "T9TK3CUKW")
        .await
        .expect("resolve succeeds for case-variant input")
        .expect("identity is found regardless of channel-name case");
    assert_eq!(resolved_lower.as_str(), "owner_alpha");
}

// (d) duplicate install of the same workspace id reuses the existing identity
#[tokio::test]
async fn install_is_idempotent_for_duplicate_workspace_id() {
    let (db, _dir) = setup_db_with_owner("owner_beta").await;

    let first = db
        .upsert_channel_identity("slack", "T0DUP", "owner_beta")
        .await
        .unwrap();
    assert!(first, "first install reports inserted=true");

    let second = db
        .upsert_channel_identity("slack", "T0DUP", "owner_beta")
        .await
        .unwrap();
    assert!(
        !second,
        "duplicate install must report inserted=false (existing row reused), got true"
    );

    // No phantom second row was written. The UNIQUE (channel, external_id)
    // constraint would have errored anyway — this assertion catches the
    // regression where we would have INSERT'd to a different unique key
    // (e.g. forgetting to lowercase the channel column).
    let resolved = db
        .resolve_channel_identity("slack", "T0DUP")
        .await
        .unwrap()
        .expect("identity still resolves after duplicate install");
    assert_eq!(resolved.as_str(), "owner_beta");
}
