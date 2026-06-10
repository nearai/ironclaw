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
use ironclaw_outbound::{
    CommunicationModality, OutboundError, ReplyTargetBindingClaim, ReplyTargetBindingValidator,
    ReplyTargetValidationRequest, ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef, ExternalConversationRef};
#[cfg(test)]
use ironclaw_product_adapters::{EgressCredentialHandle, ProtocolHttpEgress};
use ironclaw_product_workflow::{
    ProductOutboundTargetResolver, ProductWorkflowError, RebornOutboundDeliveryTargetCapabilities,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetSummary, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, VerifiedProductOutboundTargetMetadata,
    WebUiAuthenticatedCaller,
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};
use ironclaw_turns::ReplyTargetBindingRef;
use thiserror::Error;

use crate::outbound_preferences::{OutboundDeliveryTargetEntry, OutboundDeliveryTargetProvider};
use crate::slack_channel_routes::{
    SlackChannelRouteError, SlackChannelRouteKey, SlackChannelRouteStore,
};
use crate::slack_dm_open::validate_slack_dm_channel_id;
#[cfg(test)]
use crate::slack_dm_open::{SlackDmOpenError, open_slack_dm_channel};
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
    // arch-exempt: dead_code, reserved for explicit Slack DM provisioning product route, plan #4600
    #[allow(dead_code)]
    ProvisioningFailed(String),
}

#[async_trait::async_trait]
pub(crate) trait SlackPersonalDmTargetStore: Send + Sync + std::fmt::Debug {
    async fn load_personal_dm_target(
        &self,
        key: &SlackPersonalDmTargetKey,
    ) -> Result<Option<SlackPersonalDmTarget>, SlackPersonalDmTargetError>;

    // arch-exempt: dead_code, reserved for explicit Slack DM provisioning product route, plan #4600
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
        let (user_id, rest) = take_product_binding_segment(rest, "user")?;
        let (actor_id, rest) = take_product_binding_segment(rest, "actor")?;
        if actor_kind != SLACK_USER_ACTOR_KIND
            || actor_id.is_empty()
            || user_id.is_empty()
            || !rest.is_empty()
        {
            return None;
        }
        Some(ParsedSlackReplyTarget::PersonalDm {
            dm_channel_id: conversation_id.to_string(),
            user_id: UserId::new(user_id).ok()?,
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
                &target.key.user_id,
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
        binding_user_id: &UserId,
        dm_channel_id: &str,
        slack_user_id: &SlackUserId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(None);
        }
        // Defense-in-depth: reject if the binding's user_id differs from the
        // authenticated caller even when the call site already checks this.
        if binding_user_id != &caller.user_id {
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

    fn scope_matches(&self, request: &ReplyTargetValidationRequest) -> bool {
        request.scope.tenant_id == self.tenant_id
            && request.scope.agent_id.as_ref() == Some(&self.agent_id)
            && request.scope.project_id == self.project_id
            && request.modality == CommunicationModality::Text
    }

    async fn validate_shared_channel_target(
        &self,
        request: &ReplyTargetValidationRequest,
        channel_id: &str,
    ) -> Result<(), OutboundError> {
        if !self.scope_matches(request) {
            return Err(OutboundError::AccessDenied);
        }
        let route = self
            .shared_channel_route_for_channel(channel_id)
            .await
            .map_err(reborn_services_error_to_outbound_error)?
            .ok_or(OutboundError::AccessDenied)?;
        if let Some(owner_user_id) = request.scope.explicit_owner_user_id()
            && &route.subject_user_id != owner_user_id
        {
            return Err(OutboundError::AccessDenied);
        }
        Ok(())
    }

    async fn validate_personal_dm_target(
        &self,
        request: &ReplyTargetValidationRequest,
        dm_channel_id: &str,
        user_id: &UserId,
        slack_user_id: &SlackUserId,
    ) -> Result<(), OutboundError> {
        if !self.scope_matches(request) {
            return Err(OutboundError::AccessDenied);
        }
        let Some(owner_user_id) = request.scope.explicit_owner_user_id() else {
            return Err(OutboundError::AccessDenied);
        };
        if owner_user_id != user_id {
            return Err(OutboundError::AccessDenied);
        }
        let key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.as_str().to_string(),
            user_id.clone(),
        )
        .map_err(personal_dm_error_to_outbound_error)?;
        let target = self
            .personal_dm_target_store
            .load_personal_dm_target(&key)
            .await
            .map_err(personal_dm_error_to_outbound_error)?
            .ok_or(OutboundError::AccessDenied)?;
        if target.dm_channel_id != dm_channel_id || target.slack_user_id != *slack_user_id {
            return Err(OutboundError::AccessDenied);
        }
        Ok(())
    }

    async fn metadata_for_shared_channel(
        &self,
        channel_id: &str,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        self.shared_channel_route_for_channel(channel_id)
            .await
            .map_err(reborn_services_error_to_product_workflow_error)?
            .ok_or(ProductWorkflowError::BindingAccessDenied)?;
        let conversation =
            ExternalConversationRef::new(Some(self.team_id.as_str()), channel_id, None, None)
                .map_err(|_| ProductWorkflowError::BindingAccessDenied)?;
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: conversation,
            external_actor_ref: None,
        })
    }

    async fn metadata_for_personal_dm(
        &self,
        dm_channel_id: &str,
        user_id: &UserId,
        slack_user_id: &SlackUserId,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        let key = SlackPersonalDmTargetKey::new(
            self.tenant_id.clone(),
            self.installation_id.clone(),
            self.team_id.as_str().to_string(),
            user_id.clone(),
        )
        .map_err(personal_dm_error_to_product_workflow_error)?;
        let target = self
            .personal_dm_target_store
            .load_personal_dm_target(&key)
            .await
            .map_err(personal_dm_error_to_product_workflow_error)?
            .ok_or(ProductWorkflowError::BindingAccessDenied)?;
        if target.dm_channel_id != dm_channel_id || target.slack_user_id != *slack_user_id {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        let conversation =
            ExternalConversationRef::new(Some(self.team_id.as_str()), dm_channel_id, None, None)
                .map_err(|_| ProductWorkflowError::BindingAccessDenied)?;
        let actor =
            ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id.as_str(), None::<&str>)
                .map_err(|_| ProductWorkflowError::BindingAccessDenied)?;
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: conversation,
            external_actor_ref: Some(actor),
        })
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
            self.team_id.as_str().to_string(),
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

    async fn list_project_delivery_targets(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if caller.tenant_id != self.tenant_id {
            return Ok(Vec::new());
        }
        // Paginate through all shared-channel routes (all subjects) for this
        // installation so that project-level listings show every configured
        // channel regardless of which user the store assigned it to.
        let mut cursor = 0;
        let mut routes: Vec<SlackConfiguredChannelRoute> = Vec::new();
        loop {
            let page = self
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
            if routes.len() + page.routes.len() > SLACK_OUTBOUND_TARGET_LIST_MAX_TOTAL_ROUTES {
                return Err(map_slack_target_route_error(
                    SlackChannelRouteError::StoreUnavailable,
                ));
            }
            for route in page.routes {
                routes.push(
                    UserId::new(route.subject_user_id)
                        .map_err(|_| slack_target_backend_error())
                        .map(|uid| SlackConfiguredChannelRoute::new(route.channel_id, uid))?,
                );
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            if next_cursor <= cursor {
                return Err(map_slack_target_route_error(
                    SlackChannelRouteError::StoreUnavailable,
                ));
            }
            cursor = next_cursor;
        }
        routes.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
        routes
            .into_iter()
            .map(|route| self.entry_for_shared_channel_route(&route))
            .collect()
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

    async fn resolve_project_outbound_delivery_target(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        if let Some(channel_id) = self.channel_id_for_target_id(target_id) {
            if caller.tenant_id != self.tenant_id {
                return Ok(None);
            }
            let Some(route) = self.shared_channel_route_for_channel(channel_id).await? else {
                return Ok(None);
            };
            return self.entry_for_shared_channel_route(&route).map(Some);
        }
        Ok(None)
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
                user_id,
                slack_user_id,
            }) => {
                if caller.user_id != user_id {
                    return Ok(None);
                }
                self.resolve_personal_dm_for_binding(
                    caller,
                    &user_id,
                    &dm_channel_id,
                    &slack_user_id,
                )
                .await
            }
            None => Ok(None),
        }
    }

    async fn resolve_project_reply_target_binding(
        &self,
        caller: &WebUiAuthenticatedCaller,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, RebornServicesError> {
        match self.route_for_reply_target_binding_ref(target) {
            Some(ParsedSlackReplyTarget::SharedChannel { channel_id }) => {
                if caller.tenant_id != self.tenant_id {
                    return Ok(None);
                }
                let Some(route) = self.shared_channel_route_for_channel(&channel_id).await? else {
                    return Ok(None);
                };
                self.entry_for_shared_channel_route(&route).map(Some)
            }
            Some(ParsedSlackReplyTarget::PersonalDm { .. }) => Ok(None),
            None => Ok(None),
        }
    }
}

#[async_trait::async_trait]
impl ReplyTargetBindingValidator for SlackHostBetaOutboundTargetProvider {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        match self.route_for_reply_target_binding_ref(&request.candidate.target) {
            Some(ParsedSlackReplyTarget::SharedChannel { channel_id }) => {
                self.validate_shared_channel_target(&request, &channel_id)
                    .await?;
            }
            Some(ParsedSlackReplyTarget::PersonalDm {
                dm_channel_id,
                user_id,
                slack_user_id,
            }) => {
                self.validate_personal_dm_target(
                    &request,
                    &dm_channel_id,
                    &user_id,
                    &slack_user_id,
                )
                .await?;
            }
            None => return Err(OutboundError::AccessDenied),
        }
        Ok(ReplyTargetBindingClaim::new(request.candidate.target))
    }
}

#[async_trait::async_trait]
impl ProductOutboundTargetResolver for SlackHostBetaOutboundTargetProvider {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        match self.route_for_reply_target_binding_ref(target.target()) {
            Some(ParsedSlackReplyTarget::SharedChannel { channel_id }) => {
                self.metadata_for_shared_channel(&channel_id).await
            }
            Some(ParsedSlackReplyTarget::PersonalDm {
                dm_channel_id,
                user_id,
                slack_user_id,
            }) => {
                self.metadata_for_personal_dm(&dm_channel_id, &user_id, &slack_user_id)
                    .await
            }
            None => Err(ProductWorkflowError::BindingAccessDenied),
        }
    }
}

enum ParsedSlackReplyTarget {
    SharedChannel {
        channel_id: String,
    },
    PersonalDm {
        dm_channel_id: String,
        user_id: UserId,
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
    user_id: &UserId,
    slack_user_id: &SlackUserId,
) -> Result<ReplyTargetBindingRef, RebornServicesError> {
    let conversation =
        ExternalConversationRef::new(Some(team_id.as_str()), dm_channel_id, None, None)
            .map_err(|_| slack_target_backend_error())?;
    let actor = ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id.as_str(), None::<&str>)
        .map_err(|_| slack_target_backend_error())?;
    let raw = format!(
        "{}{}{}{}{}{}{}{}",
        product_binding_segment("adapter", SLACK_V2_ADAPTER_ID),
        product_binding_segment("installation", installation_id.as_str()),
        product_binding_segment("agent", agent_id.as_str()),
        product_binding_segment("project", project_id.map_or("", |id| id.as_str())),
        conversation.conversation_fingerprint(),
        product_binding_segment("actor_kind", actor.kind()),
        product_binding_segment("user", user_id.as_str()),
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

fn reborn_services_error_to_outbound_error(error: RebornServicesError) -> OutboundError {
    if error.retryable {
        OutboundError::Backend
    } else {
        OutboundError::AccessDenied
    }
}

fn reborn_services_error_to_product_workflow_error(
    error: RebornServicesError,
) -> ProductWorkflowError {
    if error.retryable {
        ProductWorkflowError::Transient {
            reason: "Slack outbound target lookup failed".to_string(),
        }
    } else {
        ProductWorkflowError::BindingAccessDenied
    }
}

fn personal_dm_error_to_outbound_error(error: SlackPersonalDmTargetError) -> OutboundError {
    match error {
        SlackPersonalDmTargetError::InvalidTarget => OutboundError::AccessDenied,
        SlackPersonalDmTargetError::StoreUnavailable
        | SlackPersonalDmTargetError::ProvisioningFailed(_) => OutboundError::Backend,
    }
}

fn personal_dm_error_to_product_workflow_error(
    error: SlackPersonalDmTargetError,
) -> ProductWorkflowError {
    match error {
        SlackPersonalDmTargetError::InvalidTarget => ProductWorkflowError::BindingAccessDenied,
        SlackPersonalDmTargetError::StoreUnavailable
        | SlackPersonalDmTargetError::ProvisioningFailed(_) => ProductWorkflowError::Transient {
            reason: "Slack personal DM target lookup failed".to_string(),
        },
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
            TEAM.to_string(),
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

    use ironclaw_host_api::ThreadId;
    use ironclaw_outbound::{OutboundPushCandidate, OutboundPushKind, ProjectionUpdateRef};
    use ironclaw_turns::{TurnActor, TurnScope};

    const THREAD: &str = "thread:slack";
    const SHARED_CHANNEL: &str = "C123";
    const DM_CHANNEL: &str = "D123";

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
            "T123".to_string(),
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

        let user_id = UserId::new(USER).expect("user");
        let binding_ref = slack_personal_dm_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            &team_id,
            "D0HOST",
            &user_id,
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
                user_id: _,
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

        let user_id = UserId::new(USER).expect("user");
        let binding_ref = slack_personal_dm_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&project_id),
            &team_id,
            "D0HOST",
            &user_id,
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
            TEAM.to_string(),
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

    #[tokio::test]
    async fn slack_reply_target_validator_accepts_ownerless_shared_channel_scope() {
        let provider = provider_with_configured_channel_route(USER);
        let target = slack_shared_channel_reply_target_binding_ref(
            &installation_id(),
            &agent_id(),
            Some(&project_id()),
            &SlackTeamId::new(TEAM),
            SHARED_CHANNEL,
        )
        .expect("shared channel target");
        let request = validation_request(ownerless_scope(), USER, target.clone());

        let claim = ReplyTargetBindingValidator::validate_reply_target(&provider, request)
            .await
            .expect("shared channel target validates");

        assert_eq!(claim.target, target);
    }

    #[tokio::test]
    async fn slack_reply_target_validator_accepts_explicit_owner_personal_dm() {
        let store = personal_dm_store_for(USER).await;
        let provider = provider_with_personal_dm_store(store);
        let target = slack_personal_dm_reply_target_binding_ref(
            &installation_id(),
            &agent_id(),
            Some(&project_id()),
            &SlackTeamId::new(TEAM),
            DM_CHANNEL,
            &UserId::new(USER).expect("user"),
            &SlackUserId::new(SLACK_USER),
        )
        .expect("personal DM target");
        let request = validation_request(explicit_owner_scope(USER), USER, target.clone());

        let claim = ReplyTargetBindingValidator::validate_reply_target(&provider, request)
            .await
            .expect("personal DM target validates for explicit owner");

        assert_eq!(claim.target, target);
    }

    #[tokio::test]
    async fn slack_reply_target_validator_rejects_personal_dm_without_explicit_owner() {
        let store = personal_dm_store_for(USER).await;
        let provider = provider_with_personal_dm_store(store);
        let target = slack_personal_dm_reply_target_binding_ref(
            &installation_id(),
            &agent_id(),
            Some(&project_id()),
            &SlackTeamId::new(TEAM),
            DM_CHANNEL,
            &UserId::new(USER).expect("user"),
            &SlackUserId::new(SLACK_USER),
        )
        .expect("personal DM target");
        let request = validation_request(actor_fallback_scope(), USER, target);

        let error = ReplyTargetBindingValidator::validate_reply_target(&provider, request)
            .await
            .expect_err("personal DM target requires an explicit owner");

        assert!(matches!(error, OutboundError::AccessDenied));
    }

    #[tokio::test]
    async fn slack_reply_target_validator_rejects_mismatched_owner_personal_dm() {
        let store = personal_dm_store_for(USER).await;
        let provider = provider_with_personal_dm_store(store);
        let target = slack_personal_dm_reply_target_binding_ref(
            &installation_id(),
            &agent_id(),
            Some(&project_id()),
            &SlackTeamId::new(TEAM),
            DM_CHANNEL,
            &UserId::new(USER).expect("user"),
            &SlackUserId::new(SLACK_USER),
        )
        .expect("personal DM target");
        let request = validation_request(explicit_owner_scope(OTHER_USER), OTHER_USER, target);

        let error = ReplyTargetBindingValidator::validate_reply_target(&provider, request)
            .await
            .expect_err("personal DM target must match owner authority");

        assert!(matches!(error, OutboundError::AccessDenied));
    }

    fn provider_with_configured_channel_route(
        subject_user_id: &str,
    ) -> SlackHostBetaOutboundTargetProvider {
        let mut config = provider_config(Vec::new());
        config
            .configured_channel_routes
            .push(SlackConfiguredChannelRoute::new(
                SHARED_CHANNEL.to_string(),
                UserId::new(subject_user_id).expect("subject user"),
            ));
        SlackHostBetaOutboundTargetProvider::new(
            config,
            Arc::new(InMemorySlackChannelRouteStore::new()),
            Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        )
    }

    fn provider_with_personal_dm_store(
        store: Arc<dyn SlackPersonalDmTargetStore>,
    ) -> SlackHostBetaOutboundTargetProvider {
        SlackHostBetaOutboundTargetProvider::new(
            provider_config(Vec::new()),
            Arc::new(InMemorySlackChannelRouteStore::new()),
            store,
        )
    }

    async fn personal_dm_store_for(user_id: &str) -> Arc<dyn SlackPersonalDmTargetStore> {
        let store = Arc::new(InMemorySlackPersonalDmTargetStore::new());
        let key = SlackPersonalDmTargetKey::new(
            tenant_id(),
            installation_id(),
            TEAM.to_string(),
            UserId::new(user_id).expect("user"),
        )
        .expect("personal target key");
        let target =
            SlackPersonalDmTarget::new(key, SlackUserId::new(SLACK_USER), DM_CHANNEL.to_string())
                .expect("personal DM target");
        store
            .upsert_personal_dm_target(target)
            .await
            .expect("personal DM target stores");
        store
    }

    fn validation_request(
        scope: TurnScope,
        actor_user_id: &str,
        target: ReplyTargetBindingRef,
    ) -> ReplyTargetValidationRequest {
        ReplyTargetValidationRequest {
            actor: TurnActor::new(UserId::new(actor_user_id).expect("actor")),
            modality: CommunicationModality::Text,
            candidate: OutboundPushCandidate {
                tenant_id: tenant_id(),
                agent_id: Some(agent_id()),
                project_id: Some(project_id()),
                thread_id: thread_id(),
                turn_run_id: None,
                target,
                kind: OutboundPushKind::FinalReply,
                projection_ref: ProjectionUpdateRef::new("projection:slack")
                    .expect("projection ref"),
                requires_reply_target_revalidation: true,
            },
            scope,
        }
    }

    fn explicit_owner_scope(owner_user_id: &str) -> TurnScope {
        TurnScope::new_with_owner(
            tenant_id(),
            Some(agent_id()),
            Some(project_id()),
            thread_id(),
            Some(UserId::new(owner_user_id).expect("owner")),
        )
    }

    fn ownerless_scope() -> TurnScope {
        TurnScope::new_with_owner(
            tenant_id(),
            Some(agent_id()),
            Some(project_id()),
            thread_id(),
            None,
        )
    }

    fn actor_fallback_scope() -> TurnScope {
        TurnScope::new(
            tenant_id(),
            Some(agent_id()),
            Some(project_id()),
            thread_id(),
        )
    }

    fn tenant_id() -> TenantId {
        TenantId::new(TENANT).expect("tenant")
    }

    fn installation_id() -> AdapterInstallationId {
        AdapterInstallationId::new(INSTALLATION).expect("installation")
    }

    fn agent_id() -> AgentId {
        AgentId::new(AGENT).expect("agent")
    }

    fn project_id() -> ProjectId {
        ProjectId::new(PROJECT).expect("project")
    }

    fn thread_id() -> ThreadId {
        ThreadId::new(THREAD).expect("thread")
    }
}
