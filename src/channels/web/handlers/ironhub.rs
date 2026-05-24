use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::{
    IronhubInfoQuery, IronhubInstallRequest, IronhubListQuery, IronhubRegisterRequest,
    IronhubSearchQuery, IronhubSigningKeyMetadata, IronhubSigningKeySetRequest,
    IronhubVerifyIntentRequest, IronhubVerifyIntentResponse,
};
use crate::secrets::{CreateSecretParams, SecretError, SecretsStore};
use crate::tools::ToolError;
use crate::tools::dispatch::DispatchSource;

const IRONHUB_SIGNING_KEY_NAME: &str = "ironhub_signing_key";
const VERIFY_INTENT_WINDOW_SECS: u64 = 300;
const SHARED_KEY_PREFIX: &str = "ihub_sk_";
const SHARED_KEY_MIN_LEN: usize = 32;
const SHARED_KEY_MIN_DISTINCT: usize = 12;
const NONCE_CACHE_MAX_ENTRIES: usize = 16_384;

struct NonceCache {
    seen: HashMap<String, Instant>,
    order: VecDeque<String>,
}

impl NonceCache {
    fn new() -> Self {
        Self {
            seen: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn front_is_expired(&self, now: Instant, ttl: Duration) -> bool {
        match self.order.front() {
            Some(front) => match self.seen.get(front) {
                Some(seen_at) => now.duration_since(*seen_at) > ttl,
                None => true,
            },
            None => false,
        }
    }

    fn record_or_seen(&mut self, key: String, now: Instant, ttl: Duration) -> bool {
        while self.front_is_expired(now, ttl) {
            if let Some(evicted) = self.order.pop_front() {
                self.seen.remove(&evicted);
            }
        }
        if self.seen.contains_key(&key) {
            return true;
        }
        if self.seen.len() >= NONCE_CACHE_MAX_ENTRIES
            && let Some(evicted) = self.order.pop_front()
        {
            self.seen.remove(&evicted);
        }
        self.seen.insert(key.clone(), now);
        self.order.push_back(key);
        false
    }
}

static NONCE_CACHE: LazyLock<Mutex<NonceCache>> = LazyLock::new(|| Mutex::new(NonceCache::new()));

fn nonce_seen_or_record(uid: &str, nonce: &str) -> bool {
    let key = format!("{uid}:{nonce}");
    let ttl = Duration::from_secs(VERIFY_INTENT_WINDOW_SECS);
    let now = Instant::now();
    let mut guard = match NONCE_CACHE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.record_or_seen(key, now, ttl)
}

fn validate_shared_key(value: &str) -> Result<(), String> {
    if !value.starts_with(SHARED_KEY_PREFIX) {
        return Err(format!(
            "shared key must start with {SHARED_KEY_PREFIX} prefix"
        ));
    }
    if value.len() < SHARED_KEY_MIN_LEN {
        return Err(format!(
            "shared key must be at least {SHARED_KEY_MIN_LEN} characters"
        ));
    }
    let body = &value[SHARED_KEY_PREFIX.len()..];
    let distinct = body.chars().collect::<HashSet<_>>().len();
    if distinct < SHARED_KEY_MIN_DISTINCT {
        return Err(format!(
            "shared key is too low-entropy; the part after {SHARED_KEY_PREFIX} must contain at least {SHARED_KEY_MIN_DISTINCT} distinct characters"
        ));
    }
    Ok(())
}

fn tool_error_to_http(err: ToolError) -> (StatusCode, String) {
    match err {
        ToolError::InvalidParameters(msg) => (StatusCode::BAD_REQUEST, msg),
        ToolError::NotAuthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
        ToolError::RateLimited(retry) => (
            StatusCode::TOO_MANY_REQUESTS,
            retry
                .map(|d: Duration| format!("rate limited; retry after {}s", d.as_secs()))
                .unwrap_or_else(|| "rate limited".to_string()),
        ),
        ToolError::Timeout(d) => (
            StatusCode::GATEWAY_TIMEOUT,
            format!("execution timed out after {}s", d.as_secs()),
        ),
        ToolError::ExternalService(msg) => (StatusCode::BAD_GATEWAY, msg),
        ToolError::ExecutionFailed(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        ToolError::Sandbox(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
    }
}

fn dispatcher_or_503(
    state: &Arc<GatewayState>,
) -> Result<Arc<crate::tools::dispatch::ToolDispatcher>, (StatusCode, String)> {
    state.tool_dispatcher.as_ref().map(Arc::clone).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "tool dispatcher not available".to_string(),
    ))
}

fn catalog_rate_limit(
    state: &Arc<GatewayState>,
    user_id: &str,
) -> Result<(), (StatusCode, String)> {
    if state.ironhub_catalog_rate_limiter.check(user_id) {
        Ok(())
    } else {
        Err((
            StatusCode::TOO_MANY_REQUESTS,
            "IronHub catalog rate limit exceeded; try again later".to_string(),
        ))
    }
}

pub async fn ironhub_install_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(user): AdminUser,
    Json(req): Json<IronhubInstallRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    catalog_rate_limit(&state, &user.user_id)?;

    let store = secrets_store_or_503(&state)?;
    let signed = SignedInstall {
        slug: &req.slug,
        version: &req.version,
        uid: &req.uid,
        aid: &req.aid,
        ts: req.ts,
        nonce: &req.nonce,
        sig: &req.sig,
        artifact_digest: &req.artifact_digest,
    };
    match verify_signed_install(store.as_ref(), &user.user_id, &signed).await {
        Ok(()) => {}
        Err(SignedInstallError::Rejected(reason)) => return Err((StatusCode::FORBIDDEN, reason)),
        Err(SignedInstallError::NoSigningKey) => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "no signing key configured on this agent".to_string(),
            ));
        }
        Err(SignedInstallError::Internal(e)) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }

    // verify-intent is a repeatable preview, so the one-shot nonce is only burned
    // here, at the install that actually mutates state.
    if nonce_seen_or_record(&user.user_id, &req.nonce) {
        return Err((StatusCode::CONFLICT, "nonce already used".to_string()));
    }

    let dispatcher = dispatcher_or_503(&state)?;
    let mut params = serde_json::Map::new();
    params.insert("name".into(), serde_json::Value::String(req.slug));
    params.insert("version".into(), serde_json::Value::String(req.version));
    params.insert(
        "artifact_digest".into(),
        serde_json::Value::String(req.artifact_digest),
    );
    params.insert(
        "acknowledge_unverified".into(),
        serde_json::Value::Bool(req.acknowledge_unverified),
    );

    let output = dispatcher
        .dispatch(
            "ironhub_install",
            serde_json::Value::Object(params),
            &user.user_id,
            DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(tool_error_to_http)?;
    Ok(Json(output.result))
}

pub async fn ironhub_search_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(q): Query<IronhubSearchQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    catalog_rate_limit(&state, &user.user_id)?;
    let dispatcher = dispatcher_or_503(&state)?;
    let mut params = serde_json::Map::new();
    params.insert("query".into(), serde_json::Value::String(q.query));
    if let Some(tag) = q.release_tag {
        params.insert("release_tag".into(), serde_json::Value::String(tag));
    }
    let output = dispatcher
        .dispatch(
            "ironhub_search",
            serde_json::Value::Object(params),
            &user.user_id,
            DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(tool_error_to_http)?;
    Ok(Json(output.result))
}

pub async fn ironhub_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(q): Query<IronhubListQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    catalog_rate_limit(&state, &user.user_id)?;
    let dispatcher = dispatcher_or_503(&state)?;
    let mut params = serde_json::Map::new();
    if let Some(tag) = q.release_tag {
        params.insert("release_tag".into(), serde_json::Value::String(tag));
    }
    let output = dispatcher
        .dispatch(
            "ironhub_list",
            serde_json::Value::Object(params),
            &user.user_id,
            DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(tool_error_to_http)?;
    Ok(Json(output.result))
}

pub async fn ironhub_info_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(q): Query<IronhubInfoQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    catalog_rate_limit(&state, &user.user_id)?;
    let dispatcher = dispatcher_or_503(&state)?;
    let mut params = serde_json::Map::new();
    params.insert("name".into(), serde_json::Value::String(q.name));
    if let Some(tag) = q.release_tag {
        params.insert("release_tag".into(), serde_json::Value::String(tag));
    }
    let output = dispatcher
        .dispatch(
            "ironhub_info",
            serde_json::Value::Object(params),
            &user.user_id,
            DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(tool_error_to_http)?;
    Ok(Json(output.result))
}

fn secrets_store_or_503(
    state: &Arc<GatewayState>,
) -> Result<Arc<dyn SecretsStore + Send + Sync>, (StatusCode, String)> {
    state.secrets_store.as_ref().map(Arc::clone).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "secrets store not available".to_string(),
    ))
}

fn fingerprint(key_hex: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(key_hex.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..6])
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hmac_hex(shared_key: &str, msg: &str) -> Result<String, String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(shared_key.as_bytes())
        .map_err(|e| format!("hmac initialization failed: {e}"))?;
    mac.update(msg.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn install_payload(
    slug: &str,
    version: &str,
    uid: &str,
    aid: &str,
    ts: u64,
    nonce: &str,
    artifact_digest: &str,
) -> String {
    format!("install:{slug}:{version}:{uid}:{aid}:{ts}:{nonce}:{artifact_digest}")
}

fn register_payload(uid: &str, aid: &str, ts: u64, nonce: &str) -> String {
    format!("register:{uid}:{aid}:{ts}:{nonce}")
}

struct SignedInstall<'a> {
    slug: &'a str,
    version: &'a str,
    uid: &'a str,
    aid: &'a str,
    ts: u64,
    nonce: &'a str,
    sig: &'a str,
    artifact_digest: &'a str,
}

enum SignedInstallError {
    Rejected(String),
    NoSigningKey,
    Internal(String),
}

async fn verify_signed_install(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    signed: &SignedInstall<'_>,
) -> Result<(), SignedInstallError> {
    if let Err(e) = crate::cli::hub_install::validate_hub_name(signed.slug) {
        return Err(SignedInstallError::Rejected(format!("invalid slug: {e}")));
    }

    let drift = now_unix().abs_diff(signed.ts);
    if drift > VERIFY_INTENT_WINDOW_SECS {
        return Err(SignedInstallError::Rejected(format!(
            "timestamp drift {drift}s exceeds window {VERIFY_INTENT_WINDOW_SECS}s"
        )));
    }

    let decrypted = match store.get_decrypted(user_id, IRONHUB_SIGNING_KEY_NAME).await {
        Ok(s) => s,
        Err(SecretError::NotFound(_)) => return Err(SignedInstallError::NoSigningKey),
        Err(e) => return Err(SignedInstallError::Internal(e.to_string())),
    };

    let payload = install_payload(
        signed.slug,
        signed.version,
        signed.uid,
        signed.aid,
        signed.ts,
        signed.nonce,
        signed.artifact_digest,
    );
    let expected = hmac_hex(decrypted.expose(), &payload).map_err(SignedInstallError::Internal)?;

    use subtle::ConstantTimeEq;
    let sig_valid: bool = expected.as_bytes().ct_eq(signed.sig.as_bytes()).into();
    if sig_valid {
        Ok(())
    } else {
        Err(SignedInstallError::Rejected(
            "signature mismatch".to_string(),
        ))
    }
}

pub async fn ironhub_signing_key_set_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(user): AdminUser,
    Json(req): Json<IronhubSigningKeySetRequest>,
) -> Result<Json<IronhubSigningKeyMetadata>, (StatusCode, String)> {
    let shared_key = req.shared_key.trim();
    if let Err(e) = validate_shared_key(shared_key) {
        return Err((StatusCode::BAD_REQUEST, e));
    }

    let store = secrets_store_or_503(&state)?;
    let secret = store
        .create(
            &user.user_id,
            CreateSecretParams::new(IRONHUB_SIGNING_KEY_NAME, shared_key),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let stored = store
        .get_decrypted(&user.user_id, IRONHUB_SIGNING_KEY_NAME)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(IronhubSigningKeyMetadata {
        fingerprint: fingerprint(stored.expose()),
        created_at: secret.created_at.to_rfc3339(),
    }))
}

pub async fn ironhub_signing_key_get_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(user): AdminUser,
) -> Result<Json<IronhubSigningKeyMetadata>, (StatusCode, String)> {
    let store = secrets_store_or_503(&state)?;
    let meta = match store.get(&user.user_id, IRONHUB_SIGNING_KEY_NAME).await {
        Ok(s) => s,
        Err(SecretError::NotFound(_)) => {
            return Err((StatusCode::NOT_FOUND, "no signing key set".to_string()));
        }
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };
    let decrypted = store
        .get_decrypted(&user.user_id, IRONHUB_SIGNING_KEY_NAME)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(IronhubSigningKeyMetadata {
        fingerprint: fingerprint(decrypted.expose()),
        created_at: meta.created_at.to_rfc3339(),
    }))
}

pub async fn ironhub_signing_key_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(user): AdminUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = secrets_store_or_503(&state)?;
    let removed = store
        .delete(&user.user_id, IRONHUB_SIGNING_KEY_NAME)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no signing key set".to_string()))
    }
}

pub async fn ironhub_verify_intent_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<IronhubVerifyIntentRequest>,
) -> Result<Json<IronhubVerifyIntentResponse>, (StatusCode, String)> {
    catalog_rate_limit(&state, &user.user_id)?;
    let store = secrets_store_or_503(&state)?;
    let signed = SignedInstall {
        slug: &req.slug,
        version: &req.version,
        uid: &req.uid,
        aid: &req.aid,
        ts: req.ts,
        nonce: &req.nonce,
        sig: &req.sig,
        artifact_digest: &req.artifact_digest,
    };
    match verify_signed_install(store.as_ref(), &user.user_id, &signed).await {
        Ok(()) => Ok(Json(IronhubVerifyIntentResponse {
            valid: true,
            reason: None,
        })),
        Err(SignedInstallError::Rejected(reason)) => Ok(Json(IronhubVerifyIntentResponse {
            valid: false,
            reason: Some(reason),
        })),
        Err(SignedInstallError::NoSigningKey) => Ok(Json(IronhubVerifyIntentResponse {
            valid: false,
            reason: Some("no signing key configured on this agent".to_string()),
        })),
        Err(SignedInstallError::Internal(e)) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

pub async fn ironhub_register_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<IronhubRegisterRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    catalog_rate_limit(&state, &user.user_id)?;
    let now = now_unix();
    let drift = now.abs_diff(req.ts);
    if drift > VERIFY_INTENT_WINDOW_SECS {
        return Err((
            StatusCode::REQUEST_TIMEOUT,
            format!("timestamp drift {drift}s exceeds window {VERIFY_INTENT_WINDOW_SECS}s"),
        ));
    }

    let store = secrets_store_or_503(&state)?;
    let decrypted = match store
        .get_decrypted(&user.user_id, IRONHUB_SIGNING_KEY_NAME)
        .await
    {
        Ok(s) => s,
        Err(SecretError::NotFound(_)) => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "no signing key configured on this agent".to_string(),
            ));
        }
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let payload = register_payload(&req.uid, &req.aid, req.ts, &req.nonce);
    let expected = hmac_hex(decrypted.expose(), &payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    use subtle::ConstantTimeEq;
    let supplied = req.sig.as_bytes();
    let sig_valid: bool = expected.as_bytes().ct_eq(supplied).into();
    if !sig_valid {
        return Err((StatusCode::UNAUTHORIZED, "signature mismatch".to_string()));
    }

    if nonce_seen_or_record(&user.user_id, &req.nonce) {
        return Err((StatusCode::CONFLICT, "nonce already used".to_string()));
    }

    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::web::platform::auth::UserIdentity;
    use crate::config::SafetyConfig;
    use crate::db::Database;
    use crate::db::UserRecord;
    use crate::db::libsql::LibSqlBackend;
    use crate::tools::dispatch::ToolDispatcher;
    use crate::tools::{ApprovalRequirement, Tool, ToolOutput, ToolRegistry};
    use async_trait::async_trait;
    use axum::Router;
    use axum::body::Body;
    use axum::routing::{get, post};
    use ironclaw_safety::SafetyLayer;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    struct StubIronhubTool {
        name: &'static str,
        schema: serde_json::Value,
        approval: ApprovalRequirement,
        calls: Arc<AtomicUsize>,
        response: serde_json::Value,
    }

    #[async_trait]
    impl Tool for StubIronhubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            self.schema.clone()
        }
        fn requires_approval(&self, _: &serde_json::Value) -> ApprovalRequirement {
            self.approval
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<ToolOutput, ToolError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::success(
                self.response.clone(),
                std::time::Duration::from_millis(1),
            ))
        }
    }

    fn install_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "pattern": "^[a-z0-9][a-z0-9_-]*$", "minLength": 1, "maxLength": 64 },
                "kind": { "type": "string", "enum": ["tool", "skill"] },
                "release_tag": { "type": "string", "pattern": "^[A-Za-z0-9._-]+$", "minLength": 1, "maxLength": 128 },
                "version": { "type": "string", "minLength": 1, "maxLength": 128 },
                "artifact_digest": { "type": "string", "minLength": 1, "maxLength": 128 },
                "force": { "type": "boolean", "default": false },
                "acknowledge_unverified": { "type": "boolean", "default": false }
            },
            "required": ["name"]
        })
    }

    fn search_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": { "type": "string", "minLength": 1, "maxLength": 128 },
                "release_tag": { "type": "string", "pattern": "^[A-Za-z0-9._-]+$", "minLength": 1, "maxLength": 128 }
            },
            "required": ["query"]
        })
    }

    fn list_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "release_tag": { "type": "string", "pattern": "^[A-Za-z0-9._-]+$", "minLength": 1, "maxLength": 128 }
            }
        })
    }

    fn info_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "pattern": "^[a-z0-9][a-z0-9_-]*$", "minLength": 1, "maxLength": 64 },
                "release_tag": { "type": "string", "pattern": "^[A-Za-z0-9._-]+$", "minLength": 1, "maxLength": 128 }
            },
            "required": ["name"]
        })
    }

    async fn build_state_with_stubs() -> (Arc<GatewayState>, Arc<AtomicUsize>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(
            LibSqlBackend::new_local(&dir.path().join("test.db"))
                .await
                .expect("libsql backend"),
        );
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::clone(&backend) as Arc<dyn Database>;
        let now = chrono::Utc::now();
        for id in ["test-admin", "test-user"] {
            db.create_user(&UserRecord {
                id: id.into(),
                email: None,
                display_name: id.into(),
                status: "active".into(),
                role: if id == "test-admin" {
                    "admin".into()
                } else {
                    "regular".into()
                },
                created_at: now,
                updated_at: now,
                last_login_at: None,
                created_by: None,
                metadata: serde_json::json!({}),
            })
            .await
            .expect("create user");
        }
        let registry = Arc::new(ToolRegistry::new());
        let calls = Arc::new(AtomicUsize::new(0));
        registry
            .register(Arc::new(StubIronhubTool {
                name: "ironhub_install",
                schema: install_schema(),
                approval: ApprovalRequirement::Never,
                calls: Arc::clone(&calls),
                response: serde_json::json!({"status": "installed", "name": "clickup"}),
            }))
            .await;
        registry
            .register(Arc::new(StubIronhubTool {
                name: "ironhub_search",
                schema: search_schema(),
                approval: ApprovalRequirement::Never,
                calls: Arc::clone(&calls),
                response: serde_json::json!({"results": []}),
            }))
            .await;
        registry
            .register(Arc::new(StubIronhubTool {
                name: "ironhub_list",
                schema: list_schema(),
                approval: ApprovalRequirement::Never,
                calls: Arc::clone(&calls),
                response: serde_json::json!({"tools": [], "skills": []}),
            }))
            .await;
        registry
            .register(Arc::new(StubIronhubTool {
                name: "ironhub_info",
                schema: info_schema(),
                approval: ApprovalRequirement::Never,
                calls: Arc::clone(&calls),
                response: serde_json::json!({"kind": "tool", "name": "clickup"}),
            }))
            .await;

        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 65_536,
            injection_check_enabled: false,
        }));
        let dispatcher = Arc::new(ToolDispatcher::new(registry, safety, db));
        std::mem::forget(dir);

        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        secrets
            .create(
                "test-admin",
                CreateSecretParams::new(IRONHUB_SIGNING_KEY_NAME, TEST_SHARED_KEY),
            )
            .await
            .expect("seed signing key");

        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .with_tool_dispatcher(dispatcher)
            .with_secrets_store(secrets)
            .build();
        (state, calls)
    }

    fn req_with_identity(
        method: &str,
        uri: &str,
        body: Body,
        role: &str,
    ) -> axum::http::Request<Body> {
        let mut req = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body)
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: if role == "admin" {
                "test-admin".into()
            } else {
                "test-user".into()
            },
            role: role.into(),
            workspace_read_scopes: Vec::new(),
        });
        req
    }

    fn req_no_identity(method: &str, uri: &str, body: Body) -> axum::http::Request<Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body)
            .expect("request")
    }

    fn install_signed_body(slug: &str, nonce: &str, ts: u64) -> serde_json::Value {
        let payload = install_payload(slug, "1.0.0", "u1", "a1", ts, nonce, TEST_ARTIFACT_DIGEST);
        let sig = hmac_hex(TEST_SHARED_KEY, &payload).expect("sign");
        serde_json::json!({
            "slug": slug,
            "version": "1.0.0",
            "uid": "u1",
            "aid": "a1",
            "ts": ts,
            "nonce": nonce,
            "sig": sig,
            "artifact_digest": TEST_ARTIFACT_DIGEST,
        })
    }

    #[test]
    fn nonce_cache_records_then_detects_replay() {
        let mut cache = NonceCache::new();
        let now = Instant::now();
        let ttl = Duration::from_secs(VERIFY_INTENT_WINDOW_SECS);
        assert!(!cache.record_or_seen("u:n1".into(), now, ttl));
        assert!(cache.record_or_seen("u:n1".into(), now, ttl));
        assert!(!cache.record_or_seen("u:n2".into(), now, ttl));
    }

    #[test]
    fn nonce_cache_evicts_expired_before_recording() {
        let mut cache = NonceCache::new();
        let ttl = Duration::from_secs(VERIFY_INTENT_WINDOW_SECS);
        let base = Instant::now();
        let later = base + ttl + Duration::from_secs(10);
        assert!(!cache.record_or_seen("u:old".into(), base, ttl));
        assert!(!cache.record_or_seen("u:fresh".into(), later, ttl));
        assert!(
            !cache.record_or_seen("u:old".into(), later, ttl),
            "an expired nonce must be evicted, so re-recording it is new, not a replay"
        );
    }

    #[test]
    fn nonce_cache_stays_bounded_and_evicts_oldest() {
        let mut cache = NonceCache::new();
        let now = Instant::now();
        let ttl = Duration::from_secs(VERIFY_INTENT_WINDOW_SECS);
        for i in 0..NONCE_CACHE_MAX_ENTRIES {
            assert!(!cache.record_or_seen(format!("u:{i}"), now, ttl));
        }
        assert_eq!(cache.seen.len(), NONCE_CACHE_MAX_ENTRIES);
        assert!(!cache.record_or_seen("u:overflow".into(), now, ttl));
        assert_eq!(cache.seen.len(), NONCE_CACHE_MAX_ENTRIES);
        assert!(!cache.seen.contains_key("u:0"));
    }

    #[tokio::test]
    async fn ironhub_install_rejects_unauthenticated() {
        let (state, _calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let req = req_no_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(serde_json::json!({"name": "clickup"}).to_string()),
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ironhub_install_rejects_non_admin() {
        let (state, _calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let req = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(serde_json::json!({"name": "clickup"}).to_string()),
            "regular",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn ironhub_install_admin_dispatches_tool() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let nonce = uuid::Uuid::new_v4().to_string();
        let body = install_signed_body("clickup", &nonce, now_unix());
        let req = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ironhub_install_handler_forwards_acknowledge_unverified() {
        use std::sync::Mutex as StdMutex;
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(
            LibSqlBackend::new_local(&dir.path().join("test.db"))
                .await
                .expect("libsql backend"),
        );
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::clone(&backend) as Arc<dyn Database>;
        let now = chrono::Utc::now();
        db.create_user(&UserRecord {
            id: "test-admin".into(),
            email: None,
            display_name: "test-admin".into(),
            status: "active".into(),
            role: "admin".into(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        })
        .await
        .expect("create user");

        struct ParamCaptureStub {
            captured: Arc<StdMutex<Option<serde_json::Value>>>,
        }
        #[async_trait]
        impl Tool for ParamCaptureStub {
            fn name(&self) -> &str {
                "ironhub_install"
            }
            fn description(&self) -> &str {
                "stub"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                install_schema()
            }
            async fn execute(
                &self,
                params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<ToolOutput, ToolError> {
                *self.captured.lock().unwrap() = Some(params);
                Ok(ToolOutput::success(
                    serde_json::json!({"status": "installed"}),
                    std::time::Duration::from_millis(1),
                ))
            }
        }

        let captured = Arc::new(StdMutex::new(None));
        let registry = Arc::new(ToolRegistry::new());
        registry
            .register(Arc::new(ParamCaptureStub {
                captured: Arc::clone(&captured),
            }))
            .await;
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 65_536,
            injection_check_enabled: false,
        }));
        let dispatcher = Arc::new(ToolDispatcher::new(registry, safety, db));
        std::mem::forget(dir);

        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        secrets
            .create(
                "test-admin",
                CreateSecretParams::new(IRONHUB_SIGNING_KEY_NAME, TEST_SHARED_KEY),
            )
            .await
            .expect("seed signing key");

        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .with_tool_dispatcher(dispatcher)
            .with_secrets_store(secrets)
            .build();
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let nonce = uuid::Uuid::new_v4().to_string();
        let mut body = install_signed_body("clickup", &nonce, now_unix());
        body["acknowledge_unverified"] = serde_json::json!(true);
        let req = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let params = captured.lock().unwrap().clone().expect("params captured");
        assert_eq!(
            params
                .get("acknowledge_unverified")
                .and_then(|v| v.as_bool()),
            Some(true),
            "gateway handler must forward acknowledge_unverified to the ironhub_install tool"
        );
        assert_eq!(
            params.get("name").and_then(|v| v.as_str()),
            Some("clickup"),
            "gateway handler must forward the signed slug as the install name"
        );
        assert_eq!(
            params.get("version").and_then(|v| v.as_str()),
            Some("1.0.0"),
            "gateway handler must forward the signed version so the tool binds the install to it"
        );
        assert_eq!(
            params.get("artifact_digest").and_then(|v| v.as_str()),
            Some(TEST_ARTIFACT_DIGEST),
            "gateway handler must forward the signed artifact_digest so the tool binds the install to the artifact content"
        );
    }

    #[tokio::test]
    async fn ironhub_catalog_handlers_rate_limit_per_user() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(
            LibSqlBackend::new_local(&dir.path().join("test.db"))
                .await
                .expect("libsql backend"),
        );
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::clone(&backend) as Arc<dyn Database>;
        let now = chrono::Utc::now();
        db.create_user(&UserRecord {
            id: "test-user".into(),
            email: None,
            display_name: "test-user".into(),
            status: "active".into(),
            role: "regular".into(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        })
        .await
        .expect("create user");
        let registry = Arc::new(ToolRegistry::new());
        registry
            .register(Arc::new(StubIronhubTool {
                name: "ironhub_search",
                schema: search_schema(),
                approval: ApprovalRequirement::Never,
                calls: Arc::new(AtomicUsize::new(0)),
                response: serde_json::json!({"results": []}),
            }))
            .await;
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 65_536,
            injection_check_enabled: false,
        }));
        let dispatcher = Arc::new(ToolDispatcher::new(registry, safety, db));
        std::mem::forget(dir);

        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .with_tool_dispatcher(dispatcher)
            .with_ironhub_catalog_rate_limit(1, 60)
            .build();
        let app = Router::new()
            .route("/api/ironhub/search", get(ironhub_search_handler))
            .with_state(state);

        let first = req_with_identity(
            "GET",
            "/api/ironhub/search?query=rpc",
            Body::empty(),
            "regular",
        );
        let resp1 = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), first)
            .await
            .expect("first");
        assert_eq!(resp1.status(), StatusCode::OK);

        let second = req_with_identity(
            "GET",
            "/api/ironhub/search?query=rpc",
            Body::empty(),
            "regular",
        );
        let resp2 = ServiceExt::<axum::http::Request<Body>>::oneshot(app, second)
            .await
            .expect("second");
        assert_eq!(
            resp2.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "catalog dispatch must rate-limit per user"
        );
    }

    #[tokio::test]
    async fn ironhub_install_accepts_underscore_in_name() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let nonce = uuid::Uuid::new_v4().to_string();
        let body = install_signed_body("microsoft_365", &nonce, now_unix());
        let req = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "underscore names like microsoft_365 must pass the schema regex"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ironhub_install_rejects_path_traversal_in_name() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let nonce = uuid::Uuid::new_v4().to_string();
        let body = install_signed_body("../etc/passwd", &nonce, now_unix());
        let req = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "tool execute must NOT run when the signed slug is rejected"
        );
    }

    #[tokio::test]
    async fn ironhub_search_rejects_unauthenticated() {
        let (state, _calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/search", get(ironhub_search_handler))
            .with_state(state);
        let req = req_no_identity("GET", "/api/ironhub/search?query=rpc", Body::empty());
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ironhub_search_returns_dispatch_result_for_authenticated_user() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/search", get(ironhub_search_handler))
            .with_state(state);
        let req = req_with_identity(
            "GET",
            "/api/ironhub/search?query=rpc",
            Body::empty(),
            "regular",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ironhub_list_passes_release_tag_query_to_dispatch() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/list", get(ironhub_list_handler))
            .with_state(state);
        let req = req_with_identity(
            "GET",
            "/api/ironhub/list?release_tag=release-test",
            Body::empty(),
            "regular",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ironhub_info_rejects_path_traversal_in_query_name() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/info", get(ironhub_info_handler))
            .with_state(state);
        let req = req_with_identity(
            "GET",
            "/api/ironhub/info?name=..%2Fetc%2Fpasswd",
            Body::empty(),
            "regular",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "tool execute must NOT run when schema rejects"
        );
    }

    #[tokio::test]
    async fn ironhub_install_rejects_unknown_field() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let nonce = uuid::Uuid::new_v4().to_string();
        let mut body = install_signed_body("clickup", &nonce, now_unix());
        body["evil"] = serde_json::json!("exfil");
        let req = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert!(
            resp.status() == StatusCode::BAD_REQUEST
                || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
            "expected 400 or 422 for unknown field, got {:?}",
            resp.status()
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "tool execute must NOT run when extra field rejected"
        );
    }

    #[tokio::test]
    async fn ironhub_install_rejects_bad_signature() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let nonce = uuid::Uuid::new_v4().to_string();
        let body = serde_json::json!({
            "slug": "clickup",
            "version": "1.0.0",
            "uid": "u1",
            "aid": "a1",
            "ts": now_unix(),
            "nonce": nonce,
            "sig": "deadbeef".repeat(8),
            "artifact_digest": "d0",
        });
        let req = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "install must reject an unsigned/forged request before dispatching"
        );
    }

    #[tokio::test]
    async fn ironhub_install_rejects_replayed_nonce() {
        let (state, calls) = build_state_with_stubs().await;
        let app = Router::new()
            .route("/api/ironhub/install", post(ironhub_install_handler))
            .with_state(state);
        let nonce = uuid::Uuid::new_v4().to_string();
        let body = install_signed_body("clickup", &nonce, now_unix());
        let first = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let r1 = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), first)
            .await
            .expect("first");
        assert_eq!(r1.status(), StatusCode::OK);

        let replay = req_with_identity(
            "POST",
            "/api/ironhub/install",
            Body::from(body.to_string()),
            "admin",
        );
        let r2 = ServiceExt::<axum::http::Request<Body>>::oneshot(app, replay)
            .await
            .expect("replay");
        assert_eq!(r2.status(), StatusCode::CONFLICT);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a replayed signed install must not dispatch the tool twice"
        );
    }

    #[test]
    fn install_hmac_is_deterministic_and_payload_sensitive() {
        let key_a = "ihub_sk_aaaaaaaaaaaaaaaaaaaaaaaa";
        let key_b = "ihub_sk_bbbbbbbbbbbbbbbbbbbbbbbb";
        let p1 = install_payload("clickup", "1.0.0", "u1", "a1", 1_700_000_000, "n1", "d0");
        let p_other_slug =
            install_payload("evm-rpc", "1.0.0", "u1", "a1", 1_700_000_000, "n1", "d0");
        let p_other_nonce =
            install_payload("clickup", "1.0.0", "u1", "a1", 1_700_000_000, "n2", "d0");
        let s1 = hmac_hex(key_a, &p1).expect("hmac");
        let s2 = hmac_hex(key_a, &p1).expect("hmac");
        let s3 = hmac_hex(key_b, &p1).expect("hmac");
        let s4 = hmac_hex(key_a, &p_other_slug).expect("hmac");
        let s5 = hmac_hex(key_a, &p_other_nonce).expect("hmac");
        assert_eq!(s1, s2, "same inputs must produce same signature");
        assert_ne!(s1, s3, "different key must produce different signature");
        assert_ne!(s1, s4, "different slug must produce different signature");
        assert_ne!(s1, s5, "different nonce must produce different signature");
        assert_eq!(s1.len(), 64, "hex sha256 hmac is 64 chars");
    }

    #[test]
    fn install_payload_format_is_stable() {
        let p = install_payload(
            "clickup",
            "1.0.0",
            "u1",
            "a1",
            1_700_000_000,
            "n1",
            "deadbeef",
        );
        assert_eq!(p, "install:clickup:1.0.0:u1:a1:1700000000:n1:deadbeef");
    }

    #[test]
    fn register_payload_format_is_stable() {
        let p = register_payload("u1", "a1", 1_700_000_000, "n1");
        assert_eq!(p, "register:u1:a1:1700000000:n1");
    }

    #[test]
    fn validate_shared_key_enforces_prefix_length_and_entropy() {
        assert!(validate_shared_key(TEST_SHARED_KEY).is_ok());
        assert!(validate_shared_key("ihub_sk_short").is_err());
        assert!(validate_shared_key("not_prefixed_keykey_keykey_keykey").is_err());
        assert!(
            validate_shared_key("ihub_sk_aaaaaaaaaaaaaaaaaaaaaaaa").is_err(),
            "a 32-char key whose body is one repeated character must be rejected as low-entropy"
        );
    }

    #[test]
    fn fingerprint_is_stable_and_short() {
        let key = "ihub_sk_aaaaaaaaaaaaaaaaaaaaaaaa";
        let f1 = fingerprint(key);
        let f2 = fingerprint(key);
        assert_eq!(f1, f2);
        assert_eq!(f1.len(), 12, "6 bytes -> 12 hex chars");
        assert_ne!(f1, fingerprint("ihub_sk_bbbbbbbbbbbbbbbbbbbbbbbb"));
    }

    const TEST_SHARED_KEY: &str = "ihub_sk_x7K2p9mQ4vR8tL3nB6wZ1yD5cF0jH";
    const TEST_ARTIFACT_DIGEST: &str =
        "4e205e4f8061512d5bca40ebe50acbb93d44afa3e083981a7b434f9ee3bab6a3";

    #[tokio::test]
    async fn verify_intent_is_rate_limited_per_user() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .user_id("test-user")
            .with_secrets_store(secrets)
            .with_ironhub_catalog_rate_limit(1, 60)
            .build();
        let app = Router::new()
            .route(
                "/api/ironhub/verify-intent",
                post(ironhub_verify_intent_handler),
            )
            .with_state(state);
        let body = serde_json::json!({
            "slug": "clickup",
            "version": "1.0.0",
            "uid": "u1",
            "aid": "a1",
            "ts": now_unix(),
            "nonce": "rl-verify-1",
            "sig": "deadbeef",
            "artifact_digest": "d0"
        })
        .to_string();
        let first = req_with_identity(
            "POST",
            "/api/ironhub/verify-intent",
            Body::from(body.clone()),
            "regular",
        );
        let r1 = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), first)
            .await
            .expect("first");
        assert_eq!(r1.status(), StatusCode::OK);
        let second = req_with_identity(
            "POST",
            "/api/ironhub/verify-intent",
            Body::from(body),
            "regular",
        );
        let r2 = ServiceExt::<axum::http::Request<Body>>::oneshot(app, second)
            .await
            .expect("second");
        assert_eq!(r2.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn register_is_rate_limited_per_user() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .user_id("test-user")
            .with_secrets_store(secrets)
            .with_ironhub_catalog_rate_limit(1, 60)
            .build();
        let app = Router::new()
            .route("/api/ironhub/register", post(ironhub_register_handler))
            .with_state(state);
        let body = serde_json::json!({
            "uid": "u1",
            "aid": "a1",
            "ts": now_unix(),
            "nonce": "rl-register-1",
            "sig": "deadbeef"
        })
        .to_string();
        let first = req_with_identity(
            "POST",
            "/api/ironhub/register",
            Body::from(body.clone()),
            "regular",
        );
        let r1 = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), first)
            .await
            .expect("first");
        assert_eq!(r1.status(), StatusCode::SERVICE_UNAVAILABLE);
        let second =
            req_with_identity("POST", "/api/ironhub/register", Body::from(body), "regular");
        let r2 = ServiceExt::<axum::http::Request<Body>>::oneshot(app, second)
            .await
            .expect("second");
        assert_eq!(r2.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    async fn verify_app() -> (Router, String, String, u64) {
        use crate::secrets::CreateSecretParams;

        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        secrets
            .create(
                "test-user",
                CreateSecretParams::new("ironhub_signing_key", TEST_SHARED_KEY),
            )
            .await
            .expect("seed signing key");

        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .user_id("test-user")
            .with_secrets_store(secrets)
            .build();
        let app = Router::new()
            .route(
                "/api/ironhub/verify-intent",
                post(ironhub_verify_intent_handler),
            )
            .route("/api/ironhub/register", post(ironhub_register_handler))
            .with_state(state);
        let ts = now_unix();
        let nonce = uuid::Uuid::new_v4().to_string();
        let payload = install_payload(
            "clickup",
            "1.0.0",
            "u1",
            "a1",
            ts,
            &nonce,
            TEST_ARTIFACT_DIGEST,
        );
        let sig = hmac_hex(TEST_SHARED_KEY, &payload).expect("sign");
        (app, sig, nonce, ts)
    }

    fn verify_body(slug: &str, ts: u64, nonce: &str, sig: &str) -> serde_json::Value {
        serde_json::json!({
            "slug": slug,
            "version": "1.0.0",
            "uid": "u1",
            "aid": "a1",
            "ts": ts,
            "nonce": nonce,
            "sig": sig,
            "artifact_digest": TEST_ARTIFACT_DIGEST,
        })
    }

    fn verify_req(body: serde_json::Value) -> axum::http::Request<Body> {
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/ironhub/verify-intent")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test-user".into(),
            role: "regular".into(),
            workspace_read_scopes: Vec::new(),
        });
        req
    }

    fn register_req(body: serde_json::Value) -> axum::http::Request<Body> {
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/ironhub/register")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test-user".into(),
            role: "regular".into(),
            workspace_read_scopes: Vec::new(),
        });
        req
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 65536)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[test]
    fn tool_error_to_http_maps_all_variants() {
        use std::time::Duration;
        assert_eq!(
            tool_error_to_http(ToolError::InvalidParameters("x".into())).0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            tool_error_to_http(ToolError::NotAuthorized("x".into())).0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            tool_error_to_http(ToolError::RateLimited(None)).0,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            tool_error_to_http(ToolError::RateLimited(Some(Duration::from_secs(5)))).0,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            tool_error_to_http(ToolError::Timeout(Duration::from_secs(3))).0,
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            tool_error_to_http(ToolError::ExternalService("x".into())).0,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            tool_error_to_http(ToolError::ExecutionFailed("x".into())).0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            tool_error_to_http(ToolError::Sandbox("x".into())).0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn verify_intent_returns_invalid_when_no_signing_key() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .user_id("test-user")
            .with_secrets_store(secrets)
            .build();
        let app = Router::new()
            .route(
                "/api/ironhub/verify-intent",
                post(ironhub_verify_intent_handler),
            )
            .with_state(state);
        let req = verify_req(verify_body(
            "clickup",
            now_unix(),
            "nokey-nonce",
            "deadbeef",
        ));
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["valid"], false);
        assert!(
            json["reason"]
                .as_str()
                .unwrap_or("")
                .contains("no signing key"),
            "{json:?}"
        );
    }

    #[tokio::test]
    async fn register_rejects_replayed_nonce() {
        let (app, _sig, _nonce, ts) = verify_app().await;
        let nonce = "register-replay-nonce";
        let payload = register_payload("u1", "a1", ts, nonce);
        let sig = hmac_hex(TEST_SHARED_KEY, &payload).expect("sign");
        let body = serde_json::json!({
            "uid": "u1",
            "aid": "a1",
            "ts": ts,
            "nonce": nonce,
            "sig": sig,
        });
        let first = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app.clone(),
            register_req(body.clone()),
        )
        .await
        .expect("first");
        assert_eq!(first.status(), StatusCode::OK);
        let second = ServiceExt::<axum::http::Request<Body>>::oneshot(app, register_req(body))
            .await
            .expect("second");
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn verify_intent_accepts_valid_signature() {
        let (app, sig, nonce, ts) = verify_app().await;
        let req = verify_req(verify_body("clickup", ts, &nonce, &sig));
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["valid"], true, "valid sig must verify: {json:?}");
    }

    #[tokio::test]
    async fn verify_intent_rejects_tampered_signature() {
        let (app, _sig, nonce, ts) = verify_app().await;
        let req = verify_req(verify_body("clickup", ts, &nonce, &"deadbeef".repeat(8)));
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        let json = body_json(resp).await;
        assert_eq!(json["valid"], false);
        assert!(json["reason"].as_str().unwrap().contains("mismatch"));
    }

    #[tokio::test]
    async fn verify_intent_rejects_expired_timestamp() {
        let (app, _sig, _nonce, _ts) = verify_app().await;
        let stale_ts = now_unix() - 4000;
        let req = verify_req(verify_body(
            "clickup",
            stale_ts,
            "n_stale",
            &"00".repeat(32),
        ));
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        let json = body_json(resp).await;
        assert_eq!(json["valid"], false);
        assert!(json["reason"].as_str().unwrap().contains("drift"));
    }

    #[tokio::test]
    async fn verify_intent_rejects_invalid_slug() {
        let (app, _sig, nonce, ts) = verify_app().await;
        let req = verify_req(verify_body("../etc/passwd", ts, &nonce, &"00".repeat(32)));
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        let json = body_json(resp).await;
        assert_eq!(json["valid"], false);
        assert!(json["reason"].as_str().unwrap().contains("invalid slug"));
    }

    #[tokio::test]
    async fn verify_intent_is_repeatable_preview() {
        let (app, sig, nonce, ts) = verify_app().await;
        let first = verify_req(verify_body("clickup", ts, &nonce, &sig));
        let resp1 = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), first)
            .await
            .expect("first response");
        assert_eq!(resp1.status(), StatusCode::OK);
        assert_eq!(body_json(resp1).await["valid"], true);

        let again = verify_req(verify_body("clickup", ts, &nonce, &sig));
        let resp2 = ServiceExt::<axum::http::Request<Body>>::oneshot(app, again)
            .await
            .expect("second response");
        assert_eq!(resp2.status(), StatusCode::OK);
        assert_eq!(
            body_json(resp2).await["valid"],
            true,
            "verify-intent is a preview and must not consume the one-shot nonce"
        );
    }

    #[tokio::test]
    async fn verify_intent_requires_authentication() {
        let (app, sig, nonce, ts) = verify_app().await;
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/ironhub/verify-intent")
            .header("content-type", "application/json")
            .body(Body::from(
                verify_body("clickup", ts, &nonce, &sig).to_string(),
            ))
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    async fn signing_key_app() -> Router {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .user_id("test-user")
            .with_secrets_store(secrets)
            .build();
        Router::new()
            .route(
                "/api/ironhub/signing-key",
                post(ironhub_signing_key_set_handler)
                    .get(ironhub_signing_key_get_handler)
                    .delete(ironhub_signing_key_delete_handler),
            )
            .with_state(state)
    }

    fn signing_key_req(method: &str, body: Body) -> axum::http::Request<Body> {
        signing_key_req_as(method, body, "admin")
    }

    fn signing_key_req_as(method: &str, body: Body, role: &str) -> axum::http::Request<Body> {
        let mut req = axum::http::Request::builder()
            .method(method)
            .uri("/api/ironhub/signing-key")
            .header("content-type", "application/json")
            .body(body)
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test-user".into(),
            role: role.into(),
            workspace_read_scopes: Vec::new(),
        });
        req
    }

    #[tokio::test]
    async fn signing_key_set_accepts_valid_prefix_and_returns_metadata() {
        let app = signing_key_app().await;
        let body = serde_json::json!({"shared_key": TEST_SHARED_KEY}).to_string();
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req("POST", Body::from(body)),
        )
        .await
        .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let meta = body_json(resp).await;
        assert_eq!(meta["fingerprint"].as_str().unwrap().len(), 12);
        assert!(meta["created_at"].as_str().is_some());
        assert!(
            meta.get("shared_key").is_none(),
            "set response must NOT echo the key"
        );
    }

    #[tokio::test]
    async fn signing_key_set_rejects_missing_prefix() {
        let app = signing_key_app().await;
        let body =
            serde_json::json!({"shared_key": "no_prefix_key_with_enough_length_xx"}).to_string();
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req("POST", Body::from(body)),
        )
        .await
        .expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signing_key_set_rejects_too_short() {
        let app = signing_key_app().await;
        let body = serde_json::json!({"shared_key": "ihub_sk_short"}).to_string();
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req("POST", Body::from(body)),
        )
        .await
        .expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signing_key_get_returns_404_when_unset() {
        let app = signing_key_app().await;
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req("GET", Body::empty()),
        )
        .await
        .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn signing_key_get_returns_metadata_without_exposing_key() {
        let app = signing_key_app().await;
        let set_body = serde_json::json!({"shared_key": TEST_SHARED_KEY}).to_string();
        let set_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app.clone(),
            signing_key_req("POST", Body::from(set_body)),
        )
        .await
        .expect("set response");
        assert_eq!(set_resp.status(), StatusCode::OK);

        let get_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req("GET", Body::empty()),
        )
        .await
        .expect("get response");
        assert_eq!(get_resp.status(), StatusCode::OK);
        let meta = body_json(get_resp).await;
        assert!(meta["fingerprint"].as_str().is_some());
        let serialized = serde_json::to_string(&meta).unwrap();
        assert!(
            !serialized.contains(TEST_SHARED_KEY),
            "GET response must never echo the raw key: {serialized}"
        );
    }

    #[tokio::test]
    async fn signing_key_delete_removes_existing_and_404s_on_missing() {
        let app = signing_key_app().await;
        let set_body = serde_json::json!({"shared_key": TEST_SHARED_KEY}).to_string();
        ServiceExt::<axum::http::Request<Body>>::oneshot(
            app.clone(),
            signing_key_req("POST", Body::from(set_body)),
        )
        .await
        .expect("set");

        let del1 = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app.clone(),
            signing_key_req("DELETE", Body::empty()),
        )
        .await
        .expect("delete");
        assert_eq!(del1.status(), StatusCode::NO_CONTENT);

        let del2 = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req("DELETE", Body::empty()),
        )
        .await
        .expect("second delete");
        assert_eq!(del2.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn signing_key_set_rejects_non_admin() {
        let app = signing_key_app().await;
        let body = serde_json::json!({"shared_key": TEST_SHARED_KEY}).to_string();
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req_as("POST", Body::from(body), "regular"),
        )
        .await
        .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn signing_key_set_replaces_existing_and_updates_fingerprint() {
        let app = signing_key_app().await;
        let first = serde_json::json!({"shared_key": TEST_SHARED_KEY}).to_string();
        let first_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app.clone(),
            signing_key_req("POST", Body::from(first)),
        )
        .await
        .expect("first set");
        assert_eq!(first_resp.status(), StatusCode::OK);
        let first_fp = body_json(first_resp).await["fingerprint"]
            .as_str()
            .expect("fingerprint")
            .to_string();

        let rotated = format!("{TEST_SHARED_KEY}_rotated");
        let second = serde_json::json!({"shared_key": rotated}).to_string();
        let second_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(
            app,
            signing_key_req("POST", Body::from(second)),
        )
        .await
        .expect("second set");
        assert_eq!(second_resp.status(), StatusCode::OK);
        let second_fp = body_json(second_resp).await["fingerprint"]
            .as_str()
            .expect("fingerprint")
            .to_string();

        assert_ne!(
            first_fp, second_fp,
            "replacing the signing key must change the returned fingerprint"
        );
    }

    #[tokio::test]
    async fn register_accepts_valid_signature() {
        let (app, _intent_sig, _intent_nonce, ts) = verify_app().await;
        let nonce = uuid::Uuid::new_v4().to_string();
        let payload = register_payload("u1", "a1", ts, &nonce);
        let sig = hmac_hex(TEST_SHARED_KEY, &payload).expect("sign");
        let body = serde_json::json!({
            "uid": "u1",
            "aid": "a1",
            "ts": ts,
            "nonce": nonce,
            "sig": sig,
        });
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, register_req(body))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn register_rejects_bad_signature() {
        let (app, _intent_sig, _intent_nonce, ts) = verify_app().await;
        let nonce = uuid::Uuid::new_v4().to_string();
        let body = serde_json::json!({
            "uid": "u1",
            "aid": "a1",
            "ts": ts,
            "nonce": nonce,
            "sig": "deadbeef".repeat(8),
        });
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, register_req(body))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn register_rejects_expired_timestamp() {
        let (app, _intent_sig, _intent_nonce, _ts) = verify_app().await;
        let stale_ts = now_unix() - 4000;
        let nonce = uuid::Uuid::new_v4().to_string();
        let payload = register_payload("u1", "a1", stale_ts, &nonce);
        let sig = hmac_hex(TEST_SHARED_KEY, &payload).expect("sign");
        let body = serde_json::json!({
            "uid": "u1",
            "aid": "a1",
            "ts": stale_ts,
            "nonce": nonce,
            "sig": sig,
        });
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, register_req(body))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::REQUEST_TIMEOUT);
    }

    #[tokio::test]
    async fn register_rejects_when_no_signing_key_configured() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let state = crate::channels::web::test_helpers::TestGatewayBuilder::new()
            .user_id("test-user")
            .with_secrets_store(secrets)
            .build();
        let app = Router::new()
            .route("/api/ironhub/register", post(ironhub_register_handler))
            .with_state(state);
        let body = serde_json::json!({
            "uid": "u1",
            "aid": "a1",
            "ts": now_unix(),
            "nonce": uuid::Uuid::new_v4().to_string(),
            "sig": "deadbeef".repeat(8),
        });
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, register_req(body))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
