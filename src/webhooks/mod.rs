//! Generic webhook ingress for tools.
//!
//! Exposes `/webhook/tools/{tool}` so external webhook providers can POST
//! payloads that are normalized by the target tool into `system_event`s.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::agent::routine_engine::RoutineEngine;
use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::ToolRegistry;

/// Shared routine engine slot, populated by Agent after startup.
pub type RoutineEngineSlot = Arc<tokio::sync::RwLock<Option<Arc<RoutineEngine>>>>;

/// Shared state for the generic tools webhook ingress.
#[derive(Clone)]
pub struct ToolWebhookState {
    pub tools: Arc<ToolRegistry>,
    pub routine_engine: RoutineEngineSlot,
    pub user_id: String,
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

#[derive(Debug, Serialize)]
struct ToolWebhookResponse {
    status: &'static str,
    tool: String,
    emitted_events: usize,
    fired_routines: usize,
}

#[derive(Debug, Deserialize)]
struct ToolWebhookOutput {
    #[serde(default)]
    emit_events: Vec<SystemEventIntent>,
}

#[derive(Debug, Deserialize)]
struct SystemEventIntent {
    source: String,
    event_type: String,
    #[serde(default)]
    payload: serde_json::Value,
}

const MAX_WEBHOOK_BODY_BYTES: usize = 64 * 1024;

/// Build routes for tool-driven webhook ingestion.
pub fn routes(state: ToolWebhookState) -> Router {
    Router::new()
        .route("/webhook/tools/{tool}", post(tool_webhook_handler))
        .route(
            "/webhook/tools/{tool}/{*rest}",
            post(tool_webhook_with_rest_handler),
        )
        .route("/webhook/tools/{tool}", get(tool_webhook_health))
        .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY_BYTES))
        .with_state(state)
}

async fn tool_webhook_health(
    Path(tool): Path<String>,
    State(state): State<ToolWebhookState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(tool_impl) = state.tools.get(&tool).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Tool not found: {tool}") })),
        );
    };
    if tool_impl.webhook_capability().is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Tool does not support webhooks: {tool}") })),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "tool": tool })),
    )
}

async fn tool_webhook_handler(
    Path(tool): Path<String>,
    State(state): State<ToolWebhookState>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    tool_webhook_handler_inner(tool, None, state, method, headers, query, body).await
}

async fn tool_webhook_with_rest_handler(
    Path((tool, rest)): Path<(String, String)>,
    State(state): State<ToolWebhookState>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    tool_webhook_handler_inner(tool, Some(rest), state, method, headers, query, body).await
}

async fn tool_webhook_handler_inner(
    tool: String,
    rest: Option<String>,
    state: ToolWebhookState,
    method: Method,
    headers: HeaderMap,
    query: HashMap<String, String>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    if body.len() > MAX_WEBHOOK_BODY_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": format!("Webhook body exceeds {} bytes", MAX_WEBHOOK_BODY_BYTES)
            })),
        );
    }

    let Some(tool_impl) = state.tools.get(&tool).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Tool not found: {tool}") })),
        );
    };

    if let Err(msg) = validate_webhook_auth(
        &*tool_impl,
        state.secrets_store.as_deref(),
        &state.user_id,
        &headers,
        &body,
    )
    .await
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": msg })),
        );
    }

    let body_json: Option<serde_json::Value> = serde_json::from_slice(&body).ok();
    let headers_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    let path = if let Some(rest) = rest.filter(|r| !r.is_empty()) {
        format!("/webhook/tools/{tool}/{rest}")
    } else {
        format!("/webhook/tools/{tool}")
    };

    let params = serde_json::json!({
        "action": "handle_webhook",
        "webhook": {
            "method": method.as_str(),
            "path": path,
            "query": query,
            "headers": headers_map,
            "body_json": body_json,
            "body_raw": String::from_utf8_lossy(&body),
        }
    });

    let ctx = JobContext::with_user(
        state.user_id.clone(),
        format!("webhook:{tool}"),
        "Process external webhook",
    );

    let output = match tool_impl.execute(params, &ctx).await {
        Ok(out) => out,
        Err(e) => {
            tracing::warn!(tool = %tool, error = %e, "Webhook tool execution failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Tool execution failed" })),
            );
        }
    };

    let parsed: ToolWebhookOutput = match serde_json::from_value(output.result) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Tool webhook response must be a JSON object (optionally with 'emit_events' array)"
                })),
            );
        }
    };

    let emitted_events = parsed.emit_events.len();
    let mut fired_routines = 0usize;
    if emitted_events > 0 {
        let Some(engine) = state.routine_engine.read().await.as_ref().cloned() else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "Routine engine not available" })),
            );
        };

        for event in parsed.emit_events {
            fired_routines += engine
                .emit_system_event(
                    &event.source,
                    &event.event_type,
                    &event.payload,
                    Some(&state.user_id),
                )
                .await;
        }
    }

    let response = ToolWebhookResponse {
        status: "accepted",
        tool,
        emitted_events,
        fired_routines,
    };
    (StatusCode::ACCEPTED, Json(serde_json::json!(response)))
}

fn header_value<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    // HeaderMap::get() already performs case-insensitive lookup per HTTP spec.
    headers.get(key).and_then(|v| v.to_str().ok())
}

async fn validate_webhook_auth(
    tool: &dyn crate::tools::Tool,
    secrets_store: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), String> {
    let Some(cfg) = tool.webhook_capability() else {
        return Err(
            "Tool does not declare a webhook capability; webhook access denied".to_string(),
        );
    };

    // Require at least one authentication mechanism to be configured.
    if cfg.secret_name.is_none()
        && cfg.signature_key_secret_name.is_none()
        && cfg.hmac_secret_name.is_none()
    {
        return Err(
            "Webhook capability misconfigured: at least one auth mechanism must be configured"
                .to_string(),
        );
    }

    let Some(store) = secrets_store else {
        return Err("Secrets store not available for webhook verification".to_string());
    };

    if let Some(secret_name) = cfg.secret_name.as_deref() {
        let expected = store
            .get_decrypted(user_id, secret_name)
            .await
            .map_err(|_| format!("Missing webhook secret '{secret_name}'"))?;
        let expected = expected.expose();
        let secret_header = cfg.secret_header.as_deref().unwrap_or("x-webhook-secret");
        let provided = header_value(headers, secret_header)
            .or_else(|| {
                if secret_header != "x-webhook-secret" {
                    header_value(headers, "x-webhook-secret")
                } else {
                    None
                }
            })
            .ok_or_else(|| "Webhook secret required".to_string())?;

        if !bool::from(expected.as_bytes().ct_eq(provided.as_bytes())) {
            return Err("Invalid webhook secret".to_string());
        }
    }

    if let Some(public_key_name) = cfg.signature_key_secret_name.as_deref() {
        let key = store
            .get_decrypted(user_id, public_key_name)
            .await
            .map_err(|_| format!("Missing signature key secret '{public_key_name}'"))?;
        let key = key.expose();
        let sig = header_value(headers, "x-signature-ed25519")
            .ok_or_else(|| "Missing signature header".to_string())?;
        let ts = header_value(headers, "x-signature-timestamp")
            .ok_or_else(|| "Missing signature timestamp header".to_string())?;
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if !crate::channels::wasm::signature::verify_discord_signature(key, sig, ts, body, now_secs)
        {
            return Err("Invalid signature".to_string());
        }
    }

    if let Some(hmac_secret_name) = cfg.hmac_secret_name.as_deref() {
        let secret = store
            .get_decrypted(user_id, hmac_secret_name)
            .await
            .map_err(|_| format!("Missing HMAC secret '{hmac_secret_name}'"))?;
        let secret = secret.expose();

        if let Some(timestamp_header) = cfg.hmac_timestamp_header.as_deref() {
            let sig_header = cfg
                .hmac_signature_header
                .as_deref()
                .unwrap_or("x-slack-signature");
            let sig = header_value(headers, sig_header)
                .ok_or_else(|| "Missing HMAC signature header".to_string())?;
            let ts = header_value(headers, timestamp_header)
                .ok_or_else(|| "Missing HMAC timestamp header".to_string())?;
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            if !crate::channels::wasm::signature::verify_slack_signature(
                secret, ts, body, sig, now_secs,
            ) {
                return Err("Invalid timestamped HMAC signature".to_string());
            }
        } else {
            let sig_header = cfg
                .hmac_signature_header
                .as_deref()
                .unwrap_or("x-hub-signature-256");
            let prefix = cfg.hmac_prefix.as_deref().unwrap_or("sha256=");
            let sig = header_value(headers, sig_header)
                .ok_or_else(|| "Missing HMAC signature header".to_string())?;
            if !crate::channels::wasm::signature::verify_hmac_sha256_prefixed(
                secret, body, sig, prefix,
            ) {
                return Err("Invalid HMAC signature".to_string());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::body::Body;
    use tower::ServiceExt;

    use crate::context::JobContext;
    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto};
    use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};

    use super::*;

    struct TestWebhookTool;
    struct ProtectedWebhookTool;
    struct HmacWebhookTool;
    /// Tool that declares webhook_capability() but with no auth mechanism configured.
    struct MisconfiguredWebhookTool;

    #[async_trait]
    impl Tool for TestWebhookTool {
        fn name(&self) -> &str {
            "test_webhook"
        }

        fn description(&self) -> &str {
            "test"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({"emit_events":[]}),
                Duration::from_millis(1),
            ))
        }
    }

    #[async_trait]
    impl Tool for ProtectedWebhookTool {
        fn name(&self) -> &str {
            "protected_webhook"
        }

        fn description(&self) -> &str {
            "protected test"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({"emit_events":[]}),
                Duration::from_millis(1),
            ))
        }

        fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
            Some(crate::tools::wasm::WebhookCapability {
                secret_name: Some("test_webhook_secret".to_string()),
                secret_header: Some("x-webhook-secret".to_string()),
                ..Default::default()
            })
        }
    }

    #[async_trait]
    impl Tool for HmacWebhookTool {
        fn name(&self) -> &str {
            "hmac_webhook"
        }

        fn description(&self) -> &str {
            "hmac test"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({"emit_events":[]}),
                Duration::from_millis(1),
            ))
        }

        fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
            Some(crate::tools::wasm::WebhookCapability {
                hmac_secret_name: Some("hmac_secret".to_string()),
                hmac_signature_header: Some("x-hub-signature-256".to_string()),
                hmac_prefix: Some("sha256=".to_string()),
                ..Default::default()
            })
        }
    }

    #[async_trait]
    impl Tool for MisconfiguredWebhookTool {
        fn name(&self) -> &str {
            "misconfigured_webhook"
        }

        fn description(&self) -> &str {
            "misconfigured test"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({"emit_events":[]}),
                Duration::from_millis(1),
            ))
        }

        fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
            Some(crate::tools::wasm::WebhookCapability::default())
        }
    }

    #[tokio::test]
    async fn returns_not_found_for_unknown_tool() {
        let tools = Arc::new(ToolRegistry::new());
        let app = routes(ToolWebhookState {
            tools,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            user_id: "test".to_string(),
            secrets_store: None,
        });

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/tools/missing")
            .body(Body::from("{}"))
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_tool_without_webhook_capability() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(TestWebhookTool)).await;
        let app = routes(ToolWebhookState {
            tools,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            user_id: "test".to_string(),
            secrets_store: None,
        });

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/tools/test_webhook")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"ok":true}"#))
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_when_required_secret_missing() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(ProtectedWebhookTool)).await;

        let secrets = Arc::new(InMemorySecretsStore::new(Arc::new(
            SecretsCrypto::new(secrecy::SecretString::from(
                "test-key-at-least-32-chars-long!!".to_string(),
            ))
            .expect("crypto"),
        )));
        secrets
            .create(
                "test",
                CreateSecretParams::new("test_webhook_secret", "s3cret"),
            )
            .await
            .expect("secret create");

        let app = routes(ToolWebhookState {
            tools,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            user_id: "test".to_string(),
            secrets_store: Some(secrets),
        });

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/tools/protected_webhook")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"ok":true}"#))
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn accepts_with_valid_hmac_signature() {
        use hmac::Mac;

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(HmacWebhookTool)).await;

        let secrets = Arc::new(InMemorySecretsStore::new(Arc::new(
            SecretsCrypto::new(secrecy::SecretString::from(
                "test-key-at-least-32-chars-long!!".to_string(),
            ))
            .expect("crypto"),
        )));
        secrets
            .create(
                "test",
                CreateSecretParams::new("hmac_secret", "github-secret"),
            )
            .await
            .expect("secret create");

        let app = routes(ToolWebhookState {
            tools,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            user_id: "test".to_string(),
            secrets_store: Some(secrets),
        });

        let payload = br#"{"action":"opened"}"#;
        let mut mac =
            hmac::Hmac::<sha2::Sha256>::new_from_slice(b"github-secret").expect("hmac key");
        mac.update(payload);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/tools/hmac_webhook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", sig)
            .body(Body::from(payload.to_vec()))
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn rejects_empty_webhook_capability_as_misconfigured() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(MisconfiguredWebhookTool)).await;

        let secrets = Arc::new(InMemorySecretsStore::new(Arc::new(
            SecretsCrypto::new(secrecy::SecretString::from(
                "test-key-at-least-32-chars-long!!".to_string(),
            ))
            .expect("crypto"),
        )));

        let app = routes(ToolWebhookState {
            tools,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            user_id: "test".to_string(),
            secrets_store: Some(secrets),
        });

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/tools/misconfigured_webhook")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"ok":true}"#))
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn health_check_returns_ok_for_webhook_capable_tool() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(ProtectedWebhookTool)).await;
        let app = routes(ToolWebhookState {
            tools,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            user_id: "test".to_string(),
            secrets_store: None,
        });

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/webhook/tools/protected_webhook")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn health_check_returns_not_found_for_non_webhook_tool() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(TestWebhookTool)).await;
        let app = routes(ToolWebhookState {
            tools,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            user_id: "test".to_string(),
            secrets_store: None,
        });

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/webhook/tools/test_webhook")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
