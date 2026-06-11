//! Cross-tenant authz regression for `POST /v1/responses`.
//!
//! `create_response_handler` decodes a `thread_uuid` from the caller's
//! `previous_response_id` and stamps it into outbound
//! `notify_thread_id` / `client_thread_id`. Without an ownership check,
//! Alice could POST Bob's `previous_response_id` and have callbacks
//! tagged to Bob's thread. The handler mirrors the GET-path
//! `conversation_belongs_to_user` check and 404s on a foreign id
//! (404 not 403 — don't leak existence); it fails closed when
//! `state.store` is `None`.
//!
//! Drives the handler over real HTTP with two tokens:
//!   1. Alice's token + Bob's response id ⇒ 404, no `IncomingMessage`.
//!   2. Alice's token + her own response id ⇒ passes; message lands on
//!      `msg_tx` (pins against over-rotating to "404 everything").
//!
//! libsql-gated: needs a real DB to seed conversations and to satisfy
//! the fail-closed `state.store.is_some()` requirement.

#![cfg(feature = "libsql")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ironclaw::channels::IncomingMessage;
use ironclaw::channels::web::auth::{MultiAuthState, UserIdentity};
use ironclaw::channels::web::platform::router::start_server;
use ironclaw::channels::web::platform::state::GatewayState;
use ironclaw::channels::web::test_helpers::TestGatewayBuilder;
use ironclaw::db::Database;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

const ALICE_TOKEN: &str = "tok-alice-responses-cross-tenant";
const BOB_TOKEN: &str = "tok-bob-responses-cross-tenant";
const ALICE_USER_ID: &str = "alice-responses-cross-tenant";
const BOB_USER_ID: &str = "bob-responses-cross-tenant";

struct ServerGuard {
    shutdown: Option<oneshot::Sender<()>>,
    _tmp: tempfile::TempDir,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

fn two_user_auth() -> MultiAuthState {
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
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );
    MultiAuthState::multi(tokens)
}

/// Spin up the gateway with a real libSQL store + msg_tx capture so
/// tests can assert both the negative (no enqueue) and positive
/// (enqueued past the gate) outcomes of the authz check.
async fn start_test_server() -> (
    SocketAddr,
    Arc<GatewayState>,
    Arc<dyn Database>,
    mpsc::Receiver<IncomingMessage>,
    ServerGuard,
) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("responses_cross_tenant.db");
    let backend = ironclaw::db::libsql::LibSqlBackend::new_local(&path)
        .await
        .expect("libsql backend");
    backend.run_migrations().await.expect("migrations");
    let db: Arc<dyn Database> = Arc::new(backend);

    let (tx, rx) = mpsc::channel::<IncomingMessage>(16);
    let state = TestGatewayBuilder::new()
        .user_id(ALICE_USER_ID)
        .msg_tx(tx)
        .store(Arc::clone(&db))
        .build();

    let auth = two_user_auth();
    let addr: SocketAddr = "127.0.0.1:0".parse().expect("addr");
    let bound = start_server(addr, state.clone(), auth.into())
        .await
        .expect("start_server");
    let shutdown = state.shutdown_tx.write().await.take();
    (
        bound,
        state,
        db,
        rx,
        ServerGuard {
            shutdown,
            _tmp: tmp,
        },
    )
}

async fn seed_conversation(db: &Arc<dyn Database>, user_id: &str) -> Uuid {
    db.create_conversation_with_metadata(
        "gateway",
        user_id,
        &serde_json::json!({ "title": format!("{user_id}'s thread") }),
    )
    .await
    .expect("create conversation")
}

/// Reconstruct a Responses-API response id pointing at a given thread.
/// Format is documented on `encode_response_id` in `responses_api.rs`:
/// `resp_{response_uuid_hex}{thread_uuid_hex}` with both UUIDs in
/// `simple()` (32-char, no-hyphen) form. We build it directly so the
/// test does not need to be in the same crate as the private helper.
fn make_response_id(thread_uuid: Uuid) -> String {
    let response_uuid = Uuid::new_v4();
    format!("resp_{}{}", response_uuid.simple(), thread_uuid.simple())
}

fn client() -> reqwest::Client {
    // Short client-side timeout: the same-user path passes the gate
    // and then waits for SSE events that never arrive in this fixture.
    // We assert the captured msg on msg_tx, not the HTTP response.
    reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .expect("http client")
}

/// Alice's token + Bob's `previous_response_id` ⇒ **404
/// `invalid_request_error`**, and no `IncomingMessage` enqueued.
/// Regression: pre-#3669 this returned 200 and tagged callbacks to
/// Bob's `notify_thread_id`. Post-Z1, the new fail-closed gate must
/// reject before any agent-loop side effects.
#[tokio::test]
async fn alice_with_bobs_previous_response_id_returns_404_and_does_not_enqueue() {
    let (addr, _state, db, mut rx, _guard) = start_test_server().await;
    let bob_thread = seed_conversation(&db, BOB_USER_ID).await;
    let foreign_resp_id = make_response_id(bob_thread);
    let url = format!("http://{}/v1/responses", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(ALICE_TOKEN)
        .json(&serde_json::json!({
            "model": "default",
            "input": "hello",
            "previous_response_id": foreign_resp_id,
        }))
        .send()
        .await
        .expect("send /v1/responses request");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "foreign previous_response_id must be rejected with 404",
    );
    let body: serde_json::Value = resp.json().await.expect("parse JSON body");
    let kind = body
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    assert_eq!(
        kind, "invalid_request_error",
        "error.type for foreign previous_response_id should be \
         invalid_request_error, body={body}",
    );

    // No IncomingMessage may have been enqueued on the agent channel
    // before the rejection.
    match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
        Ok(Some(msg)) => panic!(
            "rejected cross-tenant request must not enqueue an \
             IncomingMessage; got metadata: {:?}",
            msg.metadata
        ),
        Ok(None) => panic!("agent channel must not be closed"),
        Err(_) => {} // timeout = nothing enqueued, expected
    }
}

/// Same-user pin: Alice's token + Alice's own `previous_response_id`
/// must pass the gate. Without this case, over-rotating the check to
/// "404 every previous_response_id" would still satisfy the negative
/// test above.
///
/// We assert the `IncomingMessage` lands on the captured `msg_tx`,
/// which proves the request reached the agent-loop dispatch past the
/// authz check. The HTTP response itself isn't asserted because the
/// handler then waits for SSE events that never arrive in this
/// fixture and the request times out from the client side.
#[tokio::test]
async fn alice_with_own_previous_response_id_passes_authz_and_enqueues() {
    let (addr, _state, db, mut rx, _guard) = start_test_server().await;
    let alice_thread = seed_conversation(&db, ALICE_USER_ID).await;
    let own_resp_id = make_response_id(alice_thread);
    let url = format!("http://{}/v1/responses", addr);

    let http = client();
    let request = async move {
        // Don't care about the HTTP response — handler will time out
        // waiting for SSE. We only care that the message reaches the
        // agent channel, which happens before the SSE wait.
        let _ = http
            .post(&url)
            .bearer_auth(ALICE_TOKEN)
            .json(&serde_json::json!({
                "model": "default",
                "input": "hello",
                "previous_response_id": own_resp_id,
            }))
            .send()
            .await;
    };
    let captured = async {
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("agent channel must receive a message within 2s")
            .expect("agent channel must not be closed")
    };

    let (_, msg) = tokio::join!(request, captured);

    let thread_id = msg
        .metadata
        .get("thread_id")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("captured message missing thread_id: {}", msg.metadata));
    assert_eq!(
        thread_id,
        alice_thread.to_string(),
        "same-user request must reach the agent loop carrying Alice's \
         thread_uuid, proving the authz gate passed",
    );
}
