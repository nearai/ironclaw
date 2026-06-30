//! WebUI v2 Slack allowed-channel admin facade.

use std::collections::{BTreeMap, BTreeSet};

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
    SlackChannelRouteAdminContext, SlackChannelRouteAdminRouteConfig, SlackChannelRouteAssignment,
    SlackRouteError, WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH, ensure_allowed_subject_user,
    ensure_authorized_operator, route_policy, scan_route_admin_field, subjects,
};
use ironclaw_host_api::UserId;
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
    subject_display_name: String,
}

#[derive(Debug, Serialize)]
struct SlackAllowedChannelListResponse {
    team_id: String,
    channels: Vec<SlackAllowedChannel>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackAllowedChannelSaveRequest {
    channel_ids: Option<Vec<String>>,
    channels: Option<Vec<SlackAllowedChannelSaveAssignment>>,
}

enum SlackAllowedChannelSaveSelection {
    Managed(Vec<String>),
    Explicit(Vec<SlackAllowedChannelSaveAssignment>),
}

impl SlackAllowedChannelSaveRequest {
    fn into_selection(self) -> Result<SlackAllowedChannelSaveSelection, SlackRouteError> {
        match (self.channel_ids, self.channels) {
            (Some(channel_ids), None) => Ok(SlackAllowedChannelSaveSelection::Managed(channel_ids)),
            (None, Some(channels)) => Ok(SlackAllowedChannelSaveSelection::Explicit(channels)),
            _ => Err(SlackRouteError::BadRequest),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SlackAllowedChannelSaveAssignment {
    channel_id: String,
    subject_user_id: Option<String>,
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

    async fn list(
        &self,
        context: &SlackChannelRouteAdminContext,
    ) -> Result<Vec<SlackAllowedChannel>, SlackRouteError> {
        let routes = self.list_all_routes(context).await?;
        Ok(allowed_channels_from_routes(routes))
    }

    async fn replace(
        &self,
        context: &SlackChannelRouteAdminContext,
        request: SlackAllowedChannelSaveRequest,
    ) -> Result<Vec<SlackAllowedChannel>, SlackRouteError> {
        let assignments = match request.into_selection()? {
            SlackAllowedChannelSaveSelection::Managed(channel_ids) => {
                self.managed_assignments(context, channel_ids)?
            }
            SlackAllowedChannelSaveSelection::Explicit(channels) => {
                let current_subjects_by_channel = self.current_subjects_by_channel(context).await?;
                self.explicit_assignments(context, channels, &current_subjects_by_channel)?
            }
        };
        let routes = self
            .config
            .store
            .replace_managed_routes(
                &context.tenant_id,
                &context.installation_id,
                &context.team_id,
                assignments,
            )
            .await?;
        Ok(routes.into_iter().map(SlackAllowedChannel::from).collect())
    }

    fn managed_assignments(
        &self,
        context: &SlackChannelRouteAdminContext,
        channel_ids: Vec<String>,
    ) -> Result<Vec<SlackChannelRouteAssignment>, SlackRouteError> {
        let channel_ids = self.normalize_channel_ids(context, channel_ids)?;
        channel_ids
            .into_iter()
            .map(|channel_id| {
                context
                    .channel_subject_assigner
                    .assignment_for(channel_id)
                    .map_err(Into::into)
            })
            .collect()
    }

    fn explicit_assignments(
        &self,
        context: &SlackChannelRouteAdminContext,
        channels: Vec<SlackAllowedChannelSaveAssignment>,
        current_subjects_by_channel: &BTreeMap<String, UserId>,
    ) -> Result<Vec<SlackChannelRouteAssignment>, SlackRouteError> {
        if channels.len() > MAX_ALLOWED_CHANNELS {
            return Err(SlackRouteError::BadRequest);
        }
        let mut assignments = BTreeMap::new();
        for channel in channels {
            let channel_id = channel.channel_id.trim().to_string();
            if channel_id.is_empty() {
                return Err(SlackRouteError::BadRequest);
            }
            scan_route_admin_field(self.config, "channel_id", &channel_id)?;
            self.config
                .key_for_channel_in_context(context, channel_id.clone())?;

            let subject_user_id = match channel
                .subject_user_id
                .as_deref()
                .map(str::trim)
                .filter(|subject_user_id| !subject_user_id.is_empty())
            {
                Some(subject_user_id) => {
                    scan_route_admin_field(self.config, "subject_user_id", subject_user_id)?;
                    let subject_user_id = UserId::new(subject_user_id.to_string())
                        .map_err(|_| SlackRouteError::BadRequest)?;
                    ensure_selected_subject_user(
                        context,
                        current_subjects_by_channel,
                        &channel_id,
                        &subject_user_id,
                    )?;
                    subject_user_id
                }
                None => {
                    context
                        .channel_subject_assigner
                        .assignment_for(channel_id.clone())?
                        .subject_user_id
                }
            };

            if assignments
                .insert(channel_id.clone(), subject_user_id.clone())
                .is_some()
            {
                return Err(SlackRouteError::BadRequest);
            }
        }
        Ok(assignments
            .into_iter()
            .map(|(channel_id, subject_user_id)| {
                SlackChannelRouteAssignment::new(channel_id, subject_user_id)
            })
            .collect())
    }

    async fn current_subjects_by_channel(
        &self,
        context: &SlackChannelRouteAdminContext,
    ) -> Result<BTreeMap<String, UserId>, SlackRouteError> {
        self.list_all_routes(context)
            .await?
            .into_iter()
            .map(|route| {
                let subject_user_id =
                    UserId::new(route.subject_user_id).map_err(|_| SlackRouteError::Unavailable)?;
                Ok((route.channel_id, subject_user_id))
            })
            .collect()
    }

    async fn list_all_routes(
        &self,
        context: &SlackChannelRouteAdminContext,
    ) -> Result<Vec<SlackChannelRoute>, SlackRouteError> {
        let mut cursor = 0;
        let mut routes = Vec::new();
        loop {
            let page = self
                .config
                .store
                .list_routes(
                    &context.tenant_id,
                    &context.installation_id,
                    &context.team_id,
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
        context: &SlackChannelRouteAdminContext,
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
            self.config
                .key_for_channel_in_context(context, channel_id.clone())?;
            normalized.insert(channel_id);
        }
        Ok(normalized.into_iter().collect())
    }
}

fn ensure_selected_subject_user(
    context: &SlackChannelRouteAdminContext,
    current_subjects_by_channel: &BTreeMap<String, UserId>,
    channel_id: &str,
    subject_user_id: &UserId,
) -> Result<(), SlackRouteError> {
    if ensure_allowed_subject_user(context, subject_user_id).is_ok() {
        return Ok(());
    }
    if context.preserve_existing_subjects
        && current_subjects_by_channel
            .get(channel_id)
            .is_some_and(|current_subject_user_id| current_subject_user_id == subject_user_id)
    {
        return Ok(());
    }
    let managed_assignment = context
        .channel_subject_assigner
        .assignment_for(channel_id.to_string())?;
    if managed_assignment.subject_user_id == *subject_user_id {
        return Ok(());
    }
    Err(SlackRouteError::Forbidden)
}

async fn list_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<SlackAllowedChannelListResponse>, SlackRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    let admin = SlackAllowedChannelAdmin::new(&config);
    let context = config.route_context().await?;
    Ok(Json(SlackAllowedChannelListResponse {
        team_id: context.team_id.clone(),
        channels: admin.list(&context).await?,
    }))
}

async fn save_handler(
    State(config): State<SlackChannelRouteAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SlackAllowedChannelSaveRequest>,
) -> Result<Json<SlackAllowedChannelSaveResponse>, SlackRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    let admin = SlackAllowedChannelAdmin::new(&config);
    let context = config.route_context().await?;
    Ok(Json(SlackAllowedChannelSaveResponse {
        success: true,
        team_id: context.team_id.clone(),
        channels: admin.replace(&context, request).await?,
    }))
}

fn allowed_channels_from_routes(routes: Vec<SlackChannelRoute>) -> Vec<SlackAllowedChannel> {
    let mut channels = routes
        .into_iter()
        .map(SlackAllowedChannel::from)
        .collect::<Vec<_>>();
    channels.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
    channels
}

impl From<SlackChannelRoute> for SlackAllowedChannel {
    fn from(route: SlackChannelRoute) -> Self {
        Self {
            channel_id: route.channel_id,
            subject_display_name: subjects::display_name_for_subject_user_id(
                &route.subject_user_id,
            ),
            subject_user_id: route.subject_user_id,
        }
    }
}

#[cfg(test)]
mod tests;
