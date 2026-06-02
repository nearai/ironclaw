//! Caller-level tests for the WebChat v2 NEAR wallet login surface.
//!
//! Drives the unauthenticated `Router` returned by
//! [`webui_v2_auth_router`] through `tower::ServiceExt::oneshot`, with
//! the `NearLoginProvider`'s `view_access_key` RPC pointed at a local
//! mock NEAR RPC server. Per `.claude/rules/testing.md` "Test Through
//! the Caller, Not Just the Helper", the side effect we care about
//! (session creation, status mapping, replay rejection) is
//! end-of-pipeline; the NEP-413 helper has its own unit tests but they
//! wouldn't catch a wrapper that forgets to consume the nonce or maps
//! an error to the wrong status.
//!
//! Gated on `dev-in-memory-session` because the test wires
//! `InMemorySessionStore` + `EmailUserDirectory` and the provider's
//! `with_rpc_endpoint` test constructor — all behind that feature.

#![cfg(feature = "dev-in-memory-session")]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use base64::Engine;
use chrono::Duration as ChronoDuration;
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use ironclaw_host_api::TenantId;
use ironclaw_reborn_webui_ingress::{
    EmailUserDirectory, InMemorySessionStore, NearLoginProvider, NearNetwork, OAuthRouterConfig,
    SessionStore, webui_v2_auth_router,
};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::Deserialize;
use tower::ServiceExt;

// ── NEP-413 payload construction (mirrors the provider's verify.rs) ──

const NEP413_TAG: u32 = (1 << 31) + 413;

/// v1 (spec) field order: tag → message → nonce → recipient → callback.
fn nep413_v1(message: &str, nonce: &[u8; 32], recipient: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&NEP413_TAG.to_le_bytes());
    buf.extend_from_slice(&(message.len() as u32).to_le_bytes());
    buf.extend_from_slice(message.as_bytes());
    buf.extend_from_slice(nonce);
    buf.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
    buf.extend_from_slice(recipient.as_bytes());
    buf.push(0);
    buf
}

fn random_key() -> SigningKey {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    SigningKey::from_bytes(&b)
}

// ── mock NEAR RPC ─────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum RpcMode {
    /// `view_access_key` finds an active key.
    Found,
    /// RPC-level error (key absent / wrong network).
    AccessKeyError,
    /// Transport / server fault (HTTP 500).
    ServerError,
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn spawn_mock_rpc(mode: RpcMode) -> (String, AbortOnDrop) {
    let router = axum::Router::new().route(
        "/",
        post(move |_: Json<serde_json::Value>| async move {
            match mode {
                RpcMode::Found => Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": "ironclaw",
                    "result": { "nonce": 1, "permission": "FullAccess", "block_height": 1, "block_hash": "x" }
                }))
                .into_response(),
                RpcMode::AccessKeyError => Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": "ironclaw",
                    "error": { "cause": { "name": "UNKNOWN_ACCESS_KEY" }, "name": "HANDLER_ERROR" }
                }))
                .into_response(),
                RpcMode::ServerError => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}/"), AbortOnDrop(handle))
}

use axum::response::IntoResponse;

// ── router construction ───────────────────────────────────────────────

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("tenant")
}

fn build_router_with_near(
    near: Option<Arc<NearLoginProvider>>,
    store: Arc<dyn SessionStore>,
) -> axum::Router {
    let mut config = OAuthRouterConfig::new(
        tenant(),
        store,
        Arc::new(EmailUserDirectory),
        Vec::new(),
        "https://gateway.example",
    )
    .with_session_lifetime(ChronoDuration::hours(1));
    if let Some(near) = near {
        config = config.with_near_provider(near);
    }
    webui_v2_auth_router(config).router
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.expect("collect").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[derive(Deserialize)]
struct ProvidersResponse {
    providers: Vec<String>,
}

#[derive(Deserialize)]
struct ChallengeResponse {
    nonce: String,
    message: String,
    recipient: String,
    network: String,
}

#[derive(Deserialize)]
struct VerifyResponse {
    token: String,
}

async fn get(router: &axum::Router, uri: &str) -> axum::http::Response<Body> {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("oneshot")
}

async fn post_json(
    router: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("req"),
        )
        .await
        .expect("oneshot")
}

/// Run the full challenge → sign → verify flow and return the verify
/// response. `account_id` is the NEAR account claimed; `signer` is the
/// keypair the wallet uses; `tamper_nonce` overrides the nonce echoed
/// back on verify (to exercise replay / unknown-nonce paths).
async fn challenge_and_verify(
    router: &axum::Router,
    account_id: &str,
    signer: &SigningKey,
) -> axum::http::Response<Body> {
    let resp = get(router, "/auth/near/challenge").await;
    assert_eq!(resp.status(), StatusCode::OK, "challenge must succeed");
    let challenge: ChallengeResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("challenge json");

    let nonce_bytes_vec = hex::decode(&challenge.nonce).expect("hex nonce");
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes.copy_from_slice(&nonce_bytes_vec);
    let sig = signer.sign(&nep413_v1(
        &challenge.message,
        &nonce_bytes,
        &challenge.recipient,
    ));

    let public_key = format!(
        "ed25519:{}",
        bs58::encode(signer.verifying_key().as_bytes()).into_string()
    );
    let signature = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    post_json(
        router,
        "/auth/near/verify",
        serde_json::json!({
            "account_id": account_id,
            "public_key": public_key,
            "signature": signature,
            "nonce": challenge.nonce,
        }),
    )
    .await
}

// ── tests ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn providers_lists_near_when_configured() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::Found).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Testnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    let resp = get(&router, "/auth/providers").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let payload: ProvidersResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    assert!(payload.providers.contains(&"near".to_string()));
}

#[tokio::test]
async fn providers_omits_near_when_not_configured() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(None, store);
    let resp = get(&router, "/auth/providers").await;
    let payload: ProvidersResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    assert!(!payload.providers.contains(&"near".to_string()));
}

#[tokio::test]
async fn near_routes_404_when_not_configured() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(None, store);

    let challenge = get(&router, "/auth/near/challenge").await;
    assert_eq!(challenge.status(), StatusCode::NOT_FOUND);

    let verify = post_json(
        &router,
        "/auth/near/verify",
        serde_json::json!({
            "account_id": "alice.testnet",
            "public_key": "ed25519:11111111111111111111111111111111",
            "signature": "AA",
            "nonce": "00",
        }),
    )
    .await;
    assert_eq!(verify.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn challenge_returns_nonce_message_and_network() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::Found).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Testnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    let resp = get(&router, "/auth/near/challenge").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let challenge: ChallengeResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    assert_eq!(challenge.nonce.len(), 64, "32-byte nonce hex-encoded");
    assert!(challenge.message.contains(&challenge.nonce));
    assert_eq!(challenge.recipient, "ironclaw");
    assert_eq!(challenge.network, "testnet");
}

#[tokio::test]
async fn full_flow_mints_session_bound_to_account() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::Found).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Mainnet, rpc).expect("near"));
    let store = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store.clone());

    let signer = random_key();
    let resp = challenge_and_verify(&router, "alice.near", &signer).await;
    assert_eq!(resp.status(), StatusCode::OK, "verify must succeed");
    let verify: VerifyResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    assert!(!verify.token.is_empty());

    // The bearer must resolve to a live session for the NEAR account
    // (EmailUserDirectory maps a NEAR profile to `near:<account_id>`).
    let record = store
        .lookup(&verify.token)
        .await
        .expect("lookup")
        .expect("session present");
    assert_eq!(record.user_id.as_str(), "near:alice.near");
}

#[tokio::test]
async fn replayed_nonce_is_rejected() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::Found).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Testnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    // First obtain a valid challenge + signature, capture the request,
    // and replay the exact same body twice.
    let resp = get(&router, "/auth/near/challenge").await;
    let challenge: ChallengeResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes.copy_from_slice(&hex::decode(&challenge.nonce).expect("hex"));
    let signer = random_key();
    let sig = signer.sign(&nep413_v1(
        &challenge.message,
        &nonce_bytes,
        &challenge.recipient,
    ));
    let req = serde_json::json!({
        "account_id": "alice.testnet",
        "public_key": format!("ed25519:{}", bs58::encode(signer.verifying_key().as_bytes()).into_string()),
        "signature": base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
        "nonce": challenge.nonce,
    });

    let first = post_json(&router, "/auth/near/verify", req.clone()).await;
    assert_eq!(first.status(), StatusCode::OK, "first verify succeeds");
    let second = post_json(&router, "/auth/near/verify", req).await;
    assert_eq!(
        second.status(),
        StatusCode::BAD_REQUEST,
        "replayed nonce must fail closed",
    );
}

#[tokio::test]
async fn unknown_nonce_is_rejected() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::Found).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Testnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    // A well-formed but never-issued nonce — fails before any crypto.
    let signer = random_key();
    let nonce = hex::encode([3u8; 32]);
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes.copy_from_slice(&hex::decode(&nonce).expect("hex"));
    let message = format!("Sign in to IronClaw\nNonce: {nonce}");
    let sig = signer.sign(&nep413_v1(&message, &nonce_bytes, "ironclaw"));
    let resp = post_json(
        &router,
        "/auth/near/verify",
        serde_json::json!({
            "account_id": "alice.testnet",
            "public_key": format!("ed25519:{}", bs58::encode(signer.verifying_key().as_bytes()).into_string()),
            "signature": base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
            "nonce": nonce,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invalid_signature_is_rejected_401() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::Found).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Testnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    // Get a real challenge but sign with a key that does not match the
    // public key we send — signature verification must fail.
    let resp = get(&router, "/auth/near/challenge").await;
    let challenge: ChallengeResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes.copy_from_slice(&hex::decode(&challenge.nonce).expect("hex"));
    let real_signer = random_key();
    let attacker = random_key();
    let sig = attacker.sign(&nep413_v1(
        &challenge.message,
        &nonce_bytes,
        &challenge.recipient,
    ));
    let resp = post_json(
        &router,
        "/auth/near/verify",
        serde_json::json!({
            "account_id": "alice.testnet",
            // public key of the honest signer, signature from attacker
            "public_key": format!("ed25519:{}", bs58::encode(real_signer.verifying_key().as_bytes()).into_string()),
            "signature": base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
            "nonce": challenge.nonce,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_network_access_key_is_rejected_401() {
    // The RPC reports the key is not a known access key (the shape a
    // wrong-network or non-existent key produces). Verify must 401.
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::AccessKeyError).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Mainnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    let signer = random_key();
    let resp = challenge_and_verify(&router, "alice.near", &signer).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rpc_backend_failure_returns_503() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::ServerError).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Mainnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    let signer = random_key();
    let resp = challenge_and_verify(&router, "alice.near", &signer).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn malformed_public_key_is_rejected_400() {
    let (rpc, _guard) = spawn_mock_rpc(RpcMode::Found).await;
    let near =
        Arc::new(NearLoginProvider::with_rpc_endpoint(NearNetwork::Testnet, rpc).expect("near"));
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router_with_near(Some(near), store);

    // Valid nonce, but a public key missing the `ed25519:` prefix.
    let resp = get(&router, "/auth/near/challenge").await;
    let challenge: ChallengeResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    let resp = post_json(
        &router,
        "/auth/near/verify",
        serde_json::json!({
            "account_id": "alice.testnet",
            "public_key": "not-a-key",
            "signature": base64::engine::general_purpose::STANDARD.encode([0u8; 64]),
            "nonce": challenge.nonce,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
