//! Host-owned admin REST surface for **per-user capability policy deltas**
//! (#5268).
//!
//! Admin-gated (`UserRole::Admin`, via `WebUiAuthenticatedCaller::is_admin`)
//! routes that write per-user [`CapabilityPolicyDelta`] rows at
//! [`PolicyScope::User`] into the **same** durable
//! [`CapabilityPolicyDeltaStore`] the dispatch `PolicyResolver`
//! (`capability_policy_resolver`, #5261 D3) reads. This is the "admin decides
//! what *this user* can do" write path layered on top of the tenant-wide
//! availability surface owned by `capability_admin_routes.rs`.
//!
//! One delta row per `(user, capability)` carries all four optional policy
//! dimensions (availability / identity / approval / config_patch); an absent
//! field inherits the layer above. PUT upserts that single row, DELETE revokes
//! it (idempotent), GET lists the user's effective deltas.
//!
//! Mounted into `webui_v2_app` via
//! [`WebuiServeConfig::with_protected_route_mount`](crate::WebuiServeConfig);
//! the host (CLI `serve`) reaches the shared delta store through the built
//! runtime so an admin grant here is immediately visible to enforcement and
//! durable across restart.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::StatusCode,
    http::request::Parts,
    response::{IntoResponse, Response},
    routing::get,
};
use ironclaw_capability_policy::{
    Availability, CapabilityPolicyDelta, CapabilityPolicyDeltaStore, IdentityMode, PolicyError,
    PolicyScope, PolicySubject,
};
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_host_api::{CapabilityId, NetworkMethod, PermissionMode, TenantId, UserId};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use serde::{Deserialize, Serialize};

use crate::ProtectedRouteMount;

const ADMIN_USER_CAPS_LIST_PATH: &str = "/api/webchat/v2/admin/users/{user_id}/capabilities";
const ADMIN_USER_CAP_ITEM_PATH: &str =
    "/api/webchat/v2/admin/users/{user_id}/capabilities/{capability_id}";
const ADMIN_USER_CAPS_SET_ROUTE_ID: &str = "webui.v2.admin.users.capabilities.set";
const ADMIN_USER_CAPS_DELETE_ROUTE_ID: &str = "webui.v2.admin.users.capabilities.delete";
const ADMIN_USER_CAPS_LIST_ROUTE_ID: &str = "webui.v2.admin.users.capabilities.list";
const ADMIN_USER_CAPS_BODY_LIMIT_BYTES: NonZeroU64 = NonZeroU64::new(16 * 1024).unwrap(); // safety: 16 KiB is non-zero.
const ADMIN_USER_CAPS_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.
const ADMIN_USER_CAPS_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.

/// Host config for the per-user capability-policy routes: the trusted tenant
/// plus the SHARED delta store the dispatch resolver also reads.
#[derive(Clone)]
pub struct CapabilityUserPolicyRouteConfig {
    tenant_id: TenantId,
    deltas: Arc<dyn CapabilityPolicyDeltaStore>,
}

impl CapabilityUserPolicyRouteConfig {
    pub fn new(tenant_id: TenantId, deltas: Arc<dyn CapabilityPolicyDeltaStore>) -> Self {
        Self { tenant_id, deltas }
    }
}

/// Build the per-user capability-policy routes as a [`ProtectedRouteMount`]
/// (router + descriptors), ready for
/// [`WebuiServeConfig::with_protected_route_mount`](crate::WebuiServeConfig).
pub fn capability_user_policy_route_mount(
    config: CapabilityUserPolicyRouteConfig,
) -> ProtectedRouteMount {
    let router = Router::new()
        .route(
            ADMIN_USER_CAPS_LIST_PATH,
            get(list_user_capabilities_handler),
        )
        .route(
            ADMIN_USER_CAP_ITEM_PATH,
            axum::routing::put(set_user_capability_handler).delete(delete_user_capability_handler),
        )
        .with_state(config);
    ProtectedRouteMount::new(router, capability_user_policy_descriptors())
}

/// Build the per-user capability-policy mount from an already-built runtime,
/// reusing the **same** [`CapabilityPolicyDeltaStore`] `Arc` the dispatch
/// `PolicyResolver` reads (`local_runtime.capability_policy_delta_store`, #5261
/// D3) — so an admin grant here is immediately visible to enforcement and
/// durable across restart. Returns `None` when the runtime has no local
/// substrate or the capability policy is not activated (the shared handle is
/// `None`); never constructs a second store.
pub fn build_capability_user_policy_route_mount(
    runtime: &crate::runtime::RebornRuntime,
    tenant_id: TenantId,
) -> Option<ProtectedRouteMount> {
    let local_runtime = runtime.services().local_runtime.as_ref()?;
    let deltas = local_runtime.capability_policy_delta_store.clone()?;
    Some(capability_user_policy_route_mount(
        CapabilityUserPolicyRouteConfig::new(tenant_id, deltas),
    ))
}

/// Ingress descriptors so the descriptor-driven body-limit / rate-limit
/// middleware covers these routes like every other WebChat v2 route.
fn capability_user_policy_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            ADMIN_USER_CAPS_LIST_ROUTE_ID,
            NetworkMethod::Get,
            ADMIN_USER_CAPS_LIST_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("admin user capabilities list descriptor must validate at startup"),
        IngressRouteDescriptor::new(
            ADMIN_USER_CAPS_SET_ROUTE_ID,
            NetworkMethod::Put,
            ADMIN_USER_CAP_ITEM_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: ADMIN_USER_CAPS_BODY_LIMIT_BYTES,
            }),
        )
        .expect("admin user capabilities set descriptor must validate at startup"),
        IngressRouteDescriptor::new(
            ADMIN_USER_CAPS_DELETE_ROUTE_ID,
            NetworkMethod::Delete,
            ADMIN_USER_CAP_ITEM_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("admin user capabilities delete descriptor must validate at startup"),
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
            max_requests: ADMIN_USER_CAPS_MAX_REQUESTS,
            window_seconds: ADMIN_USER_CAPS_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("admin user capabilities policy must validate")
}

/// Admin gate extractor (admin-rest-1): runs the tenant-match (404 to avoid
/// tenant enumeration) + admin-role (`is_admin`, 403) check as a
/// [`FromRequestParts`] extractor. Declaring it BEFORE a `Json` body extractor
/// in a handler signature means the admin gate runs before the body is parsed,
/// so a non-admin caller cannot probe JSON-parse behaviour (no body-parse
/// oracle). The caller identity is host-supplied authority (the
/// `WebUiAuthenticatedCaller` extension inserted by the bearer-auth layer), not
/// browser input.
struct AdminCaller(#[allow(dead_code)] WebUiAuthenticatedCaller);

impl FromRequestParts<CapabilityUserPolicyRouteConfig> for AdminCaller {
    type Rejection = CapabilityUserPolicyError;

    async fn from_request_parts(
        parts: &mut Parts,
        config: &CapabilityUserPolicyRouteConfig,
    ) -> Result<Self, Self::Rejection> {
        let caller = parts
            .extensions
            .get::<WebUiAuthenticatedCaller>()
            .cloned()
            // The bearer-auth layer always inserts this extension before any
            // protected route runs; its absence is a composition fault, not a
            // client error. Fail closed (403) rather than leak the distinction.
            .ok_or(CapabilityUserPolicyError::Forbidden)?;
        if caller.tenant_id != config.tenant_id {
            return Err(CapabilityUserPolicyError::NotFound);
        }
        if !caller.is_admin() {
            return Err(CapabilityUserPolicyError::Forbidden);
        }
        Ok(Self(caller))
    }
}

/// PUT body: the four optional policy dimensions written into one delta row.
/// All snake_case; an absent field inherits the layer above.
#[derive(Debug, Deserialize, Default)]
struct SetUserCapabilityPolicyRequest {
    #[serde(default)]
    availability: Option<Availability>,
    #[serde(default)]
    identity: Option<IdentityMode>,
    #[serde(default)]
    approval: Option<PermissionMode>,
    #[serde(default)]
    config_patch: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct UserCapabilityPolicySummary {
    capability_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    availability: Option<Availability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<IdentityMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<PermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_patch: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ListUserCapabilityPoliciesResponse {
    capabilities: Vec<UserCapabilityPolicySummary>,
}

fn summary_from_delta(delta: CapabilityPolicyDelta) -> UserCapabilityPolicySummary {
    UserCapabilityPolicySummary {
        capability_id: delta.capability.as_str().to_string(),
        availability: delta.availability,
        identity: delta.identity,
        approval: delta.approval,
        config_patch: delta.config_patch,
    }
}

async fn set_user_capability_handler(
    State(config): State<CapabilityUserPolicyRouteConfig>,
    Path((user_id, capability_id)): Path<(String, String)>,
    // `AdminCaller` is declared BEFORE `Json` so the admin gate (tenant-match +
    // is_admin) runs before the body is parsed (admin-rest-1: no body-parse
    // oracle for a non-admin caller).
    _admin: AdminCaller,
    Json(request): Json<SetUserCapabilityPolicyRequest>,
) -> Result<Json<UserCapabilityPolicySummary>, CapabilityUserPolicyError> {
    let user_id = UserId::new(&user_id).map_err(bad_request_from_id_parse)?;
    let capability = CapabilityId::new(&capability_id).map_err(bad_request_from_id_parse)?;
    let delta = CapabilityPolicyDelta {
        scope: PolicyScope::User { user_id },
        capability,
        availability: request.availability,
        identity: request.identity,
        approval: request.approval,
        config_patch: request.config_patch,
    };
    let summary = summary_from_delta(delta.clone());
    config.deltas.upsert_delta(&config.tenant_id, delta).await?;
    Ok(Json(summary))
}

async fn delete_user_capability_handler(
    State(config): State<CapabilityUserPolicyRouteConfig>,
    Path((user_id, capability_id)): Path<(String, String)>,
    _admin: AdminCaller,
) -> Result<StatusCode, CapabilityUserPolicyError> {
    let user_id = UserId::new(&user_id).map_err(bad_request_from_id_parse)?;
    let capability = CapabilityId::new(&capability_id).map_err(bad_request_from_id_parse)?;
    config
        .deltas
        .delete_delta(
            &config.tenant_id,
            &PolicyScope::User { user_id },
            &capability,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_user_capabilities_handler(
    State(config): State<CapabilityUserPolicyRouteConfig>,
    Path(user_id): Path<String>,
    _admin: AdminCaller,
) -> Result<Json<ListUserCapabilityPoliciesResponse>, CapabilityUserPolicyError> {
    let user_id = UserId::new(&user_id).map_err(bad_request_from_id_parse)?;
    let subject = PolicySubject {
        tenant_id: config.tenant_id.clone(),
        user_id: user_id.clone(),
    };
    // admin-rest-2: `list_subject_deltas` returns BOTH the user's User-scope
    // deltas AND the tenant-scope deltas that apply to the subject. This is the
    // per-user admin surface (PUT/DELETE write only `PolicyScope::User`), so a
    // tenant-scope delta must NOT leak here — it belongs to the tenant-wide
    // availability surface (`capability_admin_routes.rs`). Filter to the rows
    // whose scope is exactly this user before mapping to summaries.
    let capabilities = config
        .deltas
        .list_subject_deltas(&subject)
        .await?
        .into_iter()
        .filter(|delta| matches!(&delta.scope, PolicyScope::User { user_id: scoped } if *scoped == user_id))
        .map(summary_from_delta)
        .collect();
    Ok(Json(ListUserCapabilityPoliciesResponse { capabilities }))
}

/// Map a `UserId`/`CapabilityId` parse failure to a sanitized `BadRequest`,
/// logging the bound `HostApiError` at debug first (admin-rest-3: never drop
/// the cause — mirrors the `From<PolicyError>` discipline below). `debug!` only,
/// so handler paths never corrupt the REPL/TUI surface.
fn bad_request_from_id_parse(error: ironclaw_host_api::HostApiError) -> CapabilityUserPolicyError {
    tracing::debug!(%error, "rejecting admin capability-policy request: invalid id in path");
    CapabilityUserPolicyError::BadRequest
}

/// Sanitized error surface — never leaks store internals to the client.
#[derive(Debug)]
enum CapabilityUserPolicyError {
    BadRequest,
    Forbidden,
    NotFound,
    Unavailable,
}

impl From<PolicyError> for CapabilityUserPolicyError {
    fn from(error: PolicyError) -> Self {
        // Log the typed cause before sanitizing (error-handling.md: never drop
        // the cause). `debug!` only — handler paths must not corrupt the REPL.
        tracing::debug!(%error, "capability user policy delta store operation failed");
        Self::Unavailable
    }
}

impl IntoResponse for CapabilityUserPolicyError {
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

    use axum::body::Body;
    use axum::http::Request;
    use ironclaw_capability_policy::InMemoryCapabilityPolicyDeltaStore;
    use ironclaw_host_api::UserRole;
    use tower::ServiceExt;

    const TENANT: &str = "tenant:acme";

    fn mount() -> (ProtectedRouteMount, Arc<InMemoryCapabilityPolicyDeltaStore>) {
        let store = Arc::new(InMemoryCapabilityPolicyDeltaStore::new());
        let config = CapabilityUserPolicyRouteConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            store.clone() as Arc<dyn CapabilityPolicyDeltaStore>,
        );
        (capability_user_policy_route_mount(config), store)
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
                // Deliberately false: admin gating comes from the role, not
                // operator_webui_config (user-token callers have it off).
                operator_webui_config: false,
                role,
            });
        if method == "GET" || method == "DELETE" {
            builder = builder.header("content-length", "0");
        }
        let body = if method == "PUT" {
            "{\"availability\":\"hidden\",\"approval\":\"deny\"}"
        } else {
            ""
        };
        builder.body(Body::from(body)).expect("request builds")
    }

    #[tokio::test]
    async fn admin_sets_and_lists_user_capability_delta() {
        let (mount, _store) = mount();
        let set = mount
            .router
            .clone()
            .oneshot(request(
                "PUT",
                "/api/webchat/v2/admin/users/user:bob/capabilities/web.fetch",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("set responds");
        assert_eq!(set.status(), StatusCode::OK);

        let list = mount
            .router
            .oneshot(request(
                "GET",
                "/api/webchat/v2/admin/users/user:bob/capabilities",
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
        assert_eq!(body["capabilities"][0]["capability_id"], "web.fetch");
        assert_eq!(body["capabilities"][0]["availability"], "hidden");
        assert_eq!(body["capabilities"][0]["approval"], "deny");
    }

    #[tokio::test]
    async fn member_is_forbidden_and_other_tenant_is_not_found() {
        let (mount, store) = mount();

        let member = mount
            .router
            .clone()
            .oneshot(request(
                "PUT",
                "/api/webchat/v2/admin/users/user:bob/capabilities/web.fetch",
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
                "/api/webchat/v2/admin/users/user:bob/capabilities/web.fetch",
                "tenant:other",
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("cross-tenant responds");
        assert_eq!(other_tenant.status(), StatusCode::NOT_FOUND);

        let subject = PolicySubject {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            user_id: UserId::new("user:bob").expect("user"),
        };
        assert!(
            store
                .list_subject_deltas(&subject)
                .await
                .expect("list")
                .is_empty(),
            "neither a forbidden nor a wrong-tenant call may write"
        );
    }

    #[tokio::test]
    async fn admin_delete_is_idempotent() {
        let (mount, _store) = mount();
        // Delete a delta that was never written: still 204 (idempotent revoke).
        let response = mount
            .router
            .clone()
            .oneshot(request(
                "DELETE",
                "/api/webchat/v2/admin/users/user:bob/capabilities/web.fetch",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("delete responds");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Set then delete: also 204, and the row is gone.
        let set = mount
            .router
            .clone()
            .oneshot(request(
                "PUT",
                "/api/webchat/v2/admin/users/user:bob/capabilities/web.fetch",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("set responds");
        assert_eq!(set.status(), StatusCode::OK);

        let delete = mount
            .router
            .oneshot(request(
                "DELETE",
                "/api/webchat/v2/admin/users/user:bob/capabilities/web.fetch",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("delete responds");
        assert_eq!(delete.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn list_returns_user_scope_deltas_only_not_tenant_scope() {
        // admin-rest-2: `list_subject_deltas` returns a subject's User-scope rows
        // AND the tenant-wide rows. This per-user surface writes only
        // `PolicyScope::User`, so the GET must NOT leak the tenant-scope row.
        let (mount, store) = mount();
        let tenant = TenantId::new(TENANT).expect("tenant");

        // A tenant-wide delta (belongs to the availability surface, not here).
        store
            .upsert_delta(
                &tenant,
                CapabilityPolicyDelta {
                    scope: PolicyScope::Tenant,
                    capability: CapabilityId::new("tenant.cap").expect("cap"),
                    availability: Some(Availability::Hidden),
                    identity: None,
                    approval: None,
                    config_patch: None,
                },
            )
            .await
            .expect("seed tenant delta");
        // A user-scope delta for user:bob (this IS the per-user surface).
        store
            .upsert_delta(
                &tenant,
                CapabilityPolicyDelta {
                    scope: PolicyScope::User {
                        user_id: UserId::new("user:bob").expect("user"),
                    },
                    capability: CapabilityId::new("user.cap").expect("cap"),
                    availability: Some(Availability::Hidden),
                    identity: None,
                    approval: None,
                    config_patch: None,
                },
            )
            .await
            .expect("seed user delta");

        let list = mount
            .router
            .oneshot(request(
                "GET",
                "/api/webchat/v2/admin/users/user:bob/capabilities",
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
        let ids: Vec<&str> = body["capabilities"]
            .as_array()
            .expect("capabilities array")
            .iter()
            .map(|cap| cap["capability_id"].as_str().expect("capability_id"))
            .collect();
        assert!(
            ids.contains(&"user.cap"),
            "the per-user delta must be returned, got {ids:?}"
        );
        assert!(
            !ids.contains(&"tenant.cap"),
            "the tenant-scope delta must NOT leak through the per-user surface, got {ids:?}"
        );
    }
}
