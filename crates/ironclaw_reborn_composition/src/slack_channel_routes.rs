//! Host-owned Slack shared-channel route store and WebUI admin surface.

use std::collections::HashMap;
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_host_api::{NetworkMethod, TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    ProductConversationRouteKey, ProductConversationSubjectRouteResolutionRequest,
    ProductConversationSubjectRouteResolver, ProductWorkflowError, WebUiAuthenticatedCaller,
};
use ironclaw_slack_v2_adapter::SLACK_V2_ADAPTER_ID;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH: &str = "/api/webchat/v2/channels/slack/routes";

const SLACK_CHANNEL_ROUTES_LIST_ROUTE_ID: &str = "webui.v2.channels.slack.routes.list";
const SLACK_CHANNEL_ROUTES_UPSERT_ROUTE_ID: &str = "webui.v2.channels.slack.routes.upsert";
const SLACK_CHANNEL_ROUTES_DELETE_ROUTE_ID: &str = "webui.v2.channels.slack.routes.delete";
const SLACK_CHANNEL_ROUTES_BODY_LIMIT_BYTES: NonZeroU64 = NonZeroU64::new(16 * 1024).unwrap(); // safety: 16 KiB is non-zero.
const SLACK_CHANNEL_ROUTES_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.
const SLACK_CHANNEL_ROUTES_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlackChannelRouteKey {
    pub tenant_id: TenantId,
    pub installation_id: AdapterInstallationId,
    pub team_id: String,
    pub channel_id: String,
}

impl SlackChannelRouteKey {
    pub fn new(
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
        team_id: String,
        channel_id: String,
    ) -> Result<Self, SlackChannelRouteError> {
        ProductConversationRouteKey::new(Some(team_id.clone()), channel_id.clone())
            .map_err(|_| SlackChannelRouteError::InvalidRoute)?;
        Ok(Self {
            tenant_id,
            installation_id,
            team_id,
            channel_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SlackChannelRoute {
    pub tenant_id: String,
    pub installation_id: String,
    pub team_id: String,
    pub channel_id: String,
    pub subject_user_id: String,
}

impl SlackChannelRoute {
    pub(crate) fn new(key: SlackChannelRouteKey, subject_user_id: UserId) -> Self {
        Self {
            tenant_id: key.tenant_id.to_string(),
            installation_id: key.installation_id.to_string(),
            team_id: key.team_id,
            channel_id: key.channel_id,
            subject_user_id: subject_user_id.to_string(),
        }
    }
}

#[async_trait]
pub trait SlackChannelRouteStore: Send + Sync + std::fmt::Debug {
    async fn list_routes(
        &self,
        tenant_id: &TenantId,
        installation_id: &AdapterInstallationId,
    ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError>;

    async fn upsert_route(
        &self,
        key: SlackChannelRouteKey,
        subject_user_id: UserId,
    ) -> Result<SlackChannelRoute, SlackChannelRouteError>;

    async fn delete_route(
        &self,
        key: &SlackChannelRouteKey,
    ) -> Result<bool, SlackChannelRouteError>;

    async fn resolve_subject_user_id(
        &self,
        key: &SlackChannelRouteKey,
    ) -> Result<Option<UserId>, SlackChannelRouteError>;
}

#[derive(Debug, Default)]
pub struct InMemorySlackChannelRouteStore {
    routes: RwLock<HashMap<SlackChannelRouteKey, UserId>>,
}

impl InMemorySlackChannelRouteStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed_route(
        &self,
        key: SlackChannelRouteKey,
        subject_user_id: UserId,
    ) -> Result<(), SlackChannelRouteError> {
        self.routes
            .write()
            .map_err(|_| SlackChannelRouteError::StoreUnavailable)?
            .insert(key, subject_user_id);
        Ok(())
    }
}

#[async_trait]
impl SlackChannelRouteStore for InMemorySlackChannelRouteStore {
    async fn list_routes(
        &self,
        tenant_id: &TenantId,
        installation_id: &AdapterInstallationId,
    ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError> {
        let routes = self
            .routes
            .read()
            .map_err(|_| SlackChannelRouteError::StoreUnavailable)?;
        let mut result = routes
            .iter()
            .filter(|(key, _)| {
                &key.tenant_id == tenant_id && &key.installation_id == installation_id
            })
            .map(|(key, subject_user_id)| {
                SlackChannelRoute::new(key.clone(), subject_user_id.clone())
            })
            .collect::<Vec<_>>();
        result.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
        Ok(result)
    }

    async fn upsert_route(
        &self,
        key: SlackChannelRouteKey,
        subject_user_id: UserId,
    ) -> Result<SlackChannelRoute, SlackChannelRouteError> {
        self.routes
            .write()
            .map_err(|_| SlackChannelRouteError::StoreUnavailable)?
            .insert(key.clone(), subject_user_id.clone());
        Ok(SlackChannelRoute::new(key, subject_user_id))
    }

    async fn delete_route(
        &self,
        key: &SlackChannelRouteKey,
    ) -> Result<bool, SlackChannelRouteError> {
        Ok(self
            .routes
            .write()
            .map_err(|_| SlackChannelRouteError::StoreUnavailable)?
            .remove(key)
            .is_some())
    }

    async fn resolve_subject_user_id(
        &self,
        key: &SlackChannelRouteKey,
    ) -> Result<Option<UserId>, SlackChannelRouteError> {
        Ok(self
            .routes
            .read()
            .map_err(|_| SlackChannelRouteError::StoreUnavailable)?
            .get(key)
            .cloned())
    }
}

#[derive(Debug, Clone)]
pub struct SlackChannelRouteSubjectResolver {
    tenant_id: TenantId,
    installation_id: AdapterInstallationId,
    store: Arc<dyn SlackChannelRouteStore>,
}

impl SlackChannelRouteSubjectResolver {
    pub fn new(
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
        store: Arc<dyn SlackChannelRouteStore>,
    ) -> Self {
        Self {
            tenant_id,
            installation_id,
            store,
        }
    }
}

#[async_trait]
impl ProductConversationSubjectRouteResolver for SlackChannelRouteSubjectResolver {
    async fn resolve_product_conversation_subject_route(
        &self,
        request: ProductConversationSubjectRouteResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if request.adapter_id.as_str() != SLACK_V2_ADAPTER_ID
            || request.installation_id != self.installation_id
        {
            return Ok(None);
        }
        let Some(team_id) = request.route_key.space_id() else {
            return Ok(None);
        };
        let key = SlackChannelRouteKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            team_id.to_string(),
            request.route_key.conversation_id().to_string(),
        )
        .map_err(map_route_error_to_workflow)?;
        self.store
            .resolve_subject_user_id(&key)
            .await
            .map_err(map_route_error_to_workflow)
    }
}

fn map_route_error_to_workflow(error: SlackChannelRouteError) -> ProductWorkflowError {
    match error {
        SlackChannelRouteError::InvalidRoute => ProductWorkflowError::InvalidBindingRequest {
            reason: "invalid Slack channel route".into(),
        },
        SlackChannelRouteError::StoreUnavailable => ProductWorkflowError::Transient {
            reason: "Slack channel route store unavailable".into(),
        },
    }
}

#[derive(Debug, Error)]
pub enum SlackChannelRouteError {
    #[error("invalid Slack channel route")]
    InvalidRoute,
    #[error("Slack channel route store unavailable")]
    StoreUnavailable,
}

#[derive(Clone)]
pub struct SlackChannelRouteAdminRouteConfig {
    tenant_id: TenantId,
    installation_id: AdapterInstallationId,
    team_id: String,
    store: Arc<dyn SlackChannelRouteStore>,
}

impl SlackChannelRouteAdminRouteConfig {
    pub fn new(
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
        team_id: String,
        store: Arc<dyn SlackChannelRouteStore>,
    ) -> Self {
        Self {
            tenant_id,
            installation_id,
            team_id,
            store,
        }
    }

    fn key_for_channel(&self, channel_id: String) -> Result<SlackChannelRouteKey, SlackRouteError> {
        SlackChannelRouteKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.clone(),
            channel_id,
        )
        .map_err(|_| SlackRouteError::BadRequest)
    }
}

pub(crate) struct SlackChannelRouteAdminRouteMount {
    pub(crate) protected: Router,
    pub(crate) descriptors: Vec<IngressRouteDescriptor>,
}

pub(crate) fn slack_channel_route_admin_route_mount(
    config: SlackChannelRouteAdminRouteConfig,
) -> SlackChannelRouteAdminRouteMount {
    SlackChannelRouteAdminRouteMount {
        protected: Router::new()
            .route(
                WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
                get(list_slack_channel_routes_handler)
                    .put(upsert_slack_channel_route_handler)
                    .delete(delete_slack_channel_route_handler),
            )
            .with_state(config),
        descriptors: slack_channel_route_admin_descriptors(),
    }
}

pub(crate) fn slack_channel_route_admin_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            SLACK_CHANNEL_ROUTES_LIST_ROUTE_ID,
            NetworkMethod::Get,
            WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("Slack channel route list descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
        IngressRouteDescriptor::new(
            SLACK_CHANNEL_ROUTES_UPSERT_ROUTE_ID,
            NetworkMethod::Put,
            WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: SLACK_CHANNEL_ROUTES_BODY_LIMIT_BYTES,
            }),
        )
        .expect("Slack channel route upsert descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
        IngressRouteDescriptor::new(
            SLACK_CHANNEL_ROUTES_DELETE_ROUTE_ID,
            NetworkMethod::Delete,
            WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: SLACK_CHANNEL_ROUTES_BODY_LIMIT_BYTES,
            }),
        )
        .expect("Slack channel route delete descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
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
            max_requests: SLACK_CHANNEL_ROUTES_MAX_REQUESTS,
            window_seconds: SLACK_CHANNEL_ROUTES_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("Slack channel route admin policy must validate") // safety: policy fields are typed static literals with non-zero limits.
}

#[derive(Debug, Serialize)]
struct SlackChannelRouteListResponse {
    routes: Vec<SlackChannelRoute>,
}

#[derive(Debug, Deserialize)]
struct SlackChannelRouteUpsertRequest {
    channel_id: String,
    subject_user_id: String,
}

#[derive(Debug, Deserialize)]
struct SlackChannelRouteDeleteRequest {
    channel_id: String,
}

#[derive(Debug, Serialize)]
struct SlackChannelRouteDeleteResponse {
    deleted: bool,
}

async fn list_slack_channel_routes_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<SlackChannelRouteListResponse>, SlackRouteError> {
    ensure_tenant_matches(&config, &caller)?;
    let routes = config
        .store
        .list_routes(&config.tenant_id, &config.installation_id)
        .await?;
    Ok(Json(SlackChannelRouteListResponse { routes }))
}

async fn upsert_slack_channel_route_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SlackChannelRouteUpsertRequest>,
) -> Result<Json<SlackChannelRoute>, SlackRouteError> {
    ensure_tenant_matches(&config, &caller)?;
    let subject_user_id =
        UserId::new(request.subject_user_id).map_err(|_| SlackRouteError::BadRequest)?;
    let key = config.key_for_channel(request.channel_id)?;
    let route = config.store.upsert_route(key, subject_user_id).await?;
    Ok(Json(route))
}

async fn delete_slack_channel_route_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SlackChannelRouteDeleteRequest>,
) -> Result<Json<SlackChannelRouteDeleteResponse>, SlackRouteError> {
    ensure_tenant_matches(&config, &caller)?;
    let key = config.key_for_channel(request.channel_id)?;
    let deleted = config.store.delete_route(&key).await?;
    Ok(Json(SlackChannelRouteDeleteResponse { deleted }))
}

fn ensure_tenant_matches(
    config: &SlackChannelRouteAdminRouteConfig,
    caller: &WebUiAuthenticatedCaller,
) -> Result<(), SlackRouteError> {
    if caller.tenant_id == config.tenant_id {
        Ok(())
    } else {
        Err(SlackRouteError::NotFound)
    }
}

#[derive(Debug)]
enum SlackRouteError {
    BadRequest,
    NotFound,
    Unavailable,
}

impl From<SlackChannelRouteError> for SlackRouteError {
    fn from(error: SlackChannelRouteError) -> Self {
        match error {
            SlackChannelRouteError::InvalidRoute => Self::BadRequest,
            SlackChannelRouteError::StoreUnavailable => Self::Unavailable,
        }
    }
}

impl IntoResponse for SlackRouteError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, "Invalid Slack channel route."),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "Slack channel route configuration not found.",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Slack channel route service is unavailable.",
            ),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    const TENANT: &str = "tenant:slack-routes";
    const INSTALLATION: &str = "install_slack_routes";
    const TEAM: &str = "T0ROUTES";

    #[tokio::test]
    async fn route_admin_upserts_lists_and_deletes_server_scoped_channel_route() {
        let store = Arc::new(InMemorySlackChannelRouteStore::new());
        let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

        let upsert_response = mount
            .protected
            .clone()
            .oneshot(request(
                "PUT",
                r#"{"channel_id":"C0ENG","subject_user_id":"user:eng-team-agent"}"#,
                TENANT,
            ))
            .await
            .expect("upsert responds");
        assert_eq!(upsert_response.status(), StatusCode::OK);

        let routes = store
            .list_routes(
                &TenantId::new(TENANT).expect("tenant"),
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
            )
            .await
            .expect("routes list");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].team_id, TEAM);
        assert_eq!(routes[0].channel_id, "C0ENG");
        assert_eq!(routes[0].subject_user_id, "user:eng-team-agent");

        let list_response = mount
            .protected
            .clone()
            .oneshot(request("GET", "", TENANT))
            .await
            .expect("list responds");
        assert_eq!(list_response.status(), StatusCode::OK);

        let delete_response = mount
            .protected
            .oneshot(request("DELETE", r#"{"channel_id":"C0ENG"}"#, TENANT))
            .await
            .expect("delete responds");
        assert_eq!(delete_response.status(), StatusCode::OK);
        assert!(
            store
                .list_routes(
                    &TenantId::new(TENANT).expect("tenant"),
                    &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                )
                .await
                .expect("routes list")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn route_admin_rejects_cross_tenant_callers() {
        let mount = slack_channel_route_admin_route_mount(route_config(Arc::new(
            InMemorySlackChannelRouteStore::new(),
        )));

        let response = mount
            .protected
            .oneshot(request(
                "PUT",
                r#"{"channel_id":"C0ENG","subject_user_id":"user:eng-team-agent"}"#,
                "tenant:other",
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    fn route_config(store: Arc<dyn SlackChannelRouteStore>) -> SlackChannelRouteAdminRouteConfig {
        SlackChannelRouteAdminRouteConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            TEAM.to_string(),
            store,
        )
    }

    fn request(method: &str, body: &str, tenant_id: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
            .header("content-type", "application/json")
            .extension(WebUiAuthenticatedCaller {
                tenant_id: TenantId::new(tenant_id).expect("tenant"),
                user_id: UserId::new("user:admin").expect("user"),
                agent_id: None,
                project_id: None,
            });
        if method == "GET" {
            builder = builder.header("content-length", "0");
        }
        builder
            .body(Body::from(body.to_string()))
            .expect("request builds")
    }
}
