//! Generic outbound delivery targets over the channel host assembly
//! (extension-runtime §5.4, P6 c-rest).
//!
//! One provider serves every ACTIVE channel extension whose composition lane
//! registered a [`PreferenceTargetCodec`] in the assembly extras. Targets
//! come from generic state only:
//!
//! - **Shared conversations** — the extension's `*_subject_routes`
//!   `[channel.config]` value: entries whose subject is the caller become
//!   the caller's shared-conversation targets.
//! - **Personal direct messages** — the generic per-(extension, user)
//!   DM-target store seeded by post-bind provisioning (and the H.4 fold).
//!
//! Binding refs are encoded through the vendor codec with the DURABLE
//! installation id from the active snapshot. Resolution of a STORED
//! preference binding ref deliberately never validates the ref's embedded
//! installation id: stored beta preferences carry the retired setup id, and
//! ownership is proven against the caller-scoped route/DM state instead —
//! so saved targets keep resolving across the setup→durable-id migration
//! (each resolve returns a freshly encoded ref carrying the durable id).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::SnapshotWatch;
use ironclaw_extension_host::active::ActiveExtension;
use ironclaw_host_api::{AgentId, ExtensionId, ProjectId, TenantId, UserId};
use ironclaw_outbound::OutboundError;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalConversationRef, PreferenceTargetEncodeRequest,
};
use ironclaw_product_workflow::PreferenceTargetCodec;
use ironclaw_turns::ReplyTargetBindingRef;

use crate::extension_host::channel_config::ChannelConfigService;
use crate::extension_host::channel_dm_targets::{
    ChannelDmTargetRecord, DM_TARGET_CONVERSATION_ID_KEY, DM_TARGET_SPACE_ID_KEY,
    FilesystemChannelDmTargetStore,
};
use crate::extension_host::channel_host::GenericChannelHostAssembly;
use crate::extension_host::channel_subject_routes::{
    handle_declares_field, shared_channel_admission_handles,
};
use crate::outbound::{
    DeliveryTargetCapabilities, MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetEntry,
    OutboundDeliveryTargetId, OutboundDeliveryTargetOwner, OutboundDeliveryTargetProvider,
    OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary,
};

/// The deployment identity every encoded binding ref carries (the same
/// identity the assembly binds per-extension workflows under).
#[derive(Clone)]
pub(crate) struct ChannelOutboundTargetIdentity {
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
}

/// Everything the generic provider reads. All reads are per-request:
/// configure saves and DM provisioning take effect on the next call.
pub(crate) struct GenericChannelOutboundTargetDeps {
    pub(crate) watch: SnapshotWatch,
    pub(crate) assembly: Arc<GenericChannelHostAssembly>,
    pub(crate) channel_config: Arc<ChannelConfigService>,
    pub(crate) dm_targets: Arc<FilesystemChannelDmTargetStore>,
    pub(crate) identity: ChannelOutboundTargetIdentity,
}

/// The generic outbound-delivery-target provider (replaces the retired
/// per-vendor lane-owned target providers).
pub(crate) struct GenericChannelOutboundTargetProvider {
    deps: GenericChannelOutboundTargetDeps,
}

/// One extension's per-request target context.
struct ChannelTargetContext {
    extension_id: String,
    display_name: String,
    installation_id: AdapterInstallationId,
    codec: Arc<dyn PreferenceTargetCodec>,
    /// The `*_team_id` connection-scoping claim value — the space every
    /// encoded conversation binds under. `None` until configured.
    space_id: Option<String>,
    /// Explicit subject routes (`*_subject_routes`): conversation id → the
    /// subject user id delivery in that conversation belongs to.
    subject_routes: BTreeMap<String, String>,
}

impl GenericChannelOutboundTargetProvider {
    pub(crate) fn new(deps: GenericChannelOutboundTargetDeps) -> Self {
        Self { deps }
    }

    /// Per-request contexts for every active channel extension with a
    /// registered preference-target codec, in extension-id order.
    async fn contexts(&self) -> Result<Vec<ChannelTargetContext>, OutboundError> {
        let snapshot = self.deps.watch.current();
        let mut contexts = Vec::new();
        for extension_id in snapshot.extension_ids() {
            let Some(active) = snapshot.extension(&extension_id) else {
                continue;
            };
            let Some(context) = self.context_for_extension(&active).await? else {
                continue;
            };
            contexts.push(context);
        }
        Ok(contexts)
    }

    async fn context_for_extension(
        &self,
        active: &ActiveExtension,
    ) -> Result<Option<ChannelTargetContext>, OutboundError> {
        let Some(channel) = active.resolved.channel.as_ref() else {
            return Ok(None);
        };
        if !channel.outbound {
            return Ok(None);
        }
        let Some(codec) = self
            .deps
            .assembly
            .preference_target_codec(&active.extension_id)
        else {
            return Ok(None);
        };
        let Ok(installation_id) = AdapterInstallationId::new(&active.installation_id) else {
            tracing::warn!(
                target = "ironclaw::reborn::channel_outbound_targets",
                extension_id = %active.extension_id,
                "active installation id is not a valid adapter installation id; \
                 extension offers no outbound targets"
            );
            return Ok(None);
        };
        let Ok(extension_id) = ExtensionId::new(&active.extension_id) else {
            return Ok(None);
        };

        // The `*_team_id` connection-scoping claim (same handle-suffix
        // convention as the identity hook) supplies the space id.
        let mut space_id = None;
        if let Some(field) = channel
            .config
            .fields
            .iter()
            .filter(|field| !field.secret)
            .find(|field| handle_declares_field(field.handle.as_str(), "team_id"))
        {
            space_id = self
                .config_value(&extension_id, field.handle.as_str())
                .await?
                .filter(|value| !value.trim().is_empty());
        }

        let mut subject_routes = BTreeMap::new();
        let handles = shared_channel_admission_handles(&channel.config.fields);
        if let Some(handle) = handles.subject_routes.as_deref()
            && let Some(raw) = self.config_value(&extension_id, handle).await?
        {
            match serde_json::from_str::<BTreeMap<String, String>>(&raw) {
                Ok(routes) => subject_routes = routes,
                Err(error) => {
                    tracing::warn!(
                        target = "ironclaw::reborn::channel_outbound_targets",
                        extension_id = %active.extension_id,
                        handle,
                        %error,
                        "subject-route config value is not a JSON object; \
                         treating as no routes"
                    );
                }
            }
        }

        Ok(Some(ChannelTargetContext {
            extension_id: active.extension_id.clone(),
            display_name: active.resolved.name.clone(),
            installation_id,
            codec,
            space_id,
            subject_routes,
        }))
    }

    async fn config_value(
        &self,
        extension_id: &ExtensionId,
        handle: &str,
    ) -> Result<Option<String>, OutboundError> {
        self.deps
            .channel_config
            .non_secret_value(extension_id, handle)
            .await
            .map_err(|error| {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_outbound_targets",
                    extension_id = %extension_id,
                    handle,
                    %error,
                    "channel config unavailable while resolving outbound targets"
                );
                OutboundError::Backend
            })
    }

    fn encode_request<'a>(
        &'a self,
        context: &'a ChannelTargetContext,
        conversation: &'a ExternalConversationRef,
    ) -> PreferenceTargetEncodeRequest<'a> {
        PreferenceTargetEncodeRequest {
            installation_id: &context.installation_id,
            agent_id: &self.deps.identity.agent_id,
            project_id: self.deps.identity.project_id.as_ref(),
            conversation,
        }
    }

    /// Build the caller's shared-conversation entry for one routed
    /// conversation. `None` when the vendor codec cannot encode it (for
    /// example the space claim is not configured yet) — fail closed.
    fn shared_entry(
        &self,
        context: &ChannelTargetContext,
        conversation_id: &str,
    ) -> Option<OutboundDeliveryTargetEntry> {
        // The owner is derived from the route's subject (the resolved
        // resource), never echoed from the caller, so the registry's
        // caller-scoping filter stays genuine defense in depth.
        let subject = context.subject_routes.get(conversation_id)?;
        let owner_user = UserId::new(subject.clone()).ok()?;
        let conversation =
            ExternalConversationRef::new(context.space_id.as_deref(), conversation_id, None, None)
                .ok()?;
        let reply_target_binding_ref = context
            .codec
            .encode_shared_conversation_target(self.encode_request(context, &conversation))?;
        let target_id = OutboundDeliveryTargetId::new(format!(
            "{}:shared-channel:{}:{}",
            context.extension_id,
            context.space_id.as_deref().unwrap_or_default(),
            conversation_id
        ))
        .ok()?;
        let summary = OutboundDeliveryTargetSummary::new(
            target_id,
            context.extension_id.as_str(),
            format!("{} channel {}", context.display_name, conversation_id),
            Some(format!(
                "{} channel {} in {}",
                context.display_name,
                conversation_id,
                context.space_id.as_deref().unwrap_or("this workspace")
            )),
        )
        .ok()?;
        Some(OutboundDeliveryTargetEntry {
            summary,
            capabilities: full_capabilities(),
            reply_target_binding_ref,
            owner: OutboundDeliveryTargetOwner::new(
                self.deps.identity.tenant_id.clone(),
                owner_user,
            ),
        })
    }

    /// Build the caller's personal-DM entry from the provisioned record.
    fn dm_entry(
        &self,
        context: &ChannelTargetContext,
        caller: &OutboundDeliveryTargetScope,
        record: &ChannelDmTargetRecord,
    ) -> Option<OutboundDeliveryTargetEntry> {
        let (space_id, conversation_id) = dm_record_conversation(record)?;
        let conversation =
            ExternalConversationRef::new(space_id.as_deref(), &conversation_id, None, None).ok()?;
        let reply_target_binding_ref = context.codec.encode_personal_direct_message_target(
            self.encode_request(context, &conversation),
            &record.external_actor_id,
        )?;
        let target_id = OutboundDeliveryTargetId::new(format!(
            "{}:personal-dm:{}:{}",
            context.extension_id,
            space_id.as_deref().unwrap_or_default(),
            caller.user_id.as_str()
        ))
        .ok()?;
        let summary = OutboundDeliveryTargetSummary::new(
            target_id,
            context.extension_id.as_str(),
            format!("{} DM", context.display_name),
            Some(format!(
                "{} DM in {}",
                context.display_name,
                space_id.as_deref().unwrap_or("this workspace")
            )),
        )
        .ok()?;
        Some(OutboundDeliveryTargetEntry {
            summary,
            capabilities: full_capabilities(),
            reply_target_binding_ref,
            // The owner is the record's provisioned user (the resolved
            // resource), never echoed from the caller.
            owner: OutboundDeliveryTargetOwner::new(
                self.deps.identity.tenant_id.clone(),
                UserId::new(record.user_id.clone()).ok()?,
            ),
        })
    }

    /// The caller's provisioned DM-target record, if any.
    async fn dm_record(
        &self,
        context: &ChannelTargetContext,
        caller: &OutboundDeliveryTargetScope,
    ) -> Result<Option<ChannelDmTargetRecord>, OutboundError> {
        self.deps
            .dm_targets
            .load(&context.extension_id, &caller.user_id)
            .await
            .map_err(|error| {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_outbound_targets",
                    extension_id = %context.extension_id,
                    %error,
                    "channel DM-target store unavailable while resolving outbound targets"
                );
                OutboundError::Backend
            })
    }

    fn caller_in_scope(&self, caller: &OutboundDeliveryTargetScope) -> bool {
        caller.tenant_id == self.deps.identity.tenant_id
    }

    /// Whether the routed subject for one conversation is the caller.
    fn conversation_routed_to_caller(
        context: &ChannelTargetContext,
        conversation_id: &str,
        caller: &OutboundDeliveryTargetScope,
    ) -> bool {
        context
            .subject_routes
            .get(conversation_id)
            .is_some_and(|subject| subject == caller.user_id.as_str())
    }
}

#[async_trait]
impl OutboundDeliveryTargetProvider for GenericChannelOutboundTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        if !self.caller_in_scope(caller) {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for context in self.contexts().await? {
            for (conversation_id, subject) in &context.subject_routes {
                if subject != caller.user_id.as_str() {
                    continue;
                }
                if let Some(entry) = self.shared_entry(&context, conversation_id) {
                    entries.push(entry);
                }
            }
            if let Some(record) = self.dm_record(&context, caller).await?
                && let Some(entry) = self.dm_entry(&context, caller, &record)
            {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    async fn resolve_outbound_delivery_target(
        &self,
        caller: &OutboundDeliveryTargetScope,
        target_id: &OutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        if !self.caller_in_scope(caller) {
            return Ok(None);
        }
        for context in self.contexts().await? {
            let space = context.space_id.as_deref().unwrap_or_default();
            let shared_prefix = format!("{}:shared-channel:{}:", context.extension_id, space);
            if let Some(conversation_id) = target_id
                .as_str()
                .strip_prefix(&shared_prefix)
                .filter(|conversation_id| !conversation_id.is_empty())
            {
                if !Self::conversation_routed_to_caller(&context, conversation_id, caller) {
                    return Ok(None);
                }
                return Ok(self.shared_entry(&context, conversation_id));
            }
            let personal_prefix = format!("{}:personal-dm:{}:", context.extension_id, space);
            if let Some(user_id) = target_id.as_str().strip_prefix(&personal_prefix) {
                if user_id != caller.user_id.as_str() {
                    return Ok(None);
                }
                let Some(record) = self.dm_record(&context, caller).await? else {
                    return Ok(None);
                };
                return Ok(self.dm_entry(&context, caller, &record));
            }
        }
        Ok(None)
    }

    async fn resolve_reply_target_binding(
        &self,
        caller: &OutboundDeliveryTargetScope,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        if !self.caller_in_scope(caller) {
            return Ok(None);
        }
        for context in self.contexts().await? {
            let Some(decoded) = context.codec.conversation_for_target(target) else {
                continue;
            };
            if context.codec.is_personal_direct_message(target) {
                let Some(record) = self.dm_record(&context, caller).await? else {
                    return Ok(None);
                };
                let Some((_, record_conversation_id)) = dm_record_conversation(&record) else {
                    return Ok(None);
                };
                if record_conversation_id != decoded.conversation_id() {
                    return Ok(None);
                }
                // The presented ref's actor must be the provisioned actor —
                // a tampered actor segment never resolves.
                if context
                    .codec
                    .direct_message_actor_for_target(target)
                    .as_deref()
                    != Some(record.external_actor_id.as_str())
                {
                    return Ok(None);
                }
                return Ok(self.dm_entry(&context, caller, &record));
            }
            if !Self::conversation_routed_to_caller(&context, decoded.conversation_id(), caller) {
                return Ok(None);
            }
            return Ok(self.shared_entry(&context, decoded.conversation_id()));
        }
        Ok(None)
    }
}

/// The canonical DM-target payload's conversation ref.
fn dm_record_conversation(record: &ChannelDmTargetRecord) -> Option<(Option<String>, String)> {
    let conversation_id = record
        .target
        .get(DM_TARGET_CONVERSATION_ID_KEY)?
        .as_str()?
        .to_string();
    let space_id = record
        .target
        .get(DM_TARGET_SPACE_ID_KEY)
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some((space_id, conversation_id))
}

fn full_capabilities() -> DeliveryTargetCapabilities {
    DeliveryTargetCapabilities {
        final_replies: true,
        progress: false,
        gate_prompts: true,
        auth_prompts: true,
        modalities: Vec::new(),
    }
}

/// Register the generic provider on a mutable outbound-target registry.
pub(crate) fn register_generic_channel_outbound_targets(
    registry: &MutableOutboundDeliveryTargetRegistry,
    deps: GenericChannelOutboundTargetDeps,
) {
    if let Err(error) = registry.register_provider(
        GENERIC_CHANNEL_OUTBOUND_TARGET_PROVIDER_KEY,
        Arc::new(GenericChannelOutboundTargetProvider::new(deps)),
    ) {
        tracing::warn!(
            target = "ironclaw::reborn::channel_outbound_targets",
            error = ?error,
            "generic channel outbound-target provider could not be registered"
        );
    }
}

pub(crate) const GENERIC_CHANNEL_OUTBOUND_TARGET_PROVIDER_KEY: &str =
    "generic-channel-outbound-targets";
