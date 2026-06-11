//! WebUI v2 Slack installation setup facade.

use axum::{
    Json, Router,
    extract::{Extension, State},
    routing::get,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{BodyLimitPolicy, IngressRouteDescriptor};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use secrecy::SecretString;
use serde::Deserialize;

use crate::slack_setup::{SlackInstallationSetupStatus, SlackInstallationSetupUpdate};

use super::{
    SLACK_CHANNEL_ROUTES_BODY_LIMIT_BYTES, SlackChannelRouteAdminRouteConfig, SlackRouteError,
    WEBUI_V2_CHANNELS_SLACK_SETUP_PATH, ensure_authorized_operator, route_policy,
    scan_route_admin_field,
};

const SLACK_SETUP_GET_ROUTE_ID: &str = "webui.v2.channels.slack.setup.get";
const SLACK_SETUP_SAVE_ROUTE_ID: &str = "webui.v2.channels.slack.setup.save";

pub(super) fn router() -> Router<SlackChannelRouteAdminRouteConfig> {
    Router::new().route(
        WEBUI_V2_CHANNELS_SLACK_SETUP_PATH,
        get(get_handler).put(save_handler),
    )
}

pub(super) fn descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            SLACK_SETUP_GET_ROUTE_ID,
            NetworkMethod::Get,
            WEBUI_V2_CHANNELS_SLACK_SETUP_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("Slack setup get descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
        IngressRouteDescriptor::new(
            SLACK_SETUP_SAVE_ROUTE_ID,
            NetworkMethod::Put,
            WEBUI_V2_CHANNELS_SLACK_SETUP_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: SLACK_CHANNEL_ROUTES_BODY_LIMIT_BYTES,
            }),
        )
        .expect("Slack setup save descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
    ]
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackSetupSaveRequest {
    installation_id: String,
    team_id: String,
    api_app_id: String,
    user_id: Option<String>,
    shared_subject_user_id: Option<String>,
    bot_token: Option<String>,
    signing_secret: Option<String>,
}

impl SlackSetupSaveRequest {
    fn into_update(self) -> SlackInstallationSetupUpdate {
        SlackInstallationSetupUpdate {
            installation_id: self.installation_id,
            team_id: self.team_id,
            api_app_id: self.api_app_id,
            user_id: self.user_id,
            shared_subject_user_id: self.shared_subject_user_id,
            bot_token: self.bot_token.map(SecretString::from),
            signing_secret: self.signing_secret.map(SecretString::from),
        }
    }
}

async fn get_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<SlackInstallationSetupStatus>, SlackRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    Ok(Json(config.setup_service()?.status().await?))
}

async fn save_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SlackSetupSaveRequest>,
) -> Result<Json<SlackInstallationSetupStatus>, SlackRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    scan_route_admin_field(&config, "installation_id", &request.installation_id)?;
    scan_route_admin_field(&config, "team_id", &request.team_id)?;
    scan_route_admin_field(&config, "api_app_id", &request.api_app_id)?;
    if let Some(user_id) = request.user_id.as_deref() {
        scan_route_admin_field(&config, "user_id", user_id)?;
    }
    if let Some(shared_subject_user_id) = request.shared_subject_user_id.as_deref() {
        scan_route_admin_field(&config, "shared_subject_user_id", shared_subject_user_id)?;
    }
    let setup_service = config.setup_service()?;
    setup_service.save(request.into_update()).await?;
    Ok(Json(setup_service.status().await?))
}
