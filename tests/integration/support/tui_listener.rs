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
    RebornReadiness, RebornWebuiBundle, WebuiServeConfig, webui_v2_app,
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
    let authenticator = Arc::new(
        EnvBearerAuthenticator::new(SecretString::from(token.to_string()), user_id)
            .expect("non-empty test token"),
    );
    let bundle = RebornWebuiBundle {
        api,
        product_auth: None,
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
