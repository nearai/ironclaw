//! WebUI v2 Slack routable team subject catalog.

use axum::{
    Json, Router,
    extract::{Extension, State},
    routing::get,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::UserId;
use ironclaw_host_api::ingress::{BodyLimitPolicy, IngressRouteDescriptor};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use serde::Serialize;

use super::{
    SlackChannelRouteAdminRouteConfig, SlackRouteError, WEBUI_V2_CHANNELS_SLACK_SUBJECTS_PATH,
    ensure_authorized_operator, route_policy,
};

const SLACK_CHANNEL_SUBJECTS_LIST_ROUTE_ID: &str = "webui.v2.channels.slack.subjects.list";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct SlackRoutableTeamSubject {
    pub(super) subject_user_id: String,
    pub(super) display_name: String,
}

impl SlackRoutableTeamSubject {
    pub(super) fn from_user_id(subject_user_id: UserId) -> Self {
        let display_name = display_name_for_subject_user_id(&subject_user_id);
        Self {
            subject_user_id: subject_user_id.to_string(),
            display_name,
        }
    }
}

pub(super) fn router() -> Router<SlackChannelRouteAdminRouteConfig> {
    Router::new().route(WEBUI_V2_CHANNELS_SLACK_SUBJECTS_PATH, get(list_handler))
}

pub(super) fn descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            SLACK_CHANNEL_SUBJECTS_LIST_ROUTE_ID,
            NetworkMethod::Get,
            WEBUI_V2_CHANNELS_SLACK_SUBJECTS_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("Slack routable team subject list descriptor must validate at startup"), // safety: route id, method, path, and policy are static typed literals.
    ]
}

#[derive(Debug, Serialize)]
struct SlackRoutableTeamSubjectListResponse {
    team_id: String,
    subjects: Vec<SlackRoutableTeamSubject>,
}

async fn list_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<SlackRoutableTeamSubjectListResponse>, SlackRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    Ok(Json(SlackRoutableTeamSubjectListResponse {
        team_id: config.team_id.clone(),
        subjects: config.routable_team_subjects.clone(),
    }))
}

fn display_name_for_subject_user_id(subject_user_id: &UserId) -> String {
    let raw = subject_user_id
        .as_str()
        .strip_prefix("user:")
        .unwrap_or(subject_user_id.as_str());
    let words = raw
        .replace([':', '_', '-'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>();
    if words.is_empty() {
        subject_user_id.to_string()
    } else {
        words.join(" ")
    }
}
