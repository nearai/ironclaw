//! Slack outbound target authority for default delivery.
//!
//! Core outbound preferences only see opaque target ids and validated reply
//! target bindings. Slack-specific channel and DM authority stays here.

#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
#[cfg(test)]
use std::sync::RwLock;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef, ExternalConversationRef};
#[cfg(test)]
use ironclaw_product_adapters::{EgressCredentialHandle, ProtocolHttpEgress};
use ironclaw_product_workflow::{
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetSummary, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind, WebUiAuthenticatedCaller,
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};
use ironclaw_turns::ReplyTargetBindingRef;
use thiserror::Error;

use crate::outbound_preferences::{OutboundDeliveryTargetEntry, OutboundDeliveryTargetProvider};
use crate::slack_channel_routes::{
    SlackChannelRouteError, SlackChannelRouteKey, SlackChannelRouteStore,
};
#[cfg(test)]
use crate::slack_dm_open::{SlackDmOpenError, open_slack_dm_channel};
use crate::slack_serve::{SlackTeamId, SlackUserId};

pub(crate) const SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackConfiguredChannelRoute {
    pub(crate) channel_id: String,
    pub(crate) subject_user_id: UserId,
}

impl SlackConfiguredChannelRoute {
    pub(crate) fn new(channel_id: String, subject_user_id: UserId) -> Self {
        Self {
            channel_id,
            subject_user_id,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SlackOutboundTargetProviderConfig {
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) installation_id: AdapterInstallationId,
    pub(crate) team_id: SlackTeamId,
    pub(crate) configured_channel_routes: Vec<SlackConfiguredChannelRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SlackPersonalDmTargetKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) installation_id: AdapterInstallationId,
    pub(crate) team_id: String,
    pub(crate) user_id: UserId,
}

impl SlackPersonalDmTargetKey {
    pub(crate) fn new(
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
        team_id: String,
        user_id: UserId,
    ) -> Result<Self, SlackPersonalDmTargetError> {
        validate_slack_id("slack team", &team_id)?;
        Ok(Self {
            tenant_id,
            installation_id,
            team_id,
            user_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackPersonalDmTarget {
    pub(crate) key: SlackPersonalDmTargetKey,
    pub(crate) slack_user_id: SlackUserId,
    pub(crate) dm_channel_id: String,
}

impl SlackPersonalDmTarget {
    pub(crate) fn new(
        key: SlackPersonalDmTargetKey,
        slack_user_id: SlackUserId,
        dm_channel_id: String,
    ) -> Result<Self, SlackPersonalDmTargetError> {
        validate_slack_id("slack user", slack_user_id.as_str())?;
        validate_slack_dm_channel_id(&dm_channel_id)?;
        Ok(Self {
            key,
            slack_user_id,
            dm_channel_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum SlackPersonalDmTargetError {
    #[error("invalid Slack personal DM target")]
    InvalidTarget,
    #[error("Slack personal DM target store unavailable")]
    StoreUnavailable,
    #[error("Slack personal DM provisioning failed: {0}")]
    #[allow(dead_code)]
    ProvisioningFailed(String),
}

#[async_trait::async_trait]
pub(crate) trait SlackPersonalDmTargetStore: Send + Sync + std::fmt::Debug {
    async fn load_personal_dm_target(
        &self,
        key: &SlackPersonalDmTargetKey,
    ) -> Result<Option<SlackPersonalDmTarget>, SlackPersonalDmTargetError>;

    #[allow(dead_code)]
    async fn upsert_personal_dm_target(
        &self,
        target: SlackPersonalDmTarget,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError>;
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct InMemorySlackPersonalDmTargetStore {
    targets: RwLock<HashMap<SlackPersonalDmTargetKey, SlackPersonalDmTarget>>,
}

#[cfg(test)]
impl InMemorySlackPersonalDmTargetStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl SlackPersonalDmTargetStore for InMemorySlackPersonalDmTargetStore {
    async fn load_personal_dm_target(
        &self,
        key: &SlackPersonalDmTargetKey,
    ) -> Result<Option<SlackPersonalDmTarget>, SlackPersonalDmTargetError> {
        Ok(self
            .targets
            .read()
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?
            .get(key)
            .cloned())
    }

    async fn upsert_personal_dm_target(
        &self,
        target: SlackPersonalDmTarget,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        self.targets
            .write()
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?
            .insert(target.key.clone(), target.clone());
        Ok(target)
    }
}

#[cfg(test)]
pub(crate) struct SlackPersonalDmTargetProvisioner {
    tenant_id: TenantId,
    installation_id: AdapterInstallationId,
    team_id: SlackTeamId,
    egress: Arc<dyn ProtocolHttpEgress>,
    credential_handle: EgressCredentialHandle,
    store: Arc<dyn SlackPersonalDmTargetStore>,
}

#[cfg(test)]
impl std::fmt::Debug for SlackPersonalDmTargetProvisioner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackPersonalDmTargetProvisioner")
            .field("tenant_id", &self.tenant_id)
            .field("installation_id", &self.installation_id)
            .field("team_id", &self.team_id)
            .field("egress", &"Arc<dyn ProtocolHttpEgress>")
            .field("credential_handle", &self.credential_handle)
            .field("store", &self.store)
            .finish()
    }
}

#[cfg(test)]
impl SlackPersonalDmTargetProvisioner {
    pub(crate) fn new(
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
        team_id: SlackTeamId,
        egress: Arc<dyn ProtocolHttpEgress>,
        credential_handle: EgressCredentialHandle,
        store: Arc<dyn SlackPersonalDmTargetStore>,
    ) -> Self {
        Self {
            tenant_id,
            installation_id,
            team_id,
            egress,
            credential_handle,
            store,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn provision_for_user(
        &self,
        user_id: UserId,
        slack_user_id: SlackUserId,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        let key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.as_str().to_string(),
            user_id,
        )?;
        let dm_channel_id = self.open_dm_channel(slack_user_id.as_str()).await?;
        let target = SlackPersonalDmTarget::new(key, slack_user_id, dm_channel_id)?;
        self.store.upsert_personal_dm_target(target).await
    }

    #[allow(dead_code)]
    async fn open_dm_channel(
        &self,
        slack_user_id: &str,
    ) -> Result<String, SlackPersonalDmTargetError> {
        let channel_id = open_slack_dm_channel(
            self.egress.as_ref(),
            self.credential_handle.clone(),
            slack_user_id,
        )
        .await
        .map_err(|error| match error {
            SlackDmOpenError::MissingChannel => SlackPersonalDmTargetError::InvalidTarget,
            SlackDmOpenError::Backend(reason) => {
                SlackPersonalDmTargetError::ProvisioningFailed(reason)
            }
        })?;
        validate_slack_dm_channel_id(&channel_id)?;
        Ok(channel_id)
    }
}

#[derive(Debug)]
pub(crate) struct SlackHostBetaOutboundTargetProvider {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    installation_id: AdapterInstallationId,
    team_id: SlackTeamId,
    shared_target_id_prefix: String,
    personal_target_id_prefix: String,
    configured_channel_routes: Vec<SlackConfiguredChannelRoute>,
    channel_route_store: Arc<dyn SlackChannelRouteStore>,
    personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
}

impl SlackHostBetaOutboundTargetProvider {
    pub(crate) fn new(
        config: SlackOutboundTargetProviderConfig,
        channel_route_store: Arc<dyn SlackChannelRouteStore>,
        personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
    ) -> Self {
        Self {
            tenant_id: config.tenant_id,
            agent_id: config.agent_id,
            project_id: config.project_id,
            installation_id: config.installation_id,
            shared_target_id_prefix: format!("slack:shared-channel:{}:", config.team_id.as_str()),
            personal_target_id_prefix: format!("slack:personal-dm:{}:", config.team_id.as_str()),
            team_id: config.team_id,
            configured_channel_routes: config.configured_channel_routes,
            channel_route_store,
            personal_dm_target_store,
        }
    }

    fn target_id_for_shared_channel(
        &self,
        channel_id: &str,
    ) -> Result<RebornOutboundDeliveryTargetId, RebornServicesError> {
        RebornOutboundDeliveryTargetId::new(format!(
            "slack:shared-channel:{}:{}",
            self.team_id.as_str(),
            channel_id
        ))
        .map_err(|_| slack_target_backend_error())
    }

    fn target_id_for_personal_dm(
        &self,
        user_id: &UserId,
    ) -> Result<RebornOutboundDeliveryTargetId, RebornServicesError> {
        RebornOutboundDeliveryTargetId::new(format!(
            "slack:personal-dm:{}:{}",
            self.team_id.as_str(),
            user_id.as_str()
        ))
        .map_err(|_| slack_target_backend_error())
    }

    pub(crate) fn channel_id_for_target_id<'a>(
        &self,
        target_id: &'a RebornOutboundDeliveryTargetId,
    ) -> Option<&'a str> {
        target_id
            .as_str()
            .strip_prefix(&self.shared_target_id_prefix)
            .filter(|channel_id| !channel_id.is_empty())
    }

    fn user_id_for_personal_target_id(
        &self,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Option<UserId> {
        UserId::new(
            target_id
                .as_str()
                .strip_prefix(&self.personal_target_id_prefix)?,
        )
        .ok()
    }

    fn route_for_reply_target_binding_ref(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ParsedSlackReplyTarget> {
        let mut raw = target.as_str().strip_prefix("reply:")?;
        let (adapter_id, rest) = take_product_binding_segment(raw, "adapter")?;
        if adapter_id != SLACK_V2_ADAPTER_ID {
            return None;
        }
        raw = rest;
        let (installation_id, rest) = take_product_binding_segment(raw, "installation")?;
        if installation_id != self.installation_id.as_str() {
            return None;
        }
        raw = rest;
        let (agent_id, rest) = take_product_binding_segment(raw, "agent")?;
        if agent_id != self.agent_id.as_str() {
            return None;
        }
        raw = rest;
        let (project_id, rest) = take_product_binding_segment(raw, "project")?;
        if project_id != self.project_id.as_ref().map_or("", |id| id.as_str()) {
            return None;
        }
        raw = rest;
        let (space_id, rest) = take_product_binding_segment(raw, "space")?;
        if space_id != self.team_id.as_str() {
            return None;
        }
        raw = rest;
        let (conversation_id, rest) = take_product_binding_segment(raw, "conversation")?;
        let (topic_id, rest) = take_product_binding_segment(rest, "topic")?;
        if conversation_id.is_empty() || !topic_id.is_empty() {
            return None;
        }
        if rest.is_empty() {
            return Some(ParsedSlackReplyTarget::SharedChannel {
                channel_id: conversation_id.to_string(),
            });
        }
        let (actor_kind, rest) = take_product_binding_segment(rest, "actor_kind")?;
        let (actor_id, rest) = take_product_binding_segment(rest, "actor")?;
        if actor_kind != SLACK_USER_ACTOR_KIND || actor_id.is_empty() || !rest.is_empty() {
            return None;
        }
        Some(ParsedSlackReplyTarget::PersonalDm {
            dm_channel_id: conversation_id.to_string(),
            slack_user_id: SlackUserId::new(actor_id),
        })
    }

    #[allow(dead_code)]
    pub(crate) fn channel_id_for_reply_target_binding_ref(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<String> {
        match self.route_for_reply_target_binding_ref(target)? {
            ParsedSlackReplyTarget::SharedChannel { channel_id } => Some(channel_id),
            ParsedSlackReplyTarget::PersonalDm { .. } => None,
        }
    }

    async fn shared_channel_route_for_channel(
        &self,
        channel_id: &str,
    ) -> Result<Option<SlackConfiguredChannelRoute>, RebornServicesError> {
        let key = match SlackChannelRouteKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.as_str().to_string(),
            channel_id.to_string(),
        ) {
            Ok(key) => key,
            Err(SlackChannelRouteError::InvalidRoute) => return Ok(None),
            Err(error) => return Err(map_slack_target_route_error(error)),
        };
        if let Some(subject_user_id) = self
            .channel_route_store
            .resolve_subject_user_id(&key)
            .await
            .map_err(map_slack_target_route_error)?
        {
            return Ok(Some(SlackConfiguredChannelRoute::new(
                channel_id.to_string(),
                subject_user_id,
            )));
        }
        Ok(self
            .configured_channel_routes
            .iter()
            .find(|route| route.channel_id == channel_id)
            .cloned())
    }

    async fn shared_channel_routes(
        &self,
    ) -> Result<Vec<SlackConfiguredChannelRoute>, RebornServicesError> {
        let mut cursor = 0;
        let mut stored_channel_ids = HashSet::new();
        let mut routes = Vec::new();
        loop {
            let stored = self
                .channel_route_store
                .list_routes(
                    &self.tenant_id,
                    &self.installation_id,
                    self.team_id.as_str(),
                    cursor,
                    SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE,
                )
                .await
                .map_err(map_slack_target_route_error)?;
            for route in stored.routes {
                stored_channel_ids.insert(route.channel_id.clone());
                routes.push(SlackConfiguredChannelRoute::new(
                    route.channel_id,
                    UserId::new(route.subject_user_id).map_err(|_| slack_target_backend_error())?,
                ));
            }
            let Some(next_cursor) = stored.next_cursor else {
                break;
            };
            if next_cursor <= cursor {
                return Err(map_slack_target_route_error(
                    SlackChannelRouteError::StoreUnavailable,
                ));
            }
            cursor = next_cursor;
        }
        routes.extend(
            self.configured_channel_routes
                .iter()
                .filter(|route| !stored_channel_ids.contains(&route.channel_id))
                .cloned(),
        );
        Ok(routes)
    }

    fn entry_for_shared_channel_route(
        &self,
        route: &SlackConfiguredChannelRoute,
    ) -> Result<OutboundDeliveryTargetEntry, RebornServicesError> {
        let target_id = self.target_id_for_shared_channel(&route.channel_id)?;
        let display_name = format!("Slack channel {}", route.channel_id);
        Ok(OutboundDeliveryTargetEntry {
            summary: RebornOutboundDeliveryTargetSummary::new(
                target_id,
                "slack",
                display_name,
                Some(format!(
                    "Slack channel {} in team {}",
                    route.channel_id,
                    self.team_id.as_str()
                )),
            )
            .map_err(|_| slack_target_backend_error())?,
            capabilities: RebornOutboundDeliveryTargetCapabilities {
                final_replies: true,
                gate_prompts: true,
                auth_prompts: true,
            },
            reply_target_binding_ref: slack_shared_channel_reply_target_binding_ref(
                &self.installation_id,
                &self.agent_id,
                self.project_id.as_ref(),
                &self.team_id,
                &route.channel_id,
            )?,
        })
    }

    fn entry_for_personal_dm_target(
        &self,
        target: &SlackPersonalDmTarget,
    ) -> Result<OutboundDeliveryTargetEntry, RebornServicesError> {
        let target_id = self.target_id_for_personal_dm(&target.key.user_id)?;
        Ok(OutboundDeliveryTargetEntry {
            summary: RebornOutboundDeliveryTargetSummary::new(
                target_id,
                "slack",
                "Slack DM".to_string(),
                Some(format!("Slack DM in team {}", self.team_id.as_str())),
            )
            .map_err(|_| slack_target_backend_error())?,
            capabilities: RebornOutboundDeliveryTargetCapabilities {
                final_replies: true,
                gate_prompts: true,
                auth_prompts: true,
            },
            reply_target_binding_ref: slack_personal_dm_reply_target_binding_ref(
                &self.installation_id,
                &self.agent_id,
                self.project_id.as_ref(),
                &self.team_id,
                &target.dm_channel_id,
                &target.slack_user_id,
            )?,
        })
    }

    async fn resolve_for_channel_id(
        &self,
        caller: &WebUiAuthenticatedCaller,
        channel_id: &str,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(None);
        }
        let Some(route) = self.shared_channel_route_for_channel(channel_id).await? else {
            return Ok(None);
        };
        if route.subject_user_id != caller.user_id {
            return Ok(None);
        }
        self.entry_for_shared_channel_route(&route).map(Some)
    }

    async fn resolve_personal_dm_for_user(
        &self,
        caller: &WebUiAuthenticatedCaller,
        user_id: &UserId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id || &caller.user_id != user_id {
            return Ok(None);
        }
        let key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.as_str().to_string(),
            caller.user_id.clone(),
        )
        .map_err(map_slack_personal_dm_target_error)?;
        let Some(target) = self
            .personal_dm_target_store
            .load_personal_dm_target(&key)
            .await
            .map_err(map_slack_personal_dm_target_error)?
        else {
            return Ok(None);
        };
        self.entry_for_personal_dm_target(&target).map(Some)
    }

    async fn resolve_personal_dm_for_binding(
        &self,
        caller: &WebUiAuthenticatedCaller,
        dm_channel_id: &str,
        slack_user_id: &SlackUserId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(None);
        }
        let key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.as_str().to_string(),
            caller.user_id.clone(),
        )
        .map_err(map_slack_personal_dm_target_error)?;
        let Some(target) = self
            .personal_dm_target_store
            .load_personal_dm_target(&key)
            .await
            .map_err(map_slack_personal_dm_target_error)?
        else {
            return Ok(None);
        };
        if target.dm_channel_id != dm_channel_id || target.slack_user_id != *slack_user_id {
            return Ok(None);
        }
        self.entry_for_personal_dm_target(&target).map(Some)
    }
}

#[async_trait::async_trait]
impl OutboundDeliveryTargetProvider for SlackHostBetaOutboundTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(Vec::new());
        }
        let mut routes = self
            .shared_channel_routes()
            .await?
            .into_iter()
            .filter(|route| route.subject_user_id == caller.user_id)
            .collect::<Vec<_>>();
        routes.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
        let mut targets = routes
            .into_iter()
            .map(|route| self.entry_for_shared_channel_route(&route))
            .collect::<Result<Vec<_>, _>>()?;
        let key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.as_str().to_string(),
            caller.user_id.clone(),
        )
        .map_err(map_slack_personal_dm_target_error)?;
        match self
            .personal_dm_target_store
            .load_personal_dm_target(&key)
            .await
        {
            Ok(Some(target)) => match self.entry_for_personal_dm_target(&target) {
                Ok(target) => targets.push(target),
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        "Slack personal DM target was skipped while listing outbound targets"
                    );
                }
            },
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    %error,
                    "Slack personal DM target lookup failed while listing outbound targets"
                );
            }
        }
        Ok(targets)
    }

    async fn resolve_outbound_delivery_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if let Some(channel_id) = self.channel_id_for_target_id(target_id) {
            return self.resolve_for_channel_id(caller, channel_id).await;
        }
        let Some(user_id) = self.user_id_for_personal_target_id(target_id) else {
            return Ok(None);
        };
        self.resolve_personal_dm_for_user(caller, &user_id).await
    }

    async fn resolve_reply_target_binding(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        match self.route_for_reply_target_binding_ref(target) {
            Some(ParsedSlackReplyTarget::SharedChannel { channel_id }) => {
                self.resolve_for_channel_id(caller, &channel_id).await
            }
            Some(ParsedSlackReplyTarget::PersonalDm {
                dm_channel_id,
                slack_user_id,
            }) => {
                self.resolve_personal_dm_for_binding(caller, &dm_channel_id, &slack_user_id)
                    .await
            }
            None => Ok(None),
        }
    }
}

enum ParsedSlackReplyTarget {
    SharedChannel {
        channel_id: String,
    },
    PersonalDm {
        dm_channel_id: String,
        slack_user_id: SlackUserId,
    },
}

pub(crate) fn slack_shared_channel_reply_target_binding_ref(
    installation_id: &AdapterInstallationId,
    agent_id: &AgentId,
    project_id: Option<&ProjectId>,
    team_id: &SlackTeamId,
    channel_id: &str,
) -> Result<ReplyTargetBindingRef, RebornServicesError> {
    let conversation = ExternalConversationRef::new(Some(team_id.as_str()), channel_id, None, None)
        .map_err(|_| slack_target_backend_error())?;
    let raw = format!(
        "{}{}{}{}{}",
        product_binding_segment("adapter", SLACK_V2_ADAPTER_ID),
        product_binding_segment("installation", installation_id.as_str()),
        product_binding_segment("agent", agent_id.as_str()),
        product_binding_segment("project", project_id.map_or("", |id| id.as_str())),
        conversation.conversation_fingerprint()
    );
    slack_reply_target_binding_ref_from_raw(raw)
}

fn slack_personal_dm_reply_target_binding_ref(
    installation_id: &AdapterInstallationId,
    agent_id: &AgentId,
    project_id: Option<&ProjectId>,
    team_id: &SlackTeamId,
    dm_channel_id: &str,
    slack_user_id: &SlackUserId,
) -> Result<ReplyTargetBindingRef, RebornServicesError> {
    let conversation =
        ExternalConversationRef::new(Some(team_id.as_str()), dm_channel_id, None, None)
            .map_err(|_| slack_target_backend_error())?;
    let actor = ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id.as_str(), None::<&str>)
        .map_err(|_| slack_target_backend_error())?;
    let raw = format!(
        "{}{}{}{}{}{}{}",
        product_binding_segment("adapter", SLACK_V2_ADAPTER_ID),
        product_binding_segment("installation", installation_id.as_str()),
        product_binding_segment("agent", agent_id.as_str()),
        product_binding_segment("project", project_id.map_or("", |id| id.as_str())),
        conversation.conversation_fingerprint(),
        product_binding_segment("actor_kind", actor.kind()),
        product_binding_segment("actor", actor.id())
    );
    slack_reply_target_binding_ref_from_raw(raw)
}

pub(crate) fn slack_reply_target_binding_ref_from_raw(
    raw: String,
) -> Result<ReplyTargetBindingRef, RebornServicesError> {
    ReplyTargetBindingRef::new(format!("reply:{raw}")).map_err(|_| slack_target_backend_error())
}

// Keep this segment format in parity with
// `ExternalConversationRef::conversation_fingerprint`.
fn product_binding_segment(name: &str, value: &str) -> String {
    format!("{name}:{}:{value};", value.len())
}

fn take_product_binding_segment<'a>(raw: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let raw = raw.strip_prefix(name)?.strip_prefix(':')?;
    let (length, raw) = raw.split_once(':')?;
    let length = length.parse::<usize>().ok()?;
    let value = raw.get(..length)?;
    let raw = raw.get(length..)?.strip_prefix(';')?;
    Some((value, raw))
}

fn map_slack_target_route_error(error: SlackChannelRouteError) -> RebornServicesError {
    match error {
        SlackChannelRouteError::InvalidRoute => slack_target_not_found_error(),
        SlackChannelRouteError::StoreUnavailable => slack_target_backend_error(),
    }
}

fn map_slack_personal_dm_target_error(error: SlackPersonalDmTargetError) -> RebornServicesError {
    match error {
        SlackPersonalDmTargetError::InvalidTarget => slack_target_not_found_error(),
        SlackPersonalDmTargetError::StoreUnavailable
        | SlackPersonalDmTargetError::ProvisioningFailed(_) => slack_target_backend_error(),
    }
}

fn slack_target_not_found_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::NotFound,
        kind: RebornServicesErrorKind::NotFound,
        status_code: 404,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn slack_target_backend_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    }
}

fn validate_slack_id(field: &'static str, value: &str) -> Result<(), SlackPersonalDmTargetError> {
    if value.is_empty()
        || value.len() > 128
        || value.chars().any(|c| {
            c == '\0' || c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\' | ':' | ';')
        })
    {
        tracing::debug!(field, "invalid Slack id for personal DM target");
        return Err(SlackPersonalDmTargetError::InvalidTarget);
    }
    Ok(())
}

fn validate_slack_dm_channel_id(value: &str) -> Result<(), SlackPersonalDmTargetError> {
    validate_slack_id("slack dm channel", value)?;
    if !value.starts_with('D') {
        return Err(SlackPersonalDmTargetError::InvalidTarget);
    }
    Ok(())
}
