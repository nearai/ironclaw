use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironclaw::agent::SessionManager;
use ironclaw::channels::IncomingMessage;
use ironclaw::channels::web::auth::{MultiAuthState, UserIdentity};
use ironclaw::channels::web::test_helpers::TestGatewayBuilder;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::{ConversationStore, Database, UserRecord, UserStore, WorkspaceMgmtStore};
use serde_json::json;
use uuid::Uuid;

const ALICE_TOKEN: &str = "tok-alice-workspace";
const BOB_TOKEN: &str = "tok-bob-workspace";
const CHARLIE_TOKEN: &str = "tok-charlie-workspace";
const ALICE_USER_ID: &str = "alice";
const BOB_USER_ID: &str = "bob";
const CHARLIE_USER_ID: &str = "charlie";

fn workspace_auth() -> MultiAuthState {
    let mut tokens = HashMap::new();
    tokens.insert(
        ALICE_TOKEN.to_string(),
        UserIdentity {
            user_id: ALICE_USER_ID.to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );
    tokens.insert(
        BOB_TOKEN.to_string(),
        UserIdentity {
            user_id: BOB_USER_ID.to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );
    tokens.insert(
        CHARLIE_TOKEN.to_string(),
        UserIdentity {
            user_id: CHARLIE_USER_ID.to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );
    MultiAuthState::multi(tokens)
}

fn test_user(id: &str, role: &str) -> UserRecord {
    let now = Utc::now();
    UserRecord {
        id: id.to_string(),
        email: Some(format!("{id}@example.com")),
        display_name: id.to_string(),
        status: "active".to_string(),
        role: role.to_string(),
        created_at: now,
        updated_at: now,
        last_login_at: None,
        created_by: None,
        metadata: serde_json::Value::Null,
    }
}

async fn setup_store() -> Arc<LibSqlBackend> {
    let store = Arc::new(LibSqlBackend::new_memory().await.unwrap());
    store.run_migrations().await.unwrap();
    store
        .create_user(&test_user(ALICE_USER_ID, "admin"))
        .await
        .unwrap();
    store
        .create_user(&test_user(BOB_USER_ID, "member"))
        .await
        .unwrap();
    store
        .create_user(&test_user(CHARLIE_USER_ID, "member"))
        .await
        .unwrap();
    store
}

async fn start_workspace_server(
    store: Arc<LibSqlBackend>,
    msg_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
) -> SocketAddr {
    let mut builder = TestGatewayBuilder::new()
        .store(store.clone())
        .session_manager(Arc::new(SessionManager::new()));
    if let Some(tx) = msg_tx {
        builder = builder.msg_tx(tx);
    }
    let (addr, _state) = builder.start_multi(workspace_auth()).await.unwrap();
    addr
}

#[tokio::test]
async fn workspace_membership_endpoints_enforce_membership_and_admin_updates() {
    let store = setup_store().await;
    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    let create_resp = client
        .post(format!("http://{addr}/api/workspaces"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({
            "name": "Team Space",
            "slug": "team-space",
            "description": "shared",
            "settings": { "color": "green" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 200);

    let bob_forbidden = client
        .get(format!("http://{addr}/api/workspaces/team-space"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_forbidden.status(), 403);

    let add_member = client
        .put(format!(
            "http://{addr}/api/workspaces/team-space/members/{BOB_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({ "role": "member" }))
        .send()
        .await
        .unwrap();
    assert_eq!(add_member.status(), 204);

    let bob_detail = client
        .get(format!("http://{addr}/api/workspaces/team-space"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_detail.status(), 200);
    let detail_json: serde_json::Value = bob_detail.json().await.unwrap();
    assert_eq!(detail_json["slug"], "team-space");
    assert_eq!(detail_json["role"], "member");

    let members_resp = client
        .get(format!("http://{addr}/api/workspaces/team-space/members"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(members_resp.status(), 200);
    let members_json: serde_json::Value = members_resp.json().await.unwrap();
    let members = members_json["members"].as_array().unwrap();
    assert!(
        members
            .iter()
            .any(|member| member["user_id"] == ALICE_USER_ID)
    );
    assert!(
        members
            .iter()
            .any(|member| member["user_id"] == BOB_USER_ID)
    );
}

#[tokio::test]
async fn workspace_creation_rejects_invalid_slug() {
    let store = setup_store().await;
    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("http://{addr}/api/workspaces"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({
            "name": "Bad Slug",
            "slug": "-bad-slug",
            "description": "",
            "settings": {}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    assert!(
        resp.text()
            .await
            .unwrap()
            .contains("Workspace slug must match"),
        "expected slug validation error"
    );
}

#[tokio::test]
async fn workspace_member_updates_validate_roles_and_owner_permissions() {
    let store = setup_store().await;
    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    let create_resp = client
        .post(format!("http://{addr}/api/workspaces"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({
            "name": "Owner Rules",
            "slug": "owner-rules",
            "description": "",
            "settings": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 200);

    let invalid_role = client
        .put(format!(
            "http://{addr}/api/workspaces/owner-rules/members/{CHARLIE_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({ "role": "superadmin" }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_role.status(), 400);

    let add_admin = client
        .put(format!(
            "http://{addr}/api/workspaces/owner-rules/members/{BOB_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({ "role": "admin" }))
        .send()
        .await
        .unwrap();
    assert_eq!(add_admin.status(), 204);

    let bob_self_promote = client
        .put(format!(
            "http://{addr}/api/workspaces/owner-rules/members/{BOB_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .json(&json!({ "role": "owner" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_self_promote.status(), 403);

    let bob_promote_other = client
        .put(format!(
            "http://{addr}/api/workspaces/owner-rules/members/{CHARLIE_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .json(&json!({ "role": "owner" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_promote_other.status(), 403);
}

#[tokio::test]
async fn workspace_cannot_remove_or_demote_last_owner() {
    let store = setup_store().await;
    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    let create_resp = client
        .post(format!("http://{addr}/api/workspaces"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({
            "name": "Single Owner",
            "slug": "single-owner",
            "description": "",
            "settings": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 200);

    let demote_resp = client
        .put(format!(
            "http://{addr}/api/workspaces/single-owner/members/{ALICE_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({ "role": "admin" }))
        .send()
        .await
        .unwrap();
    assert_eq!(demote_resp.status(), 409);

    let delete_resp = client
        .delete(format!(
            "http://{addr}/api/workspaces/single-owner/members/{ALICE_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_resp.status(), 409);
}

#[tokio::test]
async fn archived_workspace_returns_gone_for_scoped_endpoints() {
    let store = setup_store().await;
    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    client
        .post(format!("http://{addr}/api/workspaces"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({
            "name": "Archive Me",
            "slug": "archive-me",
            "description": "",
            "settings": {}
        }))
        .send()
        .await
        .unwrap();

    client
        .put(format!(
            "http://{addr}/api/workspaces/archive-me/members/{BOB_USER_ID}"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({ "role": "member" }))
        .send()
        .await
        .unwrap();

    let archive_resp = client
        .post(format!("http://{addr}/api/workspaces/archive-me/archive"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(archive_resp.status(), 204);

    let settings_resp = client
        .get(format!("http://{addr}/api/settings?workspace=archive-me"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(settings_resp.status(), 410);

    let chat_resp = client
        .post(format!("http://{addr}/api/chat/send?workspace=archive-me"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .json(&json!({ "content": "hello" }))
        .send()
        .await
        .unwrap();
    assert_eq!(chat_resp.status(), 410);
}

#[tokio::test]
async fn archived_workspaces_are_hidden_from_workspace_lists() {
    let store = setup_store().await;
    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    let create_resp = client
        .post(format!("http://{addr}/api/workspaces"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .json(&json!({
            "name": "Archive Hidden",
            "slug": "archive-hidden",
            "description": "",
            "settings": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 200);

    let archive_resp = client
        .post(format!(
            "http://{addr}/api/workspaces/archive-hidden/archive"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(archive_resp.status(), 204);

    let list_resp = client
        .get(format!("http://{addr}/api/workspaces"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);
    let list_json: serde_json::Value = list_resp.json().await.unwrap();
    let workspaces = list_json["workspaces"].as_array().unwrap();
    assert!(
        !workspaces
            .iter()
            .any(|workspace| workspace["slug"] == "archive-hidden"),
        "archived workspaces should not be listed"
    );
}

#[tokio::test]
async fn workspace_scoped_chat_send_sets_workspace_id_on_forwarded_message() {
    let store = setup_store().await;
    let workspace = store
        .create_workspace("Scoped Chat", "scoped-chat", "", ALICE_USER_ID, &json!({}))
        .await
        .unwrap();
    store
        .add_workspace_member(workspace.id, BOB_USER_ID, "member", Some(ALICE_USER_ID))
        .await
        .unwrap();

    let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel(8);
    let addr = start_workspace_server(store, Some(agent_tx)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("http://{addr}/api/chat/send?workspace=scoped-chat"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .json(&json!({ "content": "workspace hello" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    let msg = tokio::time::timeout(Duration::from_secs(2), agent_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let workspace_id = workspace.id.to_string();
    assert_eq!(msg.content, "workspace hello");
    assert_eq!(msg.workspace_id.as_deref(), Some(workspace_id.as_str()));
    assert_eq!(
        msg.metadata
            .get("workspace_id")
            .and_then(|value| value.as_str()),
        Some(workspace_id.as_str())
    );
}

#[tokio::test]
async fn workspace_scoped_settings_are_separate_from_personal_settings() {
    let store = setup_store().await;
    let workspace = store
        .create_workspace(
            "Scoped Settings",
            "scoped-settings",
            "",
            ALICE_USER_ID,
            &json!({}),
        )
        .await
        .unwrap();
    store
        .add_workspace_member(workspace.id, BOB_USER_ID, "member", Some(ALICE_USER_ID))
        .await
        .unwrap();

    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    let personal_set = client
        .put(format!("http://{addr}/api/settings/theme"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .json(&json!({ "value": "personal-dark" }))
        .send()
        .await
        .unwrap();
    assert_eq!(personal_set.status(), 204);

    let workspace_set = client
        .put(format!(
            "http://{addr}/api/settings/theme?workspace=scoped-settings"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .json(&json!({ "value": "workspace-light" }))
        .send()
        .await
        .unwrap();
    assert_eq!(workspace_set.status(), 204);

    let personal_get = client
        .get(format!("http://{addr}/api/settings/theme"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(personal_get.status(), 200);
    let personal_json: serde_json::Value = personal_get.json().await.unwrap();
    assert_eq!(personal_json["value"], "personal-dark");

    let workspace_get = client
        .get(format!(
            "http://{addr}/api/settings/theme?workspace=scoped-settings"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(workspace_get.status(), 200);
    let workspace_json: serde_json::Value = workspace_get.json().await.unwrap();
    assert_eq!(workspace_json["value"], "workspace-light");
}

#[tokio::test]
async fn workspace_threads_do_not_appear_in_personal_thread_listing() {
    let store = setup_store().await;
    let workspace = store
        .create_workspace(
            "Scoped Threads",
            "scoped-threads",
            "",
            ALICE_USER_ID,
            &json!({}),
        )
        .await
        .unwrap();
    store
        .add_workspace_member(workspace.id, BOB_USER_ID, "member", Some(ALICE_USER_ID))
        .await
        .unwrap();

    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();

    let new_thread = client
        .post(format!(
            "http://{addr}/api/chat/thread/new?workspace=scoped-threads"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(new_thread.status(), 200);
    let new_thread_json: serde_json::Value = new_thread.json().await.unwrap();
    let scoped_thread_id = new_thread_json["id"].as_str().unwrap().to_string();

    let workspace_threads = client
        .get(format!(
            "http://{addr}/api/chat/threads?workspace=scoped-threads"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(workspace_threads.status(), 200);
    let workspace_threads_json: serde_json::Value = workspace_threads.json().await.unwrap();
    let workspace_thread_ids: Vec<&str> = workspace_threads_json["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|thread| thread["id"].as_str())
        .collect();
    assert!(workspace_thread_ids.contains(&scoped_thread_id.as_str()));

    let personal_threads = client
        .get(format!("http://{addr}/api/chat/threads"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(personal_threads.status(), 200);
    let personal_threads_json: serde_json::Value = personal_threads.json().await.unwrap();
    let personal_thread_ids: Vec<&str> = personal_threads_json["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|thread| thread["id"].as_str())
        .collect();
    assert!(!personal_thread_ids.contains(&scoped_thread_id.as_str()));
}

#[tokio::test]
async fn responses_api_fetch_uses_workspace_scope_for_workspace_threads() {
    let store = setup_store().await;
    let workspace = store
        .create_workspace(
            "Responses Scope",
            "responses-scope",
            "",
            ALICE_USER_ID,
            &json!({}),
        )
        .await
        .unwrap();
    store
        .add_workspace_member(workspace.id, BOB_USER_ID, "member", Some(ALICE_USER_ID))
        .await
        .unwrap();

    let thread_id = store
        .create_conversation_with_metadata(
            "gateway",
            BOB_USER_ID,
            Some(workspace.id),
            &json!({ "thread_type": "assistant" }),
        )
        .await
        .unwrap();
    store
        .add_conversation_message(thread_id, "assistant", "workspace response")
        .await
        .unwrap();

    let addr = start_workspace_server(store, None).await;
    let client = reqwest::Client::new();
    let response_id = format!("resp_{}{}", Uuid::new_v4().simple(), thread_id.simple());

    let member_resp = client
        .get(format!("http://{addr}/v1/responses/{response_id}"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(member_resp.status(), 200);
    let member_json: serde_json::Value = member_resp.json().await.unwrap();
    assert_eq!(member_json["id"], response_id);
    assert_eq!(member_json["status"], "completed");
    assert_eq!(
        member_json["output"][0]["content"][0]["text"],
        "workspace response"
    );

    let outsider_resp = client
        .get(format!("http://{addr}/v1/responses/{response_id}"))
        .header("Authorization", format!("Bearer {CHARLIE_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(outsider_resp.status(), 404);
}
