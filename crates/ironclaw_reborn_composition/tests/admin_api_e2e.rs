//! End-to-end coverage for the WebChat v2 admin user-management surface.
//!
//! Unlike the crate-tier facade tests (which drive `RebornServicesApi` against
//! a fake port), this stands up a REAL local-dev `RebornRuntime` with the admin
//! service wired (identity user-directory + admin secret provisioner + a signed
//! session-store token minter), composes the full `webui_v2_app` with a real
//! bearer authenticator, and drives the whole admin surface over HTTP through
//! `tower::ServiceExt::oneshot`.
//!
//! The flagship proof: an admin (operator bearer) creates a user, receives the
//! one-time `api_token`, and that token then authenticates a follow-up request
//! AS the new user — exercising the entire mint → return → validate chain
//! (facade → composition adapter → identity store → minter → the session
//! authenticator that validates the bearer). Nothing above the token crypto is
//! stubbed.

#![cfg(all(feature = "webui-v2-beta", feature = "test-support"))]

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    AdminApiTokenMinter, PollSettings, RebornBuildInput, RebornRuntime, RebornRuntimeIdentity,
    RebornRuntimeInput, WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig,
    build_reborn_runtime, build_webui_services, webui_v2_app,
};
use ironclaw_reborn_webui_ingress::{
    EnvBearerAuthenticator, SessionAuthenticator, SessionStore, signed_session_store,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use tower::ServiceExt;

const TENANT: &str = "admin-e2e-tenant";
const AGENT: &str = "admin-e2e-agent";
const OPERATOR_USER: &str = "admin-e2e-operator";
const OPERATOR_TOKEN: &str = "operator-secret-token";

// ─── no-op model gateway (admin ops never invoke the model) ───────────────

struct NoOpGateway;

#[async_trait]
impl HostManagedModelGateway for NoOpGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // Admin CRUD never dispatches a turn, so this is unreachable in these
        // tests; fail loudly if that assumption ever breaks.
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "admin e2e must not invoke the model gateway".to_string(),
        ))
    }
}

// ─── token minter (mirrors production's serve-layer minter) ───────────────

struct SessionTokenMinter {
    store: Arc<dyn SessionStore>,
}

#[async_trait]
impl AdminApiTokenMinter for SessionTokenMinter {
    async fn mint(&self, tenant: &TenantId, user_id: &UserId) -> Result<SecretString, String> {
        self.store
            .create_session(tenant.clone(), user_id.clone(), chrono::Duration::days(365))
            .await
            .map_err(|error| error.to_string())
    }
}

// ─── authenticator: operator env-bearer OR minted session bearer ──────────

struct DualAuthenticator {
    env: EnvBearerAuthenticator,
    session: SessionAuthenticator,
}

#[async_trait]
impl WebuiAuthenticator for DualAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        // Operator token first (implicit admin); otherwise a minted session
        // bearer resolves to its (non-operator) user.
        if let Some(auth) = self.env.authenticate(token).await {
            return Some(auth);
        }
        self.session.authenticate(token).await
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        true
    }
}

fn local_dev_effective_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

// ─── harness ──────────────────────────────────────────────────────────────

struct AdminHarness {
    router: axum::Router,
    // Kept alive for the test: the runtime owns the durable stores the router
    // reads through, and the tempdir backs them.
    _runtime: RebornRuntime,
    _root: tempfile::TempDir,
}

async fn build_admin_harness() -> AdminHarness {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root: PathBuf = root.path().join("local-dev");
    let tenant = TenantId::new(TENANT).expect("tenant");

    // One signed session store, shared by the minter (issues bearers) and the
    // authenticator (validates them) — the same instance, so a minted token is
    // always accepted.
    let operator_secret = SecretString::from(OPERATOR_TOKEN.to_string());
    let session_store = signed_session_store(&operator_secret, &tenant);
    let minter: Arc<dyn AdminApiTokenMinter> = Arc::new(SessionTokenMinter {
        store: session_store.clone(),
    });

    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(OPERATOR_USER, storage_root)
            .with_runtime_policy(local_dev_effective_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: TENANT.to_string(),
        agent_id: AGENT.to_string(),
        source_binding_id: "admin-e2e-source".to_string(),
        reply_target_binding_id: "admin-e2e-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: std::time::Duration::from_millis(10),
        max_total: std::time::Duration::from_secs(10),
    })
    .with_model_gateway_override(Arc::new(NoOpGateway))
    .with_admin_api_token_minter(minter);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let bundle = build_webui_services(&runtime, None).expect("webui bundle");

    let env =
        EnvBearerAuthenticator::new(operator_secret, UserId::new(OPERATOR_USER).expect("user"))
            .expect("env authenticator");
    let session = SessionAuthenticator::new(session_store);
    let authenticator = Arc::new(DualAuthenticator { env, session });

    let config = WebuiServeConfig::new(
        tenant,
        authenticator,
        vec![HeaderValue::from_static("http://localhost:0")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"));
    let router = webui_v2_app(bundle, config).expect("webui v2 app");

    AdminHarness {
        router,
        _runtime: runtime,
        _root: root,
    }
}

// ─── reusable admin API driver ─────────────────────────────────────────────

/// Drives the composed admin HTTP surface as a specific bearer principal.
/// `as_bearer` clones the driver bound to a different token (e.g. a minted
/// user bearer) so one harness can exercise operator, admin, and member
/// principals against the same app.
#[derive(Clone)]
struct AdminApiDriver {
    router: axum::Router,
    bearer: String,
}

impl AdminApiDriver {
    fn new(router: axum::Router, bearer: impl Into<String>) -> Self {
        Self {
            router,
            bearer: bearer.into(),
        }
    }

    fn as_bearer(&self, bearer: impl Into<String>) -> Self {
        Self {
            router: self.router.clone(),
            bearer: bearer.into(),
        }
    }

    async fn send(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.bearer));
        let request = match body {
            Some(body) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                builder.body(Body::from(body.to_string())).expect("request")
            }
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.router.clone().oneshot(request).await.expect("oneshot");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 256 * 1024)
            .await
            .expect("body within cap");
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    async fn session(&self) -> (StatusCode, Value) {
        self.send(Method::GET, "/api/webchat/v2/session", None)
            .await
    }

    async fn list_users(&self) -> (StatusCode, Value) {
        self.send(Method::GET, "/api/webchat/v2/admin/users", None)
            .await
    }

    async fn create_user(
        &self,
        email: Option<&str>,
        display_name: &str,
        role: &str,
    ) -> (StatusCode, Value) {
        let mut body = serde_json::Map::new();
        if let Some(email) = email {
            body.insert("email".to_string(), json!(email));
        }
        body.insert("display_name".to_string(), json!(display_name));
        body.insert("role".to_string(), json!(role));
        self.send(
            Method::POST,
            "/api/webchat/v2/admin/users",
            Some(Value::Object(body)),
        )
        .await
    }

    async fn get_user(&self, user_id: &str) -> (StatusCode, Value) {
        self.send(
            Method::GET,
            &format!("/api/webchat/v2/admin/users/{user_id}"),
            None,
        )
        .await
    }

    async fn set_role(&self, user_id: &str, role: &str) -> (StatusCode, Value) {
        self.send(
            Method::POST,
            &format!("/api/webchat/v2/admin/users/{user_id}/role"),
            Some(json!({ "role": role })),
        )
        .await
    }

    async fn set_status(&self, user_id: &str, status: &str) -> (StatusCode, Value) {
        self.send(
            Method::POST,
            &format!("/api/webchat/v2/admin/users/{user_id}/status"),
            Some(json!({ "status": status })),
        )
        .await
    }

    async fn delete_user(&self, user_id: &str) -> (StatusCode, Value) {
        self.send(
            Method::DELETE,
            &format!("/api/webchat/v2/admin/users/{user_id}"),
            None,
        )
        .await
    }

    async fn put_secret(&self, user_id: &str, handle: &str, value: &str) -> (StatusCode, Value) {
        self.send(
            Method::PUT,
            &format!("/api/webchat/v2/admin/users/{user_id}/secrets/{handle}"),
            Some(json!({ "value": value })),
        )
        .await
    }

    async fn list_secrets(&self, user_id: &str) -> (StatusCode, Value) {
        self.send(
            Method::GET,
            &format!("/api/webchat/v2/admin/users/{user_id}/secrets"),
            None,
        )
        .await
    }
}

fn user_id_of(created: &Value) -> String {
    created["user"]["user_id"]
        .as_str()
        .expect("created user carries a user_id")
        .to_string()
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_full_lifecycle_and_api_token_login() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    // Fresh tenant: no user records until the admin creates some.
    let (status, users) = operator.list_users().await;
    assert_eq!(status, StatusCode::OK, "operator may list users");
    assert_eq!(
        users["users"].as_array().map(Vec::len),
        Some(0),
        "no users exist before the admin creates any"
    );

    // Create an admin and a member.
    let (status, admin) = operator
        .create_user(Some("admin@acme.test"), "Ada Admin", "admin")
        .await;
    assert_eq!(status, StatusCode::OK, "operator creates an admin user");
    let admin_id = user_id_of(&admin);
    let admin_token = admin["api_token"]
        .as_str()
        .expect("create returns a one-time api_token")
        .to_string();

    let (status, member) = operator
        .create_user(Some("member@acme.test"), "Mo Member", "member")
        .await;
    assert_eq!(status, StatusCode::OK, "operator creates a member user");
    let member_id = user_id_of(&member);
    let member_token = member["api_token"]
        .as_str()
        .expect("create returns a one-time api_token")
        .to_string();

    let (_, users) = operator.list_users().await;
    assert_eq!(
        users["users"].as_array().map(Vec::len),
        Some(2),
        "both created users are enumerated"
    );

    // ── The flagship proof: the minted token logs in AS the new user. ──
    let admin_session = operator.as_bearer(&admin_token);
    let (status, session) = admin_session.session().await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the minted admin token authenticates"
    );
    assert_eq!(
        session["user_id"].as_str(),
        Some(admin_id.as_str()),
        "logging in with the API token resolves to the created user"
    );

    // An admin-role token clears the admin boundary; a member-role token does not.
    let (status, _) = admin_session.list_users().await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an admin-role session may list users"
    );

    let member_session = operator.as_bearer(&member_token);
    let (status, member_who) = member_session.session().await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the member token also authenticates"
    );
    assert_eq!(
        member_who["user_id"].as_str(),
        Some(member_id.as_str()),
        "the member token resolves to the member user"
    );
    let (status, _) = member_session.list_users().await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a member must not reach the admin surface"
    );

    // Role + status mutations round-trip.
    let (status, promoted) = operator.set_role(&member_id, "admin").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(promoted["user"]["role"].as_str(), Some("admin"));

    let (status, suspended) = operator.set_status(&member_id, "suspended").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(suspended["user"]["status"].as_str(), Some("suspended"));

    // Per-user secret provisioning: put then list; the material is never echoed.
    let (status, put) = operator
        .put_secret(&admin_id, "openai_key", "sk-super-secret-value")
        .await;
    assert_eq!(status, StatusCode::OK, "admin provisions a per-user secret");
    assert_eq!(put["secret"]["handle"].as_str(), Some("openai_key"));
    assert!(
        !put.to_string().contains("sk-super-secret-value"),
        "the secret material must never be echoed back"
    );
    let (status, secrets) = operator.list_secrets(&admin_id).await;
    assert_eq!(status, StatusCode::OK);
    let handles: Vec<&str> = secrets["secrets"]
        .as_array()
        .expect("secrets list")
        .iter()
        .filter_map(|entry| entry["handle"].as_str())
        .collect();
    assert!(
        handles.contains(&"openai_key"),
        "the provisioned handle lists"
    );
    assert!(
        !secrets.to_string().contains("sk-super-secret-value"),
        "listing secrets must not expose material"
    );

    // Delete cascades: the record is gone, and a re-read is a 404.
    let (status, deleted) = operator.delete_user(&member_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted["deleted"].as_bool(), Some(true));
    let (status, _) = operator.get_user(&member_id).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a deleted user reads as not found"
    );
}

#[tokio::test]
async fn admin_last_admin_protection_over_http() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    // One admin user record → it is the sole active admin.
    let (_, sole) = operator.create_user(None, "Sole Admin", "admin").await;
    let sole_id = user_id_of(&sole);

    let (status, demote) = operator.set_role(&sole_id, "member").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "demoting the sole admin is blocked"
    );
    assert_eq!(
        demote["field"].as_str(),
        Some("last_admin"),
        "the block carries the stable last_admin marker"
    );
    let (status, _) = operator.set_status(&sole_id, "suspended").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "suspending the sole admin is blocked"
    );

    // A second admin removes the protection.
    let (_, second) = operator.create_user(None, "Second Admin", "admin").await;
    let second_id = user_id_of(&second);
    let (status, _) = operator.set_role(&second_id, "member").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "demoting one of two admins is allowed"
    );
}
