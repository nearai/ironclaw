//! Host-owned admin REST surface for capability **availability** (#5268).
//!
//! Admin-gated (`UserRole::Admin`, via `WebUiAuthenticatedCaller::is_admin`)
//! routes that write **tenant-wide** extension installations to #4544's
//! scoped-lifecycle store — the same store the #5267 availability resolver
//! reads. This is the "admin decides which tools the tenant gets" write path:
//! install a package → every user in the tenant sees its capabilities; don't
//! install → hidden.
//!
//! Scope note: #4544 ownership is `AdminShared` (tenant-wide, admin-writable)
//! or `UserPrivate` (self-owned by the user). The store's `can_be_mutated_by`
//! forbids an admin from writing a *UserPrivate* row for another user, so
//! **per-user** availability ("Bob yes, Carol no") is a `CapabilityPolicyDelta`
//! concern layered on by #5273, not an installation an admin can write here.
//! This surface therefore manages the AdminShared (tenant) set only.
//!
//! Mounted into `webui_v2_app` via
//! [`WebuiServeConfig::with_protected_route_mount`](crate::WebuiServeConfig);
//! the host (CLI `serve`) constructs the store over the same filesystem the
//! resolver reads.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::Utc;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_host_api::{NetworkMethod, TenantId};
use ironclaw_product_workflow::{
    DeleteScopedLifecycleInstallationRequest, LifecyclePackageKind, LifecyclePackageRef,
    ScopedLifecycleActor, ScopedLifecycleInstallation, ScopedLifecycleInstallationId,
    ScopedLifecycleInstallationStore, ScopedLifecycleSubject,
    UpsertScopedLifecycleInstallationRequest, WebUiAuthenticatedCaller,
};
use ironclaw_product_workflow_storage::FilesystemScopedLifecycleInstallationStore;
use serde::{Deserialize, Serialize};

use crate::ProtectedRouteMount;

const ADMIN_EXTENSIONS_PATH: &str = "/api/webchat/v2/admin/extensions";
const ADMIN_EXTENSION_ITEM_PATH: &str = "/api/webchat/v2/admin/extensions/{package_id}";
const ADMIN_EXTENSIONS_LIST_ROUTE_ID: &str = "webui.v2.admin.extensions.list";
const ADMIN_EXTENSIONS_INSTALL_ROUTE_ID: &str = "webui.v2.admin.extensions.install";
const ADMIN_EXTENSIONS_UNINSTALL_ROUTE_ID: &str = "webui.v2.admin.extensions.uninstall";
const ADMIN_EXTENSIONS_BODY_LIMIT_BYTES: NonZeroU64 = NonZeroU64::new(16 * 1024).unwrap(); // safety: 16 KiB is non-zero.
const ADMIN_EXTENSIONS_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.
const ADMIN_EXTENSIONS_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.

/// Host config for the admin capability-availability routes: the trusted
/// tenant plus the scoped-lifecycle store to write tenant installations into.
#[derive(Clone)]
pub struct CapabilityAdminRouteConfig {
    tenant_id: TenantId,
    installations: Arc<dyn ScopedLifecycleInstallationStore>,
}

impl CapabilityAdminRouteConfig {
    pub fn new(
        tenant_id: TenantId,
        installations: Arc<dyn ScopedLifecycleInstallationStore>,
    ) -> Self {
        Self {
            tenant_id,
            installations,
        }
    }
}

/// Build the admin capability-availability routes as a [`ProtectedRouteMount`]
/// (router + descriptors), ready for
/// [`WebuiServeConfig::with_protected_route_mount`](crate::WebuiServeConfig).
pub fn capability_admin_route_mount(config: CapabilityAdminRouteConfig) -> ProtectedRouteMount {
    let router = Router::new()
        .route(ADMIN_EXTENSIONS_PATH, get(list_extensions_handler))
        .route(
            ADMIN_EXTENSION_ITEM_PATH,
            axum::routing::put(install_extension_handler).delete(uninstall_extension_handler),
        )
        .with_state(config);
    ProtectedRouteMount::new(router, capability_admin_descriptors())
}

/// Build the admin capability-availability mount from an already-built runtime,
/// constructing the scoped-lifecycle store over the **same** extension
/// filesystem the #5267 availability resolver reads — so an admin install here
/// is visible to the dispatch resolver. Returns `None` for a runtime with no
/// local substrate (production profiles do not expose this local-dev admin
/// write path).
pub fn build_capability_admin_route_mount(
    runtime: &crate::runtime::RebornRuntime,
    tenant_id: TenantId,
) -> Option<ProtectedRouteMount> {
    let local_runtime = runtime.services().local_runtime.as_ref()?;
    let installations = Arc::new(FilesystemScopedLifecycleInstallationStore::new(Arc::clone(
        &local_runtime.extension_filesystem,
    )
        as Arc<dyn ironclaw_filesystem::RootFilesystem>));
    Some(capability_admin_route_mount(
        CapabilityAdminRouteConfig::new(tenant_id, installations),
    ))
}

/// Ingress descriptors so the descriptor-driven body-limit / rate-limit
/// middleware covers these routes like every other WebChat v2 route.
fn capability_admin_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            ADMIN_EXTENSIONS_LIST_ROUTE_ID,
            NetworkMethod::Get,
            ADMIN_EXTENSIONS_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("admin extensions list descriptor must validate at startup"),
        IngressRouteDescriptor::new(
            ADMIN_EXTENSIONS_INSTALL_ROUTE_ID,
            NetworkMethod::Put,
            ADMIN_EXTENSION_ITEM_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: ADMIN_EXTENSIONS_BODY_LIMIT_BYTES,
            }),
        )
        .expect("admin extensions install descriptor must validate at startup"),
        IngressRouteDescriptor::new(
            ADMIN_EXTENSIONS_UNINSTALL_ROUTE_ID,
            NetworkMethod::Delete,
            ADMIN_EXTENSION_ITEM_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("admin extensions uninstall descriptor must validate at startup"),
    ]
}

fn route_policy(body_limit: BodyLimitPolicy) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: ADMIN_EXTENSIONS_MAX_REQUESTS,
            window_seconds: ADMIN_EXTENSIONS_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("admin extensions policy must validate")
}

/// Admin gate: the caller's tenant must match (404 to avoid tenant
/// enumeration) and the caller must hold an admin role (`is_admin`).
fn ensure_admin(
    config: &CapabilityAdminRouteConfig,
    caller: &WebUiAuthenticatedCaller,
) -> Result<(), CapabilityAdminError> {
    if caller.tenant_id != config.tenant_id {
        return Err(CapabilityAdminError::NotFound);
    }
    if !caller.is_admin() {
        return Err(CapabilityAdminError::Forbidden);
    }
    Ok(())
}

/// Deterministic, validated installation id for the tenant-shared install of a
/// package, so repeated installs replace rather than duplicate.
fn admin_shared_installation_id(
    package: &LifecyclePackageRef,
) -> Result<ScopedLifecycleInstallationId, CapabilityAdminError> {
    let sanitized: String = package
        .id
        .as_str()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    ScopedLifecycleInstallationId::new(format!("admin-shared-{sanitized}"))
        .map_err(|_| CapabilityAdminError::BadRequest)
}

#[derive(Debug, Deserialize, Default)]
struct InstallRequest {
    /// Optional config the capability runs with (stored on the installation).
    #[serde(default)]
    config: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct InstallResponse {
    installation_id: String,
    package_id: String,
    ownership: &'static str,
}

#[derive(Debug, Serialize)]
struct ExtensionSummary {
    package_id: String,
    ownership: &'static str,
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    extensions: Vec<ExtensionSummary>,
}

#[derive(Debug, Serialize)]
struct UninstallResponse {
    uninstalled: bool,
}

async fn install_extension_handler(
    State(config): State<CapabilityAdminRouteConfig>,
    Path(package_id): Path<String>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<InstallRequest>,
) -> Result<Json<InstallResponse>, CapabilityAdminError> {
    ensure_admin(&config, &caller)?;
    let package = LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id.clone())
        .map_err(|_| CapabilityAdminError::BadRequest)?;
    let admin_actor = ScopedLifecycleActor::admin(config.tenant_id.clone(), caller.user_id.clone());
    let installation_id = admin_shared_installation_id(&package)?;
    let mut installation = ScopedLifecycleInstallation::admin_shared(
        installation_id,
        package,
        admin_actor.clone(),
        Utc::now(),
    )
    .map_err(|_| CapabilityAdminError::Forbidden)?;
    installation.config = request.config;
    let response = InstallResponse {
        installation_id: installation.installation_id.as_str().to_string(),
        package_id,
        ownership: installation.ownership.label(),
    };
    config
        .installations
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: admin_actor,
            installation,
        })
        .await?;
    Ok(Json(response))
}

async fn uninstall_extension_handler(
    State(config): State<CapabilityAdminRouteConfig>,
    Path(package_id): Path<String>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<UninstallResponse>, CapabilityAdminError> {
    ensure_admin(&config, &caller)?;
    let package = LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
        .map_err(|_| CapabilityAdminError::BadRequest)?;
    let admin_actor = ScopedLifecycleActor::admin(config.tenant_id.clone(), caller.user_id.clone());
    let installation_id = admin_shared_installation_id(&package)?;
    config
        .installations
        .delete_installation(DeleteScopedLifecycleInstallationRequest {
            actor: admin_actor,
            tenant_id: config.tenant_id.clone(),
            installation_id,
        })
        .await?;
    Ok(Json(UninstallResponse { uninstalled: true }))
}

async fn list_extensions_handler(
    State(config): State<CapabilityAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<ListResponse>, CapabilityAdminError> {
    ensure_admin(&config, &caller)?;
    // The admin's own subject surfaces the tenant-shared set (admin-shared rows
    // are visible to every user in the tenant).
    let subject = ScopedLifecycleSubject::new(config.tenant_id.clone(), caller.user_id.clone());
    let effective = config
        .installations
        .list_effective_installations(subject)
        .await?;
    let extensions = effective
        .installations
        .into_iter()
        .map(|installation| ExtensionSummary {
            package_id: installation.package_ref.id.as_str().to_string(),
            ownership: installation.ownership.label(),
            enabled: installation.enabled,
        })
        .collect();
    Ok(Json(ListResponse { extensions }))
}

/// Sanitized error surface — never leaks store internals to the client.
#[derive(Debug)]
enum CapabilityAdminError {
    BadRequest,
    Forbidden,
    NotFound,
    Unavailable,
}

impl From<ironclaw_product_workflow::ProductWorkflowError> for CapabilityAdminError {
    fn from(_error: ironclaw_product_workflow::ProductWorkflowError) -> Self {
        // The store's typed errors (access denied, invalid request, backend) all
        // map to a sanitized boundary status; the cause is logged store-side.
        Self::Unavailable
    }
}

impl IntoResponse for CapabilityAdminError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        };
        status.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use axum::body::Body;
    use axum::http::Request;
    use ironclaw_host_api::{UserId, UserRole};
    use ironclaw_product_workflow::EffectiveScopedLifecycleInstallations;
    use tower::ServiceExt;

    const TENANT: &str = "tenant:acme";

    /// Minimal in-memory store for caller-level route tests. The store's own
    /// ownership/authorization is covered by #4544; this fake exercises the
    /// route → store round-trip.
    #[derive(Default)]
    struct FakeStore {
        installations: Mutex<Vec<ScopedLifecycleInstallation>>,
    }

    #[async_trait::async_trait]
    impl ScopedLifecycleInstallationStore for FakeStore {
        async fn upsert_installation(
            &self,
            request: UpsertScopedLifecycleInstallationRequest,
        ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
            let mut installations = self.installations.lock().expect("lock");
            installations.retain(|existing| {
                existing.installation_id != request.installation.installation_id
            });
            installations.push(request.installation);
            Ok(())
        }

        async fn get_installation(
            &self,
            _tenant_id: &TenantId,
            installation_id: &ScopedLifecycleInstallationId,
        ) -> Result<
            Option<ScopedLifecycleInstallation>,
            ironclaw_product_workflow::ProductWorkflowError,
        > {
            Ok(self
                .installations
                .lock()
                .expect("lock")
                .iter()
                .find(|existing| &existing.installation_id == installation_id)
                .cloned())
        }

        async fn delete_installation(
            &self,
            request: DeleteScopedLifecycleInstallationRequest,
        ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
            self.installations
                .lock()
                .expect("lock")
                .retain(|existing| existing.installation_id != request.installation_id);
            Ok(())
        }

        async fn list_installations(
            &self,
            _tenant_id: &TenantId,
        ) -> Result<Vec<ScopedLifecycleInstallation>, ironclaw_product_workflow::ProductWorkflowError>
        {
            Ok(self.installations.lock().expect("lock").clone())
        }

        async fn list_effective_installations(
            &self,
            subject: ScopedLifecycleSubject,
        ) -> Result<
            EffectiveScopedLifecycleInstallations,
            ironclaw_product_workflow::ProductWorkflowError,
        > {
            let candidates = self.list_installations(&subject.tenant_id).await?;
            Ok(
                ironclaw_product_workflow::resolve_effective_scoped_lifecycle_installations(
                    subject, candidates,
                ),
            )
        }
    }

    fn mount() -> (ProtectedRouteMount, std::sync::Arc<FakeStore>) {
        let store = std::sync::Arc::new(FakeStore::default());
        let config = CapabilityAdminRouteConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            store.clone() as Arc<dyn ScopedLifecycleInstallationStore>,
        );
        (capability_admin_route_mount(config), store)
    }

    fn request(method: &str, uri: &str, tenant: &str, user: &str, role: UserRole) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .extension(WebUiAuthenticatedCaller {
                tenant_id: TenantId::new(tenant).expect("tenant"),
                user_id: UserId::new(user).expect("user"),
                agent_id: None,
                project_id: None,
                // Deliberately false: admin gating must come from the role, not
                // operator_webui_config (user-token callers have it off).
                operator_webui_config: false,
                role,
            });
        if method == "GET" || method == "DELETE" {
            builder = builder.header("content-length", "0");
        }
        let body = if method == "PUT" { "{}" } else { "" };
        builder.body(Body::from(body)).expect("request builds")
    }

    #[tokio::test]
    async fn admin_installs_and_lists_tenant_extension() {
        let (mount, store) = mount();
        let install = mount
            .router
            .clone()
            .oneshot(request(
                "PUT",
                "/api/webchat/v2/admin/extensions/web-access",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("install responds");
        assert_eq!(install.status(), StatusCode::OK);
        assert_eq!(store.installations.lock().expect("lock").len(), 1);

        let list = mount
            .router
            .oneshot(request(
                "GET",
                "/api/webchat/v2/admin/extensions",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("list responds");
        assert_eq!(list.status(), StatusCode::OK);
        let body = axum::body::to_bytes(list.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(body["extensions"][0]["package_id"], "web-access");
        assert_eq!(body["extensions"][0]["ownership"], "admin_shared");
    }

    #[tokio::test]
    async fn member_is_forbidden_and_other_tenant_is_not_found() {
        let (mount, store) = mount();

        let member = mount
            .router
            .clone()
            .oneshot(request(
                "PUT",
                "/api/webchat/v2/admin/extensions/web-access",
                TENANT,
                "user:bob",
                UserRole::Member,
            ))
            .await
            .expect("member responds");
        assert_eq!(member.status(), StatusCode::FORBIDDEN);

        let other_tenant = mount
            .router
            .oneshot(request(
                "PUT",
                "/api/webchat/v2/admin/extensions/web-access",
                "tenant:other",
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("cross-tenant responds");
        assert_eq!(other_tenant.status(), StatusCode::NOT_FOUND);

        assert!(
            store.installations.lock().expect("lock").is_empty(),
            "neither a forbidden nor a wrong-tenant call may write"
        );
    }

    #[tokio::test]
    async fn admin_uninstall_removes_the_extension() {
        let (mount, store) = mount();
        store
            .upsert_installation(UpsertScopedLifecycleInstallationRequest {
                actor: ScopedLifecycleActor::admin(
                    TenantId::new(TENANT).expect("tenant"),
                    UserId::new("user:director").expect("user"),
                ),
                installation: ScopedLifecycleInstallation::admin_shared(
                    ScopedLifecycleInstallationId::new("admin-shared-web-access").expect("id"),
                    LifecyclePackageRef::new(LifecyclePackageKind::Extension, "web-access")
                        .expect("package"),
                    ScopedLifecycleActor::admin(
                        TenantId::new(TENANT).expect("tenant"),
                        UserId::new("user:director").expect("user"),
                    ),
                    Utc::now(),
                )
                .expect("admin-shared install"),
            })
            .await
            .expect("seed");

        let response = mount
            .router
            .oneshot(request(
                "DELETE",
                "/api/webchat/v2/admin/extensions/web-access",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("uninstall responds");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(store.installations.lock().expect("lock").is_empty());
    }
}
