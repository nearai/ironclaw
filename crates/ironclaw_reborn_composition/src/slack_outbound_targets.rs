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
use ironclaw_product_adapters::{
    AdapterInstallationId, EgressCredentialHandle, ExternalActorRef, ExternalConversationRef,
    ProtocolHttpEgress,
};
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
use crate::slack_dm_open::{SlackDmOpenError, open_slack_dm_channel, validate_slack_dm_channel_id};
use crate::slack_serve::{SlackTeamId, SlackUserId};

pub(crate) const SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE: usize = 500;
const SLACK_OUTBOUND_TARGET_LIST_MAX_TOTAL_ROUTES: usize = 10_000;

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
    pub(crate) team_id: SlackTeamId,
    pub(crate) user_id: UserId,
}

impl SlackPersonalDmTargetKey {
    pub(crate) fn new(
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
        team_id: SlackTeamId,
        user_id: UserId,
    ) -> Result<Self, SlackPersonalDmTargetError> {
        validate_slack_id("slack team", team_id.as_str())?;
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
        validate_slack_dm_channel_id(&dm_channel_id)
            .map_err(|_| SlackPersonalDmTargetError::InvalidTarget)?;
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
    ProvisioningFailed(String),
}

#[async_trait::async_trait]
pub(crate) trait SlackPersonalDmTargetStore: Send + Sync + std::fmt::Debug {
    async fn load_personal_dm_target(
        &self,
        key: &SlackPersonalDmTargetKey,
    ) -> Result<Option<SlackPersonalDmTarget>, SlackPersonalDmTargetError>;

    async fn upsert_personal_dm_target(
        &self,
        target: SlackPersonalDmTarget,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError>;

    async fn delete_personal_dm_target(
        &self,
        key: &SlackPersonalDmTargetKey,
    ) -> Result<bool, SlackPersonalDmTargetError>;

    async fn delete_personal_dm_targets_for_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        installation_id: &AdapterInstallationId,
        team_id: &SlackTeamId,
    ) -> Result<usize, SlackPersonalDmTargetError>;
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

    async fn delete_personal_dm_target(
        &self,
        key: &SlackPersonalDmTargetKey,
    ) -> Result<bool, SlackPersonalDmTargetError> {
        Ok(self
            .targets
            .write()
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?
            .remove(key)
            .is_some())
    }

    async fn delete_personal_dm_targets_for_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        installation_id: &AdapterInstallationId,
        team_id: &SlackTeamId,
    ) -> Result<usize, SlackPersonalDmTargetError> {
        let mut targets = self
            .targets
            .write()
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?;
        let before = targets.len();
        targets.retain(|key, _| {
            !(&key.tenant_id == tenant_id
                && &key.user_id == user_id
                && &key.installation_id == installation_id
                && &key.team_id == team_id)
        });
        Ok(before - targets.len())
    }
}

pub(crate) struct SlackPersonalDmTargetProvisioner {
    tenant_id: TenantId,
    installation_id: AdapterInstallationId,
    team_id: SlackTeamId,
    egress: Arc<dyn ProtocolHttpEgress>,
    credential_handle: EgressCredentialHandle,
    store: Arc<dyn SlackPersonalDmTargetStore>,
}

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

    pub(crate) async fn provision_for_user(
        &self,
        user_id: UserId,
        slack_user_id: SlackUserId,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        let key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.clone(),
            user_id,
        )?;
        let dm_channel_id = self.open_dm_channel(slack_user_id.as_str()).await?;
        let target = SlackPersonalDmTarget::new(key, slack_user_id, dm_channel_id)?;
        self.store.upsert_personal_dm_target(target).await
    }

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
            SlackDmOpenError::InvalidChannel | SlackDmOpenError::MissingChannel => {
                SlackPersonalDmTargetError::InvalidTarget
            }
            SlackDmOpenError::Backend(reason) => {
                SlackPersonalDmTargetError::ProvisioningFailed(reason)
            }
        })?;
        validate_slack_dm_channel_id(&channel_id)
            .map_err(|_| SlackPersonalDmTargetError::InvalidTarget)?;
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

    #[cfg(test)]
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
            self.team_id.clone(),
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
            self.team_id.clone(),
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
        let personal_dm_key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.clone(),
            caller.user_id.clone(),
        );
        let personal_dm_target = async {
            match personal_dm_key {
                Ok(key) => Some(
                    self.personal_dm_target_store
                        .load_personal_dm_target(&key)
                        .await,
                ),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Slack personal DM target key could not be built while listing outbound targets"
                    );
                    None
                }
            }
        };
        let subject_routes = self.channel_route_store.list_routes_for_subject(
            &self.tenant_id,
            &self.installation_id,
            self.team_id.as_str(),
            &caller.user_id,
            SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE,
            SLACK_OUTBOUND_TARGET_LIST_MAX_TOTAL_ROUTES,
        );
        let (stored_routes, personal_dm_target) = tokio::join!(subject_routes, personal_dm_target);
        let stored_routes = stored_routes.map_err(map_slack_target_route_error)?;
        // Collect the channel ids returned for this subject so we can skip
        // static configured routes that the store has already overridden with
        // any subject (including a different one).  Collect as owned Strings so
        // `stored_routes` can be moved into the route vec below.
        let stored_channel_ids: HashSet<String> =
            stored_routes.iter().map(|r| r.channel_id.clone()).collect();
        let mut routes: Vec<SlackConfiguredChannelRoute> = stored_routes
            .into_iter()
            .map(|r| {
                UserId::new(r.subject_user_id)
                    .map_err(|_| slack_target_backend_error())
                    .map(|uid| SlackConfiguredChannelRoute::new(r.channel_id, uid))
            })
            .collect::<Result<_, _>>()?;
        // For each static configured route belonging to this caller that is not
        // already in the subject-scoped store results, check whether the store
        // has ANY entry for that channel (even under a different subject).  If
        // the store has overridden the channel, omit the stale static entry so
        // the admin-assigned subject wins.  A true subject index remains a
        // tracked follow-up; for now the N point-lookups over the (typically
        // small) static route list are cheaper than a full-inventory scan.
        for static_route in self
            .configured_channel_routes
            .iter()
            .filter(|r| r.subject_user_id == caller.user_id)
        {
            if stored_channel_ids.contains(&static_route.channel_id) {
                // Already present from the subject-scoped store results.
                continue;
            }
            let key = match SlackChannelRouteKey::new(
                self.tenant_id.clone(),
                self.installation_id.clone(),
                self.team_id.as_str().to_string(),
                static_route.channel_id.clone(),
            ) {
                Ok(key) => key,
                Err(_) => continue,
            };
            let store_override = self
                .channel_route_store
                .resolve_subject_user_id(&key)
                .await
                .map_err(map_slack_target_route_error)?;
            if store_override.is_none() {
                // No store entry for this channel — static route is active.
                routes.push(static_route.clone());
            }
            // If the store has an entry for another subject, the static route
            // is suppressed (admin override takes precedence).
        }
        routes.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
        let mut targets = routes
            .into_iter()
            .map(|route| self.entry_for_shared_channel_route(&route))
            .collect::<Result<Vec<_>, _>>()?;
        match personal_dm_target {
            Some(Ok(Some(target))) => match self.entry_for_personal_dm_target(&target) {
                Ok(target) => targets.push(target),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Slack personal DM target was skipped while listing outbound targets"
                    );
                }
            },
            Some(Ok(None)) => {}
            Some(Err(error)) => {
                tracing::warn!(
                    %error,
                    "Slack personal DM target lookup failed while listing outbound targets"
                );
            }
            None => {}
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
    // Safety: all callers must pre-validate inputs via validate_slack_id /
    // validate_slack_dm_channel_id which reject control characters (including NUL).
    // ReplyTargetBindingRef::new enforces the 256-byte limit and rejects control chars
    // as the primary defense — these caller-side validators are defense-in-depth.
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

/// Decoded fields from a Slack reply-target binding ref segment walk.
///
/// Produced by [`decode_slack_reply_target_binding_ref`]; consumed by both
/// [`slack_conversation_id_from_reply_target_binding_ref`] and
/// [`slack_reply_target_is_personal_dm`] so that the segment format is
/// parsed in exactly one place.
struct DecodedSlackReplyTarget {
    conversation_id: String,
    space_id: Option<String>,
    /// `true` iff a `topic` segment was successfully consumed after
    /// `conversation` — required for the personal-DM predicate to mirror
    /// the original behaviour, which demanded `topic` be present.
    topic_present: bool,
    /// Present only when the ref carries trailing `actor_kind` + `actor`
    /// segments (personal-DM refs).
    actor_kind: Option<String>,
    /// Present only when the ref carries a non-empty `actor` segment value.
    actor_id: Option<String>,
    /// `true` iff the segment walk consumed the entire ref with no leftover
    /// bytes — required for the personal-DM predicate to reject trailing
    /// garbage.
    fully_consumed: bool,
}

/// Walk the length-prefixed segment format shared by both
/// [`slack_shared_channel_reply_target_binding_ref`] and
/// [`slack_personal_dm_reply_target_binding_ref`] and return the decoded
/// fields.
///
/// Returns `None` if the ref does not start with `reply:`, if any of the
/// required prefix segments (adapter / installation / agent / project / space /
/// conversation) are missing or malformed, or if the conversation id is empty.
/// Optional trailing segments (topic / actor_kind / actor) are consumed when
/// present but are not required; whether they were found is recorded in
/// `topic_present`, `actor_kind`, and `actor_id`.
fn decode_slack_reply_target_binding_ref(
    target: &ironclaw_turns::ReplyTargetBindingRef,
) -> Option<DecodedSlackReplyTarget> {
    let mut raw = target.as_str().strip_prefix("reply:")?;
    // Validate the adapter segment — reject non-Slack adapter refs fail-closed.
    let (adapter_id, rest) = take_product_binding_segment(raw, "adapter")?;
    if adapter_id != SLACK_V2_ADAPTER_ID {
        return None;
    }
    raw = rest;
    // Skip installation, agent, project — values not validated here.
    for name in &["installation", "agent", "project"] {
        let (_, rest) = take_product_binding_segment(raw, name)?;
        raw = rest;
    }
    // Consume the required conversation fingerprint segments: space / conversation.
    let (space_id, rest) = take_product_binding_segment(raw, "space")?;
    let (conversation_id, rest) = take_product_binding_segment(rest, "conversation")?;
    if conversation_id.is_empty() {
        return None;
    }
    // Consume the optional topic segment (present in both shared-channel and
    // personal-DM refs built by the constructors in this module).
    let (topic_present, rest) = match take_product_binding_segment(rest, "topic") {
        Some((_, after_topic)) => (true, after_topic),
        None => (false, rest),
    };
    // Consume the optional actor_kind + actor segments (personal-DM only).
    let (actor_kind, actor_id, rest) = match take_product_binding_segment(rest, "actor_kind") {
        Some((kind, after_kind)) => match take_product_binding_segment(after_kind, "actor") {
            Some((id, after_actor)) => {
                // Map an empty actor value to None so the type matches the
                // doc-comment ("Present only when the `actor` segment exists
                // with a non-empty value").
                let actor_id = if id.is_empty() {
                    None
                } else {
                    Some(id.to_string())
                };
                (Some(kind.to_string()), actor_id, after_actor)
            }
            None => (Some(kind.to_string()), None, after_kind),
        },
        None => (None, None, rest),
    };
    Some(DecodedSlackReplyTarget {
        conversation_id: conversation_id.to_string(),
        space_id: if space_id.is_empty() {
            None
        } else {
            Some(space_id.to_string())
        },
        topic_present,
        actor_kind,
        actor_id,
        fully_consumed: rest.is_empty(),
    })
}

/// Extract the Slack channel ID encoded in a Slack reply-target binding ref.
///
/// Parses the length-prefixed segment format produced by
/// [`slack_shared_channel_reply_target_binding_ref`] /
/// [`slack_personal_dm_reply_target_binding_ref`] without requiring the full
/// config for validation. Used by the triggered-run delivery path to
/// reconstruct the outbound `ExternalConversationRef` from a stored
/// preference.
///
/// Returns `None` if the ref is not a Slack reply target or is malformed.
///
/// Note: adapter identity is enforced by [`decode_slack_reply_target_binding_ref`],
/// so refs with a non-Slack adapter now return `None` where they previously
/// returned `Some`.
pub(crate) fn slack_conversation_id_from_reply_target_binding_ref(
    target: &ironclaw_turns::ReplyTargetBindingRef,
) -> Option<(String, Option<String>)> {
    let decoded = decode_slack_reply_target_binding_ref(target)?;
    Some((decoded.conversation_id, decoded.space_id))
}

/// Returns `true` iff `target` is a verified personal-DM Slack reply-target
/// binding ref.
///
/// A personal-DM ref has two structural markers beyond the shared segment
/// prefix (adapter / installation / agent / project / space / conversation /
/// topic):
///
/// 1. Trailing `actor_kind` + `actor` segments are **present** — this is the
///    distinguishing property that [`slack_personal_dm_reply_target_binding_ref`]
///    adds and [`slack_shared_channel_reply_target_binding_ref`] omits.
/// 2. The decoded conversation id (the DM channel id) **starts with uppercase
///    `'D'`** — the Slack-assigned prefix for all DM channels.
///
/// Any parse failure (malformed segment encoding, missing fields, non-`D`
/// channel id) returns `false` — fail closed.
///
/// This does NOT validate the installation, agent, project, or space values
/// against a live provider config. It is a structural parse only: safe to
/// call before a provider is available.
pub(crate) fn slack_reply_target_is_personal_dm(
    target: &ironclaw_turns::ReplyTargetBindingRef,
) -> bool {
    let Some(decoded) = decode_slack_reply_target_binding_ref(target) else {
        return false;
    };
    // A well-formed Slack reply target always carries the team/space binding. An
    // empty `space` segment (`space:0:`) decodes to `None` here; reject it so a
    // corrupted or forged preference missing the space cannot satisfy the
    // OAuth-DM gate.
    if decoded.space_id.is_none() {
        return false;
    }
    // DM channel ids start with uppercase 'D'.
    if !decoded.conversation_id.starts_with('D') {
        return false;
    }
    // The conversation fingerprint must include a topic segment (as produced by
    // `ExternalConversationRef::conversation_fingerprint`); refs missing it are
    // not well-formed personal-DM refs.
    if !decoded.topic_present {
        return false;
    }
    // actor_kind must be the Slack user kind; actor_id is Some only when the
    // actor segment is non-empty (invariant enforced at decode time); no
    // further segments are permitted.
    decoded.actor_kind.as_deref() == Some(SLACK_USER_ACTOR_KIND)
        && decoded.actor_id.is_some()
        && decoded.fully_consumed
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack_channel_routes::{
        InMemorySlackChannelRouteStore, SlackChannelRouteError, SlackChannelRouteKey,
        SlackChannelRouteListPage,
    };

    // ── test constants ────────────────────────────────────────────────────────
    const TENANT: &str = "tenant:alpha";
    const INSTALLATION: &str = "install-alpha";
    const TEAM: &str = "T123";
    const USER: &str = "user:alice";
    const OTHER_TENANT: &str = "tenant:other";
    const OTHER_USER: &str = "user:bob";
    const SLACK_USER: &str = "U123";
    const AGENT: &str = "agent:alpha";
    const PROJECT: &str = "project:alpha";

    // ── test helpers ──────────────────────────────────────────────────────────

    fn caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(USER).expect("user"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        )
    }

    fn provider_config(
        channel_routes: Vec<SlackConfiguredChannelRoute>,
    ) -> SlackOutboundTargetProviderConfig {
        SlackOutboundTargetProviderConfig {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: AdapterInstallationId::new(INSTALLATION).expect("installation"),
            team_id: SlackTeamId::new(TEAM),
            configured_channel_routes: channel_routes,
        }
    }

    fn empty_provider() -> SlackHostBetaOutboundTargetProvider {
        SlackHostBetaOutboundTargetProvider::new(
            provider_config(Vec::new()),
            Arc::new(InMemorySlackChannelRouteStore::new()),
            Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        )
    }

    async fn provider_with_provisioned_dm() -> (
        SlackHostBetaOutboundTargetProvider,
        Arc<InMemorySlackPersonalDmTargetStore>,
    ) {
        let store = Arc::new(InMemorySlackPersonalDmTargetStore::new());
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new(TENANT).expect("tenant"),
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            SlackTeamId::new(TEAM),
            UserId::new(USER).expect("user"),
        )
        .expect("key");
        let target =
            SlackPersonalDmTarget::new(key, SlackUserId::new(SLACK_USER), "D0HOST".to_string())
                .expect("target");
        store
            .upsert_personal_dm_target(target)
            .await
            .expect("stores");
        let store_dyn: Arc<dyn SlackPersonalDmTargetStore> = Arc::clone(&store) as _;
        let provider = SlackHostBetaOutboundTargetProvider::new(
            provider_config(Vec::new()),
            Arc::new(InMemorySlackChannelRouteStore::new()),
            store_dyn,
        );
        (provider, store)
    }

    // ── validate_slack_id ─────────────────────────────────────────────────────

    #[test]
    fn validate_slack_id_accepts_128_char_id_and_rejects_129_char_id() {
        validate_slack_id("slack id", &"A".repeat(128)).expect("128 chars is valid");
        assert!(matches!(
            validate_slack_id("slack id", &"A".repeat(129)),
            Err(SlackPersonalDmTargetError::InvalidTarget)
        ));
    }

    #[test]
    fn validate_slack_id_rejects_whitespace_and_special_chars() {
        for value in ["", "A B", "A\0B", "A/B", "A\\B", "A:B", "A;B", "A\nB"] {
            assert!(matches!(
                validate_slack_id("slack id", value),
                Err(SlackPersonalDmTargetError::InvalidTarget)
            ));
        }
    }

    // ── validate_slack_dm_channel_id ──────────────────────────────────────────

    #[test]
    fn slack_personal_dm_target_rejects_non_d_prefixed_channel_id() {
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            AdapterInstallationId::new("install-alpha").expect("installation"),
            SlackTeamId::new("T123"),
            UserId::new("user:alice").expect("user"),
        )
        .expect("personal target key");

        assert!(matches!(
            SlackPersonalDmTarget::new(key.clone(), SlackUserId::new("U123"), "C123".to_string()),
            Err(SlackPersonalDmTargetError::InvalidTarget)
        ));
        SlackPersonalDmTarget::new(key.clone(), SlackUserId::new("U123"), "D123".to_string())
            .expect("DM-prefixed channel is valid");
        // lowercase 'd' prefix is also invalid (must be uppercase 'D')
        assert!(matches!(
            SlackPersonalDmTarget::new(key.clone(), SlackUserId::new("U123"), "d123".to_string()),
            Err(SlackPersonalDmTargetError::InvalidTarget)
        ));
        // empty is invalid
        assert!(matches!(
            SlackPersonalDmTarget::new(key, SlackUserId::new("U123"), String::new()),
            Err(SlackPersonalDmTargetError::InvalidTarget)
        ));
    }

    // ── slack_personal_dm_reply_target_binding_ref round-trip ────────────────

    #[test]
    fn slack_personal_dm_reply_target_binding_ref_round_trips_dm_channel_and_slack_user() {
        let provider = empty_provider();
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let agent_id = AgentId::new(AGENT).expect("agent");
        let project_id = ProjectId::new(PROJECT).expect("project");
        let team_id = SlackTeamId::new(TEAM);
        let slack_user_id = SlackUserId::new(SLACK_USER);

        let binding_ref = slack_personal_dm_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            &team_id,
            "D0HOST",
            &slack_user_id,
        )
        .expect("binding ref builds");

        // parse it back via route_for_reply_target_binding_ref
        let parsed = provider
            .route_for_reply_target_binding_ref(&binding_ref)
            .expect("binding ref parses to a Slack reply target");

        match parsed {
            ParsedSlackReplyTarget::PersonalDm {
                dm_channel_id,
                slack_user_id: parsed_slack_user_id,
            } => {
                assert_eq!(dm_channel_id, "D0HOST", "dm_channel_id must round-trip");
                assert_eq!(
                    parsed_slack_user_id.as_str(),
                    SLACK_USER,
                    "slack_user_id must round-trip"
                );
            }
            ParsedSlackReplyTarget::SharedChannel { .. } => {
                panic!("personal DM binding ref must not parse as shared channel");
            }
        }
    }

    // ── resolve_reply_target_binding PersonalDm branch ────────────────────────

    #[tokio::test]
    async fn slack_personal_dm_reply_target_binding_resolves_to_stored_target() {
        let (provider, _store) = provider_with_provisioned_dm().await;

        let listed = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect("target list");
        assert_eq!(listed.len(), 1, "provisioned DM target must appear");
        let binding_ref = listed[0].reply_target_binding_ref.clone();

        let resolved = provider
            .resolve_reply_target_binding(&caller(), &binding_ref)
            .await
            .expect("binding resolves without error")
            .expect("stored DM target must resolve to Some");

        assert_eq!(
            resolved.summary.target_id.as_str(),
            format!("slack:personal-dm:{}:{}", TEAM, USER)
        );
        assert_eq!(resolved.reply_target_binding_ref, binding_ref);
    }

    #[tokio::test]
    async fn slack_personal_dm_reply_target_binding_returns_none_when_no_target_stored() {
        // Build binding ref for a user that has no provisioned DM target.
        let provider = empty_provider();
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let agent_id = AgentId::new(AGENT).expect("agent");
        let project_id = ProjectId::new(PROJECT).expect("project");
        let team_id = SlackTeamId::new(TEAM);
        let slack_user_id = SlackUserId::new(SLACK_USER);

        let binding_ref = slack_personal_dm_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            &team_id,
            "D0HOST",
            &slack_user_id,
        )
        .expect("binding ref builds");

        // No target stored → should return Ok(None) (not an error).
        let result = provider
            .resolve_reply_target_binding(&caller(), &binding_ref)
            .await
            .expect("lookup succeeds");
        assert!(result.is_none(), "ownerless binding ref must return None");
    }

    // ── mismatch-guard: stored slack_user_id differs from binding ref ─────────

    #[tokio::test]
    async fn slack_personal_dm_resolve_binding_rejects_mismatched_slack_user_id() {
        let (provider, _store) = provider_with_provisioned_dm().await;

        // Listing gives us a real binding ref that encodes SLACK_USER + D0HOST.
        let listed = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect("target list");
        assert_eq!(listed.len(), 1);

        // Replace the SLACK_USER segment in the raw binding ref string with a
        // different Slack user id, keeping the dm_channel_id the same.
        let original = listed[0].reply_target_binding_ref.as_str();
        let tampered_raw = original.replace(SLACK_USER, "U_OTHER_USER");
        let mismatched = ReplyTargetBindingRef::new(tampered_raw)
            .expect("tampered ref is still syntactically valid");

        let result = provider
            .resolve_reply_target_binding(&caller(), &mismatched)
            .await
            .expect("lookup succeeds");
        assert!(
            result.is_none(),
            "mismatched slack_user_id in binding ref must return None"
        );
    }

    // ── cross-user: caller.user_id != requested user_id ──────────────────────

    #[tokio::test]
    async fn slack_personal_dm_resolve_personal_dm_for_user_returns_none_for_cross_user_caller() {
        let (provider, _store) = provider_with_provisioned_dm().await;

        // Target was provisioned for USER ("user:alice").  A different user
        // ("user:bob") tries to resolve the same target_id.
        let target_id =
            RebornOutboundDeliveryTargetId::new(format!("slack:personal-dm:{}:{}", TEAM, USER))
                .expect("target id");

        let cross_user_caller = WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(OTHER_USER).expect("other user"),
            None,
            None,
        );

        let result = provider
            .resolve_outbound_delivery_target(&cross_user_caller, &target_id)
            .await
            .expect("lookup succeeds");
        assert!(
            result.is_none(),
            "user B must not resolve user A's personal DM target"
        );
    }

    // ── list: both shared-channel route AND personal DM appear together ───────

    #[tokio::test]
    async fn slack_list_outbound_delivery_targets_returns_shared_channels_and_personal_dm_together()
    {
        let store = Arc::new(InMemorySlackPersonalDmTargetStore::new());
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new(TENANT).expect("tenant"),
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            SlackTeamId::new(TEAM),
            UserId::new(USER).expect("user"),
        )
        .expect("key");
        let target =
            SlackPersonalDmTarget::new(key, SlackUserId::new(SLACK_USER), "D0HOST".to_string())
                .expect("target");
        store
            .upsert_personal_dm_target(target)
            .await
            .expect("stores");

        // Add a static shared-channel route for the same caller.
        let shared_route = SlackConfiguredChannelRoute::new(
            "C0HOST".to_string(),
            UserId::new(USER).expect("user"),
        );
        let provider = SlackHostBetaOutboundTargetProvider::new(
            provider_config(vec![shared_route]),
            Arc::new(InMemorySlackChannelRouteStore::new()),
            store,
        );

        let listed = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect("target list");

        let target_ids: Vec<&str> = listed
            .iter()
            .map(|e| e.summary.target_id.as_str())
            .collect();
        assert!(
            target_ids.iter().any(|id| id.contains("shared-channel")),
            "shared-channel target must appear: {:?}",
            target_ids
        );
        assert!(
            target_ids.iter().any(|id| id.contains("personal-dm")),
            "personal-DM target must appear: {:?}",
            target_ids
        );
        assert_eq!(listed.len(), 2, "exactly one shared + one DM target");
    }

    // ── shared_channel_routes cap guard ──────────────────────────────────────

    #[derive(Debug)]
    struct OversizedPageRouteStore;

    #[async_trait::async_trait]
    impl SlackChannelRouteStore for OversizedPageRouteStore {
        async fn list_routes(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &AdapterInstallationId,
            _team_id: &str,
            _cursor: usize,
            _limit: usize,
        ) -> Result<SlackChannelRouteListPage, SlackChannelRouteError> {
            // Return more routes than the cap in a single page (no next cursor,
            // so the loop will try the cap check on this batch).
            let routes = (0..=SLACK_OUTBOUND_TARGET_LIST_MAX_TOTAL_ROUTES)
                .map(|i| crate::slack_channel_routes::SlackChannelRoute {
                    tenant_id: TENANT.to_string(),
                    installation_id: INSTALLATION.to_string(),
                    team_id: TEAM.to_string(),
                    channel_id: format!("C{i:05}"),
                    subject_user_id: USER.to_string(),
                })
                .collect();
            Ok(SlackChannelRouteListPage {
                routes,
                next_cursor: None,
            })
        }

        async fn upsert_route(
            &self,
            _key: SlackChannelRouteKey,
            _subject_user_id: UserId,
        ) -> Result<crate::slack_channel_routes::SlackChannelRoute, SlackChannelRouteError>
        {
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
            _assignments: Vec<crate::slack_channel_routes::SlackChannelRouteAssignment>,
        ) -> Result<Vec<crate::slack_channel_routes::SlackChannelRoute>, SlackChannelRouteError>
        {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn resolve_subject_user_id(
            &self,
            _key: &SlackChannelRouteKey,
        ) -> Result<Option<UserId>, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }
    }

    #[tokio::test]
    async fn slack_shared_channel_routes_fails_when_stored_batch_exceeds_max_total() {
        let provider = SlackHostBetaOutboundTargetProvider::new(
            provider_config(Vec::new()),
            Arc::new(OversizedPageRouteStore),
            Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        );

        let error = provider
            .list_outbound_delivery_targets(&caller())
            .await
            .expect_err("oversized batch must fail closed");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    // ── list_routes_for_subject ───────────────────────────────────────────────

    #[tokio::test]
    async fn list_routes_for_subject_returns_only_callers_routes_across_multiple_subjects() {
        let store = Arc::new(InMemorySlackChannelRouteStore::new());
        let tenant_id = TenantId::new(TENANT).expect("tenant");
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let alice = UserId::new(USER).expect("alice");
        let bob = UserId::new(OTHER_USER).expect("bob");

        // Seed: two channels for alice, one for bob.
        for (channel_id, subject) in [
            ("C0ALICE1", alice.clone()),
            ("C0ALICE2", alice.clone()),
            ("C0BOB1", bob.clone()),
        ] {
            let key = SlackChannelRouteKey::new(
                tenant_id.clone(),
                installation_id.clone(),
                TEAM.to_string(),
                channel_id.to_string(),
            )
            .expect("key");
            store.upsert_route(key, subject).await.expect("upsert");
        }

        // Alice's scoped listing must return exactly her two routes.
        let alice_routes = store
            .list_routes_for_subject(
                &tenant_id,
                &installation_id,
                TEAM,
                &alice,
                100,
                SLACK_OUTBOUND_TARGET_LIST_MAX_TOTAL_ROUTES,
            )
            .await
            .expect("listing succeeds");
        assert_eq!(alice_routes.len(), 2, "alice must see exactly 2 routes");
        assert!(
            alice_routes
                .iter()
                .all(|r| r.subject_user_id == alice.as_str()),
            "all returned routes must belong to alice"
        );
        assert!(
            alice_routes.iter().any(|r| r.channel_id == "C0ALICE1"),
            "C0ALICE1 must be present"
        );
        assert!(
            alice_routes.iter().any(|r| r.channel_id == "C0ALICE2"),
            "C0ALICE2 must be present"
        );

        // Bob's listing must return only his route — not alice's.
        let bob_routes = store
            .list_routes_for_subject(
                &tenant_id,
                &installation_id,
                TEAM,
                &bob,
                100,
                SLACK_OUTBOUND_TARGET_LIST_MAX_TOTAL_ROUTES,
            )
            .await
            .expect("listing succeeds");
        assert_eq!(bob_routes.len(), 1, "bob must see exactly 1 route");
        assert_eq!(bob_routes[0].channel_id, "C0BOB1");
    }

    #[tokio::test]
    async fn list_routes_for_subject_enforces_cap_on_subject_matching_routes() {
        // Reuse OversizedPageRouteStore: every route belongs to USER, so the
        // subject filter won't reduce the count and the cap must still fire.
        let store = Arc::new(OversizedPageRouteStore);
        let tenant_id = TenantId::new(TENANT).expect("tenant");
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let alice = UserId::new(USER).expect("alice");

        let result = store
            .list_routes_for_subject(
                &tenant_id,
                &installation_id,
                TEAM,
                &alice,
                SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE,
                SLACK_OUTBOUND_TARGET_LIST_MAX_TOTAL_ROUTES,
            )
            .await;
        assert!(
            matches!(result, Err(SlackChannelRouteError::StoreUnavailable)),
            "cap guard must fire when subject routes exceed the maximum: {result:?}"
        );
    }

    // ── cross-tenant personal-DM resolve ─────────────────────────────────────

    #[tokio::test]
    async fn slack_host_beta_targets_ignore_cross_tenant_personal_dm_resolve() {
        let (provider, _store) = provider_with_provisioned_dm().await;

        // Build a personal-DM target_id that matches the provisioned target.
        let target_id =
            RebornOutboundDeliveryTargetId::new(format!("slack:personal-dm:{}:{}", TEAM, USER))
                .expect("target id");

        // A caller from a different tenant with the same user_id.
        let foreign_tenant_caller = WebUiAuthenticatedCaller::new(
            TenantId::new(OTHER_TENANT).expect("other tenant"),
            UserId::new(USER).expect("user"),
            None,
            None,
        );

        let result = provider
            .resolve_outbound_delivery_target(&foreign_tenant_caller, &target_id)
            .await
            .expect("lookup succeeds without error");

        assert!(
            result.is_none(),
            "cross-tenant caller must not resolve personal DM target; got: {:?}",
            result.map(|e| e.summary.target_id.as_str().to_string())
        );
    }

    // ── slack_reply_target_is_personal_dm ─────────────────────────────────────

    /// Build a personal-DM binding ref from raw segment strings (no provider needed).
    fn dm_binding_ref(dm_channel_id: &str, slack_user_id: &str) -> ReplyTargetBindingRef {
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let agent_id = AgentId::new(AGENT).expect("agent");
        let project_id = ProjectId::new(PROJECT).expect("project");
        let team_id = SlackTeamId::new(TEAM);
        let slack_user = SlackUserId::new(slack_user_id);
        slack_personal_dm_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            &team_id,
            dm_channel_id,
            &slack_user,
        )
        .expect("personal DM binding ref")
    }

    /// Build a shared-channel binding ref from raw segment strings (no provider needed).
    fn shared_channel_binding_ref(channel_id: &str) -> ReplyTargetBindingRef {
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let agent_id = AgentId::new(AGENT).expect("agent");
        let project_id = ProjectId::new(PROJECT).expect("project");
        let team_id = SlackTeamId::new(TEAM);
        slack_shared_channel_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            &team_id,
            channel_id,
        )
        .expect("shared channel binding ref")
    }

    #[test]
    fn slack_reply_target_is_personal_dm_returns_true_for_dm_ref() {
        let binding_ref = dm_binding_ref("D0HOST", SLACK_USER);
        assert!(
            slack_reply_target_is_personal_dm(&binding_ref),
            "personal-DM binding ref must be identified as a personal DM"
        );
    }

    #[test]
    fn slack_reply_target_is_personal_dm_returns_false_for_shared_channel_ref() {
        let binding_ref = shared_channel_binding_ref("C0HOST");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "shared-channel binding ref must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn slack_reply_target_is_personal_dm_returns_false_for_non_d_prefixed_channel() {
        // Build a raw ref with actor_kind/actor but a non-'D' channel id
        // (e.g. a group DM channel starting with 'G'). The parser must reject it.
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", "slack_v2"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "G0GROUP"),
            seg("topic", ""),
            seg("actor_kind", "slack_user"),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "non-'D'-prefixed channel must NOT be identified as a personal DM"
        );
    }

    #[test]
    fn slack_reply_target_is_personal_dm_returns_false_for_malformed_ref() {
        let malformed = ReplyTargetBindingRef::new("reply:not-a-valid-segment-format".to_string())
            .expect("syntactically valid ref");
        assert!(
            !slack_reply_target_is_personal_dm(&malformed),
            "malformed ref must return false (fail closed)"
        );
    }

    #[test]
    fn slack_reply_target_is_personal_dm_returns_false_for_empty_actor() {
        // Build a ref that has actor_kind but empty actor id — must be rejected.
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", "slack_v2"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", "slack_user"),
            seg("actor", ""),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref with empty actor must NOT be identified as a personal DM"
        );
    }

    /// Tests the two reject predicates not covered by the sibling tests:
    ///
    /// (a) A syntactically correct personal-DM ref whose `actor_kind` segment
    ///     contains a non-Slack actor kind must return `false` — the predicate
    ///     requires `actor_kind == SLACK_USER_ACTOR_KIND`.
    ///
    /// (b) A personal-DM ref with an EXTRA trailing segment appended after
    ///     `actor` must return `false` — the predicate requires `rest.is_empty()`
    ///     after consuming the `actor` segment.
    #[test]
    fn slack_reply_target_is_personal_dm_rejects_wrong_actor_kind_and_trailing_segments() {
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }

        // (a) Wrong actor_kind: use a non-Slack actor kind.
        let raw_wrong_kind = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", "slack_v2"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", "not_a_slack_user"),
            seg("actor", SLACK_USER),
        );
        let binding_ref_wrong_kind =
            slack_reply_target_binding_ref_from_raw(raw_wrong_kind).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref_wrong_kind),
            "actor_kind != slack_user must NOT be identified as a personal DM"
        );

        // (b) Extra trailing segment after the actor segment.
        let raw_trailing = format!(
            "{}{}{}{}{}{}{}{}{}{}",
            seg("adapter", "slack_v2"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", "slack_user"),
            seg("actor", SLACK_USER),
            seg("extra", "unexpected"),
        );
        let binding_ref_trailing =
            slack_reply_target_binding_ref_from_raw(raw_trailing).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref_trailing),
            "trailing segment after actor must NOT be identified as a personal DM"
        );
    }

    // ── slack_reply_target_is_personal_dm: missing topic segment ─────────────

    #[test]
    fn slack_reply_target_is_personal_dm_returns_false_for_missing_topic_segment() {
        // Build a ref that has all DM segments EXCEPT the topic segment.
        // The decoder records topic_present=false when the topic segment is
        // absent, and the DM predicate requires topic_present=true.
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            // topic segment intentionally omitted
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref missing topic segment must NOT be identified as a personal DM"
        );
    }

    // ── slack_reply_target_is_personal_dm: empty space segment ───────────────

    #[test]
    fn slack_reply_target_is_personal_dm_returns_false_for_empty_space_segment() {
        // A ref with an empty `space` segment (`space:0:`) decodes space_id to
        // None. A forged/corrupted preference missing the team/space binding must
        // NOT satisfy the OAuth-DM gate.
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", ""),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref with empty space segment must NOT be identified as a personal DM"
        );
    }

    // ── slack_reply_target_is_personal_dm: wrong adapter ─────────────────────

    #[test]
    fn slack_reply_target_is_personal_dm_rejects_non_slack_adapter() {
        // Build a ref identical to a valid personal-DM ref but with a
        // non-Slack adapter id.  The decoder must fail-closed and return None,
        // so the DM predicate must return false.
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}{}{}",
            seg("adapter", "other_adapter"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            seg("actor", SLACK_USER),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "non-Slack adapter ref must NOT be identified as a personal DM"
        );
    }

    // ── slack_reply_target_is_personal_dm: missing actor segment (not just empty) ──

    #[test]
    fn slack_reply_target_is_personal_dm_returns_false_for_missing_actor_segment() {
        // A ref with actor_kind present but NO actor segment at all (label absent,
        // not just empty value).  The decoder returns actor_id = None because
        // take_product_binding_segment for "actor" fails; the DM predicate must
        // return false (actor_id is None).
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}{}",
            seg("adapter", SLACK_V2_ADAPTER_ID),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "D0HOST"),
            seg("topic", ""),
            seg("actor_kind", SLACK_USER_ACTOR_KIND),
            // actor segment intentionally absent
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            !slack_reply_target_is_personal_dm(&binding_ref),
            "ref with actor_kind but no actor segment must NOT be identified as a personal DM"
        );
    }

    // ── slack_conversation_id_from_reply_target_binding_ref: adapter contract ──

    #[test]
    fn slack_conversation_id_from_reply_target_binding_ref_returns_none_for_non_slack_adapter() {
        // A syntactically valid ref with a non-Slack adapter value must return
        // None, pinning the adapter-validation contract that the shared decoder
        // now enforces for this function as well.
        fn seg(name: &str, value: &str) -> String {
            format!("{}:{}:{};", name, value.len(), value)
        }
        let raw = format!(
            "{}{}{}{}{}{}{}",
            seg("adapter", "not_slack_v2"),
            seg("installation", INSTALLATION),
            seg("agent", AGENT),
            seg("project", PROJECT),
            seg("space", TEAM),
            seg("conversation", "C0HOST"),
            seg("topic", ""),
        );
        let binding_ref =
            slack_reply_target_binding_ref_from_raw(raw).expect("builds syntactically");
        assert!(
            slack_conversation_id_from_reply_target_binding_ref(&binding_ref).is_none(),
            "non-Slack adapter ref must return None from \
             slack_conversation_id_from_reply_target_binding_ref"
        );
    }
}
