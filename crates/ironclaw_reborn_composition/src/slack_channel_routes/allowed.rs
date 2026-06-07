//! WebUI v2 Slack allowed-channel admin facade.

use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{Extension, State},
    routing::get,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{BodyLimitPolicy, IngressRouteDescriptor};
use serde::{Deserialize, Serialize};

use super::{
    MAX_LIST_LIMIT, SLACK_CHANNEL_ROUTES_BODY_LIMIT_BYTES, SlackChannelRoute,
    SlackChannelRouteAdminRouteConfig, SlackRouteError, WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH,
    ensure_authorized_operator, route_policy, scan_route_admin_field,
};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;

const SLACK_CHANNEL_ALLOWED_LIST_ROUTE_ID: &str = "webui.v2.channels.slack.allowed.list";
const SLACK_CHANNEL_ALLOWED_SAVE_ROUTE_ID: &str = "webui.v2.channels.slack.allowed.save";
const MAX_ALLOWED_CHANNELS: usize = 500;
const LIST_ALL_CURSOR_GUARD: usize = 10_000;

pub(super) fn router() -> Router<SlackChannelRouteAdminRouteConfig> {
    Router::new().route(
        WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH,
        get(list_handler).put(save_handler),
    )
}

pub(super) fn descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            SLACK_CHANNEL_ALLOWED_LIST_ROUTE_ID,
            NetworkMethod::Get,
            WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("Slack allowed channel list descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
        IngressRouteDescriptor::new(
            SLACK_CHANNEL_ALLOWED_SAVE_ROUTE_ID,
            NetworkMethod::Put,
            WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: SLACK_CHANNEL_ROUTES_BODY_LIMIT_BYTES,
            }),
        )
        .expect("Slack allowed channel save descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SlackAllowedChannel {
    channel_id: String,
    subject_user_id: String,
}

#[derive(Debug, Serialize)]
struct SlackAllowedChannelListResponse {
    team_id: String,
    channels: Vec<SlackAllowedChannel>,
}

#[derive(Debug, Deserialize)]
struct SlackAllowedChannelSaveRequest {
    channel_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SlackAllowedChannelSaveResponse {
    success: bool,
    team_id: String,
    channels: Vec<SlackAllowedChannel>,
}

struct SlackAllowedChannelAdmin<'a> {
    config: &'a SlackChannelRouteAdminRouteConfig,
}

impl<'a> SlackAllowedChannelAdmin<'a> {
    fn new(config: &'a SlackChannelRouteAdminRouteConfig) -> Self {
        Self { config }
    }

    async fn list(&self) -> Result<Vec<SlackAllowedChannel>, SlackRouteError> {
        let routes = self.list_all_routes().await?;
        Ok(allowed_channels_from_managed_routes(self.config, routes))
    }

    async fn replace(
        &self,
        channel_ids: Vec<String>,
    ) -> Result<Vec<SlackAllowedChannel>, SlackRouteError> {
        let channel_ids = self.normalize_channel_ids(channel_ids)?;
        let assignments = channel_ids
            .into_iter()
            .map(|channel_id| {
                self.config
                    .channel_subject_assigner
                    .assignment_for(channel_id)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let routes = self
            .config
            .store
            .replace_managed_routes(
                &self.config.tenant_id,
                &self.config.installation_id,
                &self.config.team_id,
                self.config.channel_subject_assigner.owner().clone(),
                assignments,
            )
            .await?;
        Ok(routes.into_iter().map(SlackAllowedChannel::from).collect())
    }

    async fn list_all_routes(&self) -> Result<Vec<SlackChannelRoute>, SlackRouteError> {
        let mut cursor = 0;
        let mut routes = Vec::new();
        loop {
            let page = self
                .config
                .store
                .list_routes(
                    &self.config.tenant_id,
                    &self.config.installation_id,
                    &self.config.team_id,
                    cursor,
                    MAX_LIST_LIMIT,
                )
                .await?;
            routes.extend(page.routes);
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            if next_cursor <= cursor || next_cursor > LIST_ALL_CURSOR_GUARD {
                return Err(SlackRouteError::Unavailable);
            }
            cursor = next_cursor;
        }
        Ok(routes)
    }

    fn normalize_channel_ids(
        &self,
        channel_ids: Vec<String>,
    ) -> Result<Vec<String>, SlackRouteError> {
        if channel_ids.len() > MAX_ALLOWED_CHANNELS {
            return Err(SlackRouteError::BadRequest);
        }
        let mut normalized = BTreeSet::new();
        for channel_id in channel_ids {
            let channel_id = channel_id.trim().to_string();
            if channel_id.is_empty() {
                return Err(SlackRouteError::BadRequest);
            }
            scan_route_admin_field(self.config, "channel_id", &channel_id)?;
            self.config.key_for_channel(channel_id.clone())?;
            normalized.insert(channel_id);
        }
        Ok(normalized.into_iter().collect())
    }
}

async fn list_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<SlackAllowedChannelListResponse>, SlackRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    let admin = SlackAllowedChannelAdmin::new(&config);
    Ok(Json(SlackAllowedChannelListResponse {
        team_id: config.team_id.clone(),
        channels: admin.list().await?,
    }))
}

async fn save_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SlackAllowedChannelSaveRequest>,
) -> Result<Json<SlackAllowedChannelSaveResponse>, SlackRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    let admin = SlackAllowedChannelAdmin::new(&config);
    Ok(Json(SlackAllowedChannelSaveResponse {
        success: true,
        team_id: config.team_id.clone(),
        channels: admin.replace(request.channel_ids).await?,
    }))
}

fn allowed_channels_from_managed_routes(
    config: &SlackChannelRouteAdminRouteConfig,
    routes: Vec<SlackChannelRoute>,
) -> Vec<SlackAllowedChannel> {
    let mut channels = routes
        .into_iter()
        .filter(|route| config.channel_subject_assigner.owns_route(route))
        .map(SlackAllowedChannel::from)
        .collect::<Vec<_>>();
    channels.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
    channels
}

impl From<SlackChannelRoute> for SlackAllowedChannel {
    fn from(route: SlackChannelRoute) -> Self {
        Self {
            channel_id: route.channel_id,
            subject_user_id: route.subject_user_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_product_adapters::AdapterInstallationId;
    use tower::ServiceExt;

    use super::*;
    use crate::slack_channel_routes::{
        DEFAULT_LIST_LIMIT, InMemorySlackChannelRouteStore, SlackChannelRouteError,
        SlackChannelRouteKey, SlackChannelRouteListPage, SlackChannelRouteStore,
        slack_channel_route_admin_route_mount,
    };

    const TENANT: &str = "tenant:slack-routes";
    const INSTALLATION: &str = "install_slack_routes";
    const TEAM: &str = "T0ROUTES";

    #[tokio::test]
    async fn allowed_channel_admin_saves_replaces_and_lists_channel_routes() {
        let store = Arc::new(InMemorySlackChannelRouteStore::new());
        let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

        let save_response = mount
            .protected
            .clone()
            .oneshot(request(
                "PUT",
                r#"{"channel_ids":["C0OPS"," C0ENG ","C0OPS"]}"#,
                TENANT,
            ))
            .await
            .expect("save responds");
        assert_eq!(save_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(save_response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(body["success"], true);
        assert_eq!(body["team_id"], TEAM);
        assert_eq!(body["channels"].as_array().expect("channels").len(), 2);
        assert_eq!(body["channels"][0]["channel_id"], "C0ENG");
        assert_eq!(body["channels"][1]["channel_id"], "C0OPS");
        assert_ne!(
            body["channels"][0]["subject_user_id"], body["channels"][1]["subject_user_id"],
            "each managed Slack channel must get its own tenant-scoped subject"
        );

        let routes = store
            .list_routes(
                &TenantId::new(TENANT).expect("tenant"),
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                TEAM,
                0,
                DEFAULT_LIST_LIMIT,
            )
            .await
            .expect("routes list");
        assert_eq!(routes.routes.len(), 2);
        assert_ne!(
            routes.routes[0].subject_user_id, routes.routes[1].subject_user_id,
            "persisted routes should keep per-channel subjects"
        );

        let replace_response = mount
            .protected
            .clone()
            .oneshot(request("PUT", r#"{"channel_ids":["C0OPS"]}"#, TENANT))
            .await
            .expect("replace responds");
        assert_eq!(replace_response.status(), StatusCode::OK);
        let replace_body = axum::body::to_bytes(replace_response.into_body(), 64 * 1024)
            .await
            .expect("replace body");
        let replace_body: serde_json::Value =
            serde_json::from_slice(&replace_body).expect("replace json");

        let list_response = mount
            .protected
            .oneshot(request("GET", "", TENANT))
            .await
            .expect("list responds");
        assert_eq!(list_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(list_response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            body["channels"],
            serde_json::json!([
                {
                    "channel_id": "C0OPS",
                    "subject_user_id": replace_body["channels"][0]["subject_user_id"].clone()
                }
            ])
        );
    }

    #[tokio::test]
    async fn allowed_channel_admin_rejects_invalid_channel_without_mutating_store() {
        let store = Arc::new(InMemorySlackChannelRouteStore::new());
        let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

        let response = mount
            .protected
            .oneshot(request("PUT", r#"{"channel_ids":["C0ENG",""]}"#, TENANT))
            .await
            .expect("save responds");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            store
                .list_routes(
                    &TenantId::new(TENANT).expect("tenant"),
                    &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                    TEAM,
                    0,
                    DEFAULT_LIST_LIMIT,
                )
                .await
                .expect("routes list")
                .routes
                .is_empty()
        );
    }

    #[tokio::test]
    async fn allowed_channel_admin_rejects_cross_tenant_and_non_operator_callers() {
        let store = Arc::new(InMemorySlackChannelRouteStore::new());
        let mount = slack_channel_route_admin_route_mount(route_config(store));

        for (method, body, tenant_id, user_id, expected_status) in [
            (
                "GET",
                "",
                "tenant:other",
                "user:admin",
                StatusCode::NOT_FOUND,
            ),
            (
                "PUT",
                r#"{"channel_ids":["C0ENG"]}"#,
                "tenant:other",
                "user:admin",
                StatusCode::NOT_FOUND,
            ),
            ("GET", "", TENANT, "user:not-admin", StatusCode::FORBIDDEN),
            (
                "PUT",
                r#"{"channel_ids":["C0ENG"]}"#,
                TENANT,
                "user:not-admin",
                StatusCode::FORBIDDEN,
            ),
        ] {
            let response = mount
                .protected
                .clone()
                .oneshot(request_for_caller(method, body, tenant_id, user_id))
                .await
                .expect("route responds");
            assert_eq!(response.status(), expected_status, "{method} {user_id}");
        }
    }

    #[tokio::test]
    async fn allowed_channel_admin_returns_503_when_store_unavailable() {
        let mount = slack_channel_route_admin_route_mount(route_config(Arc::new(
            UnavailableAllowedRouteStore,
        )));

        for (method, body) in [("GET", ""), ("PUT", r#"{"channel_ids":["C0ENG"]}"#)] {
            let response = mount
                .protected
                .clone()
                .oneshot(request(method, body, TENANT))
                .await
                .expect("route responds");
            assert_eq!(
                response.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{method}"
            );
        }
    }

    #[tokio::test]
    async fn allowed_channel_admin_empty_save_clears_all_channel_routes() {
        let store = Arc::new(InMemorySlackChannelRouteStore::new());
        let mount = slack_channel_route_admin_route_mount(route_config(store.clone()));

        let seed = mount
            .protected
            .clone()
            .oneshot(request(
                "PUT",
                r#"{"channel_ids":["C0OPS","C0ENG"]}"#,
                TENANT,
            ))
            .await
            .expect("seed responds");
        assert_eq!(seed.status(), StatusCode::OK);

        let clear = mount
            .protected
            .oneshot(request("PUT", r#"{"channel_ids":[]}"#, TENANT))
            .await
            .expect("clear responds");
        assert_eq!(clear.status(), StatusCode::OK);
        let body = axum::body::to_bytes(clear.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(body["channels"], serde_json::json!([]));
        assert!(
            store
                .list_routes(
                    &TenantId::new(TENANT).expect("tenant"),
                    &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                    TEAM,
                    0,
                    DEFAULT_LIST_LIMIT,
                )
                .await
                .expect("routes list")
                .routes
                .is_empty()
        );
    }

    fn route_config(store: Arc<dyn SlackChannelRouteStore>) -> SlackChannelRouteAdminRouteConfig {
        SlackChannelRouteAdminRouteConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            TEAM.to_string(),
            UserId::new("user:admin").expect("operator user"),
            store,
        )
        .with_allowed_subject_user_ids([UserId::new("user:eng-team-agent").expect("subject user")])
    }

    fn request(method: &str, body: &str, tenant_id: &str) -> Request<Body> {
        request_for_caller(method, body, tenant_id, "user:admin")
    }

    fn request_for_caller(
        method: &str,
        body: &str,
        tenant_id: &str,
        user_id: &str,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
            .header("content-type", "application/json")
            .extension(WebUiAuthenticatedCaller {
                tenant_id: TenantId::new(tenant_id).expect("tenant"),
                user_id: UserId::new(user_id).expect("user"),
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

    #[derive(Debug)]
    struct UnavailableAllowedRouteStore;

    #[async_trait::async_trait]
    impl SlackChannelRouteStore for UnavailableAllowedRouteStore {
        async fn list_routes(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &AdapterInstallationId,
            _team_id: &str,
            _cursor: usize,
            _limit: usize,
        ) -> Result<SlackChannelRouteListPage, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn upsert_route(
            &self,
            _key: SlackChannelRouteKey,
            _subject_user_id: UserId,
        ) -> Result<SlackChannelRoute, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn delete_route(
            &self,
            _key: &SlackChannelRouteKey,
        ) -> Result<bool, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn replace_managed_routes(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &AdapterInstallationId,
            _team_id: &str,
            _owner: crate::slack_channel_routes::SlackChannelRouteSubjectOwner,
            _assignments: Vec<crate::slack_channel_routes::SlackChannelRouteAssignment>,
        ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn resolve_subject_user_id(
            &self,
            _key: &SlackChannelRouteKey,
        ) -> Result<Option<UserId>, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }
    }
}
