//! Binds the REAL `webui_v2_app` composition (bearer-auth layer included)
//! on an ephemeral loopback port so the Reborn TUI's `ApiClient` (real
//! reqwest HTTP + SSE) can be driven against it end-to-end.
//!
//! Distinct from `support/webui_mount.rs::mount_webui_v2_router`, which
//! drives the bare router via `tower::ServiceExt::oneshot` with the
//! authenticated caller injected directly as an `Extension` — no listener,
//! no bearer-auth middleware. `ironclaw_reborn_tui::client::ApiClient` makes
//! real HTTP requests with an `Authorization` header, so there is nothing to
//! `oneshot` against; this needs a bound TCP listener running the full
//! `webui_v2_app` stack. Precedent for the bind-and-serve shape:
//! `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs::spawn_serve`.

use std::net::SocketAddr;
use std::sync::Arc;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_workflow::RebornServicesApi;
use ironclaw_reborn_composition::{
    RebornProductAuthServices, RebornReadiness, RebornWebuiBundle, WebuiServeConfig, webui_v2_app,
};
use ironclaw_reborn_webui_ingress::EnvBearerAuthenticator;
use secrecy::SecretString;

/// Aborts the background `axum::serve` task on drop so a test's listener
/// doesn't outlive the test.
pub struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Binds the full `webui_v2_app` router (bearer auth + CORS + body limit +
/// security headers + the v2 route set) over `api` on `127.0.0.1:0`, stamping
/// `tenant_id`/`agent_id`/`project_id` as the trusted host-installation
/// identity (mirrors `support/webui_mount.rs::webui_caller_for`'s
/// construction, which the auth middleware itself replicates at
/// `crates/ironclaw_reborn_composition/src/webui/webui_serve.rs`'s
/// `authenticate_request`) and accepting exactly `token` for `user_id`.
///
/// Returns the bound address and a drop guard that aborts the serve task.
pub async fn spawn_webui_v2(
    api: Arc<dyn RebornServicesApi>,
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    token: &str,
) -> (SocketAddr, AbortOnDrop) {
    spawn_webui_v2_inner(api, tenant_id, user_id, agent_id, project_id, token, None).await
}

/// Like [`spawn_webui_v2`], but additionally mounts the real product-auth
/// route set (`/api/reborn/product-auth/manual-token/submit` and friends) by
/// passing a real `Arc<RebornProductAuthServices>` into the bundle —
/// `spawn_webui_v2` always mounts `product_auth: None`, so those routes
/// 404 there. Callers proving the manual-token credential-2-step flow
/// (submit -> resolve `CredentialProvided`) need this variant with the SAME
/// `RebornProductAuthServices` instance the capability harness's credential
/// resolver reads from (`HostRuntimeCapabilityHarness::reborn_services_for_test()
/// .product_auth`, a public field on `ironclaw_reborn_composition::RebornServices`)
/// — otherwise the HTTP-submitted token lands in a disconnected store the
/// gated capability's re-dispatch can never see.
pub async fn spawn_webui_v2_with_product_auth(
    api: Arc<dyn RebornServicesApi>,
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    token: &str,
    product_auth: Arc<RebornProductAuthServices>,
) -> (SocketAddr, AbortOnDrop) {
    spawn_webui_v2_inner(
        api,
        tenant_id,
        user_id,
        agent_id,
        project_id,
        token,
        Some(product_auth),
    )
    .await
}

async fn spawn_webui_v2_inner(
    api: Arc<dyn RebornServicesApi>,
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    token: &str,
    product_auth: Option<Arc<RebornProductAuthServices>>,
) -> (SocketAddr, AbortOnDrop) {
    let authenticator = Arc::new(
        EnvBearerAuthenticator::new(SecretString::from(token.to_string()), user_id)
            .expect("non-empty test token"),
    );
    let bundle = RebornWebuiBundle {
        api,
        product_auth,
        readiness: RebornReadiness::default(),
    };
    let mut config =
        WebuiServeConfig::new(tenant_id, authenticator, Vec::new()).with_default_agent_id(agent_id);
    if let Some(project_id) = project_id {
        config = config.with_default_project_id(project_id);
    }
    let app = webui_v2_app(bundle, config).expect("compose webui_v2_app");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move {
        #[allow(clippy::let_underscore_must_use)]
        let _ = axum::serve(listener, app).await;
    });
    (addr, AbortOnDrop(handle))
}

/// Creates a thread with an explicit id via a real HTTP POST against
/// `/api/webchat/v2/threads` on the listener `spawn_webui_v2` bound —
/// production's `RebornServices::create_thread` handler, same bearer auth,
/// same `EnsureThreadRequest` path a real create does.
///
/// This exists because `ironclaw_reborn_tui::client::ApiClient::create_thread()`
/// doesn't expose `WebUiCreateThreadRequest::requested_thread_id` (real TUI
/// usage never needs to pick a specific id — see `crates/ironclaw_reborn_tui/src/lib.rs`,
/// which calls `create_thread()` plain during startup when the account has no
/// threads yet). `requested_thread_id` is nonetheless a genuine, already-shipped
/// wire field: `RebornServices::create_thread`'s doc comment
/// (`crates/ironclaw_product_workflow/src/reborn_services.rs`) says it "makes
/// the caller's choice authoritative." A caller-pinned id is what lets a test
/// align the thread this creates with a `TurnScope` it already knows about
/// (e.g. one a scripted-reply gateway was registered against before this
/// listener existed) — the same (tenant, agent, project, owner_user_id) tuple
/// the bearer authenticates as, plus a chosen `thread_id`.
///
/// Panics on any non-2xx response — test-only plumbing, not production code.
pub async fn create_thread_pinned(addr: SocketAddr, token: &str, thread_id: &str) {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/api/webchat/v2/threads"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "client_action_id": uuid::Uuid::new_v4().to_string(),
            "requested_thread_id": thread_id,
        }))
        .send()
        .await
        .expect("create_thread_pinned request");
    let status = response.status();
    assert!(
        status.is_success(),
        "create_thread_pinned({thread_id}) failed: {status} {}",
        response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string())
    );
}
