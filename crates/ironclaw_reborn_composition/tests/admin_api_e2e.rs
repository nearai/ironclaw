//! End-to-end coverage for the WebChat v2 admin user-management surface.
//!
//! Unlike the crate-tier facade tests (which drive `ProductSurface` against
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

#![cfg(feature = "test-support")]

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
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    AdminApiTokenMinter, PollSettings, RebornHostBindings, RebornRuntime, RebornRuntimeIdentity,
    RebornRuntimeInput, build_reborn_runtime,
};
use ironclaw_webui::{
    EnvBearerAuthenticator, SessionAuthenticator, SignedTokenSessionStore, signed_session_store,
};
use ironclaw_webui::{WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, webui_v2_app};
use secrecy::{ExposeSecret, SecretString};
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
    store: Arc<SignedTokenSessionStore>,
}

#[async_trait]
impl AdminApiTokenMinter for SessionTokenMinter {
    async fn mint(&self, tenant: &TenantId, user_id: &UserId) -> Result<SecretString, String> {
        self.store
            .create_session(
                tenant.clone(),
                user_id.clone(),
                chrono::Duration::days(365),
                false,
            )
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
    let build_input =
        ironclaw_reborn_composition::local_dev_build_input(OPERATOR_USER, storage_root)
            .with_runtime_policy(local_dev_effective_policy());
    build_admin_harness_from(root, build_input).await
}

/// Assemble the full admin HTTP harness over a caller-supplied
/// `RebornHostBindings` (already carrying its profile, policy, trust, and process
/// binding). Everything above the substrate — the shared signed session store /
/// minter / authenticator, the product surface, and the composed router — is
/// profile-agnostic, so the local-dev and production-shaped runs share it and
/// only differ in the build input.
async fn build_admin_harness_from(
    root: tempfile::TempDir,
    build_input: RebornHostBindings,
) -> AdminHarness {
    let tenant = TenantId::new(TENANT).expect("tenant");

    // One signed session store, shared by the minter (issues bearers) and the
    // authenticator (validates them) — the same instance, so a minted token is
    // always accepted.
    let operator_secret = SecretString::from(OPERATOR_TOKEN.to_string());
    let session_store = signed_session_store(&operator_secret, &tenant);
    let minter: Arc<dyn AdminApiTokenMinter> = Arc::new(SessionTokenMinter {
        store: session_store.clone(),
    });

    let input = RebornRuntimeInput::from_build_input(build_input)
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
    let bundle = runtime.product_surface(None).expect("product surface");

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
    let router = webui_v2_app(bundle.clone(), config).expect("webui v2 app");

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

// ─── adversarial / corner-case coverage ─────────────────────────────────────

/// Build a signed-session store keyed by `(secret, TENANT)`. Because the store
/// is deterministic in that pair, a store built here with `OPERATOR_TOKEN` mints
/// bearers that validate under the harness's own authenticator (the exact
/// property `signed_session_store`'s doc-comment guarantees); a store built with
/// a *different* secret derives a different HMAC key, so its tokens fail closed.
fn session_store_with_secret(secret: &str) -> Arc<SignedTokenSessionStore> {
    signed_session_store(
        &SecretString::from(secret.to_string()),
        &TenantId::new(TENANT).expect("tenant"),
    )
}

/// Create an admin user via the API as the operator and return `(user_id,
/// api_token)`. The `api_token` is the one-time minted session bearer.
async fn create_admin(operator: &AdminApiDriver, display_name: &str) -> (String, String) {
    let (status, created) = operator.create_user(None, display_name, "admin").await;
    assert_eq!(status, StatusCode::OK, "operator creates an admin user");
    let id = user_id_of(&created);
    let token = created["api_token"]
        .as_str()
        .expect("create returns a one-time api_token")
        .to_string();
    (id, token)
}

/// 1. A deleted admin's minted token loses admin access: with no user record,
///    `authorize_admin` fails closed → 403 on any admin verb.
#[tokio::test]
async fn deleted_admin_token_is_denied_on_admin_routes() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    // A second admin so deleting the first is not blocked by last-admin
    // protection (delete of the sole admin would 409, not 200).
    let (admin_id, admin_token) = create_admin(&operator, "Del Admin").await;
    let _ = create_admin(&operator, "Keep Admin").await;

    // Sanity: the token clears the admin boundary while the record exists.
    let admin_session = operator.as_bearer(&admin_token);
    let (status, _) = admin_session.list_users().await;
    assert_eq!(status, StatusCode::OK, "admin token works before delete");

    let (status, deleted) = operator.delete_user(&admin_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted["deleted"].as_bool(), Some(true));

    // The deleted user's token no longer authorizes admin actions: the record
    // is gone, so `authorize_admin`'s `get_user` returns None → 403.
    let (status, _) = admin_session.list_users().await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a deleted admin's token must not reach the admin surface"
    );

    // KNOWN REVOCATION GAP (verified, deliberately NOT asserted as desired):
    // signed session tokens are stateless and are NOT revoked on user delete,
    // so the same deleted token still authenticates a non-admin route
    // (`GET /api/webchat/v2/session` returns 200 as the tombstoned user id).
    // We exercise that path to document the gap but do not lock it in with an
    // assertion — when session revocation-on-delete lands this should change to
    // a 401, and a test asserting 200 here would then wrongly fail-block the fix.
    let _ = admin_session.session().await;
}

/// 2. A suspended admin's own token loses admin access. Relies on the
///    status-gate in `authorize_admin` (role.is_admin() AND status==Active).
#[tokio::test]
async fn suspended_admin_token_is_denied_on_admin_routes() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    // Two admins so suspending one is not blocked by last-admin protection.
    let (admin_id, admin_token) = create_admin(&operator, "Suspendable Admin").await;
    let _ = create_admin(&operator, "Other Admin").await;

    let admin_session = operator.as_bearer(&admin_token);
    let (status, _) = admin_session.list_users().await;
    assert_eq!(status, StatusCode::OK, "admin token works while active");

    let (status, suspended) = operator.set_status(&admin_id, "suspended").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(suspended["user"]["status"].as_str(), Some("suspended"));

    // The suspended admin keeps the `admin` role but a non-Active status must
    // immediately revoke admin API access.
    let (status, _) = admin_session.list_users().await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a suspended admin's token must not reach the admin surface"
    );
}

/// 3. A minted admin *session* bearer is admin for user-management but must NOT
///    carry `operator_webui_config`: it is rejected on an operator-gated route
///    (`GET /api/webchat/v2/operator/status`) that the operator env-bearer may
///    reach. The harness mounts operator routes
///    (`mounts_operator_webui_config_routes() == true`).
#[tokio::test]
async fn admin_session_bearer_cannot_reach_operator_routes() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);
    let (_admin_id, admin_token) = create_admin(&operator, "Ops Curious Admin").await;

    const OPERATOR_STATUS: &str = "/api/webchat/v2/operator/status";

    // The operator env-bearer clears the operator capability gate.
    let (status, _) = operator.send(Method::GET, OPERATOR_STATUS, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "operator env-bearer reaches the operator status route"
    );

    // The admin session bearer is admin for user CRUD but not an operator: the
    // capability gate in composition rejects it before the handler runs.
    let admin_session = operator.as_bearer(&admin_token);
    let (status, _) = admin_session.list_users().await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the admin session bearer IS admin for user management"
    );
    let (status, _) = admin_session.send(Method::GET, OPERATOR_STATUS, None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an admin session bearer must NOT carry operator_webui_config"
    );
}

/// 4. Forged / tampered / expired tokens are rejected at the auth boundary with
///    a 401 — they never reach the admin facade.
#[tokio::test]
async fn forged_and_expired_tokens_are_rejected() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);
    let admin_route = "/api/webchat/v2/admin/users";

    let tenant = TenantId::new(TENANT).expect("tenant");
    let user = UserId::new("forge-victim").expect("user");

    // (a) random garbage bearer → 401.
    let (status, _) = operator
        .as_bearer("not-a-real-token")
        .send(Method::GET, admin_route, None)
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "garbage bearer is rejected"
    );

    // Mint a genuine, in-secret bearer (validates under the harness).
    let good_store = session_store_with_secret(OPERATOR_TOKEN);
    let good_token = good_store
        .create_session(
            tenant.clone(),
            user.clone(),
            chrono::Duration::days(1),
            false,
        )
        .await
        .expect("mint valid token")
        .expose_secret()
        .to_string();

    // (b) a bit-flipped valid token breaks the HMAC → 401.
    let mut flipped: Vec<char> = good_token.chars().collect();
    let last = flipped.len() - 1;
    flipped[last] = if flipped[last] == 'A' { 'B' } else { 'A' };
    let flipped: String = flipped.into_iter().collect();
    assert_ne!(flipped, good_token, "bit-flip actually changed the token");
    let (status, _) = operator
        .as_bearer(&flipped)
        .send(Method::GET, admin_route, None)
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a tampered token is rejected"
    );

    // (c) a token minted under a DIFFERENT operator secret → 401 (foreign key).
    let foreign_token = session_store_with_secret("a-totally-different-operator-secret")
        .create_session(
            tenant.clone(),
            user.clone(),
            chrono::Duration::days(1),
            false,
        )
        .await
        .expect("mint foreign token")
        .expose_secret()
        .to_string();
    let (status, _) = operator
        .as_bearer(&foreign_token)
        .send(Method::GET, admin_route, None)
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a token signed by a different secret is rejected"
    );

    // (d) an expired session → 401. The store fails LOUD on a non-positive
    // lifetime (it refuses to mint an already-dead token), so we cannot mint
    // one with `Duration::zero()` — assert that fail-loud contract, then mint a
    // minimum 1s token and let it lapse (exp is second-granularity, so wait
    // past the next whole second).
    let zero = good_store
        .create_session(
            tenant.clone(),
            user.clone(),
            chrono::Duration::zero(),
            false,
        )
        .await;
    assert!(
        zero.is_err(),
        "create_session refuses a zero/negative lifetime rather than minting a dead token"
    );
    let expiring = good_store
        .create_session(tenant, user, chrono::Duration::seconds(1), false)
        .await
        .expect("mint short-lived token")
        .expose_secret()
        .to_string();
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    let (status, _) = operator
        .as_bearer(&expiring)
        .send(Method::GET, admin_route, None)
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an expired session token is rejected"
    );
}

/// 5. An oversized create-user body is rejected with 413 by the descriptor
///    body-limit middleware BEFORE the facade runs. The `admin_create_user`
///    route declares a 16 KiB per-route cap.
#[tokio::test]
async fn oversized_create_body_is_rejected_with_413() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    // 20 KiB display_name → well past the 16 KiB `admin_create_user` cap.
    let huge = "x".repeat(20 * 1024);
    let body = json!({ "display_name": huge, "role": "member" });
    let (status, _) = operator
        .send(Method::POST, "/api/webchat/v2/admin/users", Some(body))
        .await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "an oversized create body is rejected by the per-route 16 KiB cap"
    );
}

/// 6. A path-traversal-shaped secret handle cannot escape the target user's
///    secret namespace. `SecretHandle` validation rejects `/` and dot-dot
///    segments, so nothing is ever written; containment holds (fail-closed).
///    The rejection now maps to a **400** (client input at fault), not a 500 —
///    `admin_user_directory.rs` maps the `SecretHandle` construction failure to
///    `AdminUserError::InvalidInput`. (Previously it returned 500 with a comment
///    falsely claiming the handle was validated at the HTTP edge; this test pins
///    the corrected 4xx.)
#[tokio::test]
async fn secret_handle_path_traversal_is_contained() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    let (target_id, _) = create_admin(&operator, "Secret Target").await;
    let (other_id, _) = create_admin(&operator, "Bystander").await;

    for handle in ["..%2F..%2Fother", "a%2Fb"] {
        let (status, _) = operator
            .send(
                Method::PUT,
                &format!("/api/webchat/v2/admin/users/{target_id}/secrets/{handle}"),
                Some(json!({ "value": "sk-should-never-persist" })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a traversal-shaped handle {handle:?} is a client error (400), not a 500"
        );
        // Containment is enforced by SecretHandle validation rejecting the path
        // separators before any write; the failure is a 400, not an internal 500.
    }

    // Nothing leaked into the target's namespace...
    let (status, secrets) = operator.list_secrets(&target_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        secrets["secrets"].as_array().map(Vec::len),
        Some(0),
        "no secret was written under the target user despite the traversal attempt"
    );
    // ...and nothing escaped into the bystander's namespace either.
    let (status, secrets) = operator.list_secrets(&other_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        secrets["secrets"].as_array().map(Vec::len),
        Some(0),
        "the traversal did not escape into another user's secret namespace"
    );
}

/// 7. Malformed inputs surface as 4xx (serde/path validation), never a 500.
#[tokio::test]
async fn malformed_inputs_are_4xx_not_500() {
    let harness = build_admin_harness().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    // A malformed `{user_id}` path segment (whitespace/control) → 400 from the
    // handler's `parse_admin_user_id` before the facade is touched.
    let (status, _) = operator
        .send(
            Method::GET,
            "/api/webchat/v2/admin/users/not%20a%20valid%20id",
            None,
        )
        .await;
    assert!(
        status.is_client_error() && status != StatusCode::INTERNAL_SERVER_ERROR,
        "a malformed user_id is a 4xx (got {status}), never a 500"
    );

    // An unknown `role` enum in the body → serde rejection at the Json
    // extractor (4xx), never a 500. Use a real user id so path extraction
    // succeeds and the enum is what fails.
    let (target_id, _) = create_admin(&operator, "Enum Target").await;
    let (status, _) = operator
        .send(
            Method::POST,
            &format!("/api/webchat/v2/admin/users/{target_id}/role"),
            Some(json!({ "role": "superuser" })),
        )
        .await;
    assert!(
        status.is_client_error() && status != StatusCode::INTERNAL_SERVER_ERROR,
        "an invalid role enum is a 4xx (got {status}), never a 500"
    );

    // An unknown `status` enum likewise.
    let (status, _) = operator
        .send(
            Method::POST,
            &format!("/api/webchat/v2/admin/users/{target_id}/status"),
            Some(json!({ "status": "hibernating" })),
        )
        .await;
    assert!(
        status.is_client_error() && status != StatusCode::INTERNAL_SERVER_ERROR,
        "an invalid status enum is a 4xx (got {status}), never a 500"
    );
}

// ─── production-profile admin surface (bucket-2 parity: secret provisioner) ──
//
// The harness above builds a local-dev runtime. These cover the production
// store graph (`local_runtime: None`, `production_runtime: Some`), where the
// admin user service is wired only when BOTH the identity directory (#6395) and
// the admin secret provisioner are sourced from that graph. Gated on `libsql`
// because the production-runtime path requires the libSQL substrate.

#[derive(Debug)]
struct RecordingSandboxTransport;

#[async_trait]
impl ironclaw_host_runtime::SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        _request: ironclaw_host_runtime::CommandExecutionRequest,
    ) -> Result<
        ironclaw_host_runtime::CommandExecutionOutput,
        ironclaw_host_runtime::RuntimeProcessError,
    > {
        Ok(ironclaw_host_runtime::CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: std::time::Duration::ZERO,
        })
    }
}

fn production_effective_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::Deny,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::Standard,
    }
}

#[path = "support/first_party.rs"]
mod first_party_support;

async fn build_admin_harness_production() -> AdminHarness {
    use ironclaw_reborn_composition::{RebornCompositionProfile, RebornRuntimeProcessBinding};

    let root = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(root.path().join("reborn.db"))
            .build()
            .await
            .expect("libsql db"),
    );
    let events = root.path().join("events.db").to_string_lossy().to_string();
    let build_input = RebornHostBindings::libsql(
        RebornCompositionProfile::Production,
        OPERATOR_USER,
        db,
        events,
        None,
        ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
    )
    .with_first_party_bundles(first_party_support::test_first_party_bundles())
    .with_runtime_policy(production_effective_policy())
    .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
        ironclaw_host_runtime::TenantSandboxProcessPort::new(Arc::new(RecordingSandboxTransport)),
    )));
    build_admin_harness_from(root, build_input).await
}

/// Regression for the bucket-2 admin-secret-provisioner parity gap. On a
/// production-shaped runtime the admin user service is `RejectingAdminUserService`
/// unless the identity directory (#6395) AND the admin secret provisioner are
/// both sourced from the production store graph. Before the provisioner
/// production fallback, `reborn_admin_secret_provisioner` returned `None`, so
/// every admin op — including these — returned service-unavailable. Drives the
/// full HTTP admin surface and asserts create-user + per-user secret
/// provisioning work and are isolated across users.
#[tokio::test]
async fn production_admin_surface_provisions_and_isolates_per_user_secrets() {
    let harness = build_admin_harness_production().await;
    let operator = AdminApiDriver::new(harness.router.clone(), OPERATOR_TOKEN);

    // The admin surface is reachable on production (not `RejectingAdminUserService`).
    let (status, users) = operator.list_users().await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin user surface must be wired on production; got {users:?}"
    );

    // create_user needs the identity directory (#6395); its success confirms
    // the directory half of the trio is sourced from the production graph.
    let (status, user_a) = operator
        .create_user(Some("a@prod.test"), "User A", "member")
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "operator creates a user on production"
    );
    let user_a = user_id_of(&user_a);
    let (status, user_b) = operator
        .create_user(Some("b@prod.test"), "User B", "member")
        .await;
    assert_eq!(status, StatusCode::OK);
    let user_b = user_id_of(&user_b);

    // The admin secret provisioner (this change) accepts a per-user secret over
    // the production secret substrate — the direct regression assertion.
    let (status, put) = operator
        .put_secret(&user_a, "openai_key", "sk-prod-secret-value")
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin secret provisioner must be wired on production; got {put:?}"
    );
    assert_eq!(put["secret"]["handle"].as_str(), Some("openai_key"));
    assert!(
        !put.to_string().contains("sk-prod-secret-value"),
        "the secret material must never be echoed back"
    );

    // Per-user isolation: A's secret lists for A and NOT for B.
    let (status, a_secrets) = operator.list_secrets(&user_a).await;
    assert_eq!(status, StatusCode::OK);
    let a_handles: Vec<&str> = a_secrets["secrets"]
        .as_array()
        .expect("secrets list")
        .iter()
        .filter_map(|entry| entry["handle"].as_str())
        .collect();
    assert!(
        a_handles.contains(&"openai_key"),
        "user A sees its provisioned secret"
    );

    let (status, b_secrets) = operator.list_secrets(&user_b).await;
    assert_eq!(status, StatusCode::OK);
    let b_handles: Vec<&str> = b_secrets["secrets"]
        .as_array()
        .expect("secrets list")
        .iter()
        .filter_map(|entry| entry["handle"].as_str())
        .collect();
    assert!(
        !b_handles.contains(&"openai_key"),
        "user B must NOT see user A's secret (per-user provisioner isolation)"
    );
    assert!(
        !b_secrets.to_string().contains("sk-prod-secret-value"),
        "no cross-user secret material leak"
    );
}
