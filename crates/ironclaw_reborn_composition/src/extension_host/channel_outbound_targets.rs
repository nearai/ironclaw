//! Generic outbound delivery targets over the channel host assembly
//! (extension-runtime §5.4, P6 c-rest).
//!
//! One provider serves every ACTIVE channel extension whose composition lane
//! registered a [`PreferenceTargetCodec`] in the assembly extras. Targets
//! come from generic state only:
//!
//! - **Shared conversations** — the extension's `*_subject_routes`
//!   administrator value: entries whose subject is the caller become
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock, Weak};

use async_trait::async_trait;
use ironclaw_extension_host::SnapshotWatch;
use ironclaw_extension_host::active::ActiveExtension;
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_host_api::{AgentId, ExtensionId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_outbound::{OutboundDeliveryTargetProvider, OutboundError, RunFinalReplyDestination};
use ironclaw_product::{
    AdapterInstallationId, ExternalConversationRef, PreferenceTargetEncodeRequest,
};
use ironclaw_product::{
    CurrentDeliveryTarget, CurrentDeliveryTargetResolver, PreferenceTargetCodec,
    ProductWorkflowError,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};

use crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver;
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
    OutboundDeliveryTargetId, OutboundDeliveryTargetOwner, OutboundDeliveryTargetScope,
    OutboundDeliveryTargetSummary,
};

/// Product-facing current-target authority over the canonical outbound
/// registry. The registry owns caller scoping and destination identity; this
/// adapter only asks the registered vendor codec to decode the already
/// authorized external binding into the product workflow's conversation
/// shape.
///
/// The assembly is attached after runtime construction. A weak reference
/// avoids a cycle because the assembly's delivery dependencies hold this
/// resolver. Before attachment (or after assembly shutdown), external target
/// decoding fails closed while host-owned destinations such as WebApp remain
/// resolvable directly through the canonical registry.
pub(crate) struct ComposedCurrentDeliveryTargetResolver {
    targets: Arc<MutableOutboundDeliveryTargetRegistry>,
    assembly: RwLock<Weak<GenericChannelHostAssembly>>,
}

impl ComposedCurrentDeliveryTargetResolver {
    pub(crate) fn new(targets: Arc<MutableOutboundDeliveryTargetRegistry>) -> Self {
        Self {
            targets,
            assembly: RwLock::new(Weak::new()),
        }
    }

    pub(crate) fn attach_assembly(
        &self,
        assembly: &Arc<GenericChannelHostAssembly>,
    ) -> Result<(), ProductWorkflowError> {
        let mut slot = self
            .assembly
            .write()
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("current delivery target assembly lock failed: {error}"),
            })?;
        *slot = Arc::downgrade(assembly);
        Ok(())
    }

    fn assembly(&self) -> Result<Option<Arc<GenericChannelHostAssembly>>, ProductWorkflowError> {
        self.assembly
            .read()
            .map(|assembly| assembly.upgrade())
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("current delivery target assembly lock failed: {error}"),
            })
    }

    fn outbound_scope(tenant_id: TenantId, user_id: UserId) -> OutboundDeliveryTargetScope {
        OutboundDeliveryTargetScope::new(tenant_id, user_id)
    }
}

#[async_trait]
impl CurrentDeliveryTargetResolver for ComposedCurrentDeliveryTargetResolver {
    async fn resolve_current_target(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<CurrentDeliveryTarget>, ProductWorkflowError> {
        let caller = Self::outbound_scope(scope.tenant_id.clone(), actor.user_id.clone());
        let Some(entry) = self
            .targets
            .resolve_reply_target_binding(&caller, target)
            .await
            .map_err(map_current_target_error)?
        else {
            return Ok(None);
        };
        let RunFinalReplyDestination::External {
            reply_target_binding_ref,
        } = entry.destination
        else {
            return Ok(None);
        };
        let Some(assembly) = self.assembly()? else {
            return Ok(None);
        };
        let extension_id = entry.summary.channel.as_str().to_string();
        let Some(codec) = assembly.preference_target_codec(&extension_id) else {
            return Ok(None);
        };
        let Some(external_conversation_ref) =
            codec.conversation_for_target(&reply_target_binding_ref)
        else {
            return Ok(None);
        };
        Ok(Some(CurrentDeliveryTarget {
            extension_id,
            external_conversation_ref,
            personal_direct_message: codec.is_personal_direct_message(&reply_target_binding_ref),
        }))
    }

    async fn resolve_current_destination(
        &self,
        scope: &ResourceScope,
        target_id: &OutboundDeliveryTargetId,
    ) -> Result<Option<RunFinalReplyDestination>, ProductWorkflowError> {
        let caller = Self::outbound_scope(scope.tenant_id.clone(), scope.user_id.clone());
        self.targets
            .resolve_outbound_delivery_target(&caller, target_id)
            .await
            .map(|entry| entry.map(|entry| entry.destination))
            .map_err(map_current_target_error)
    }

    async fn resolve_current_target_id(
        &self,
        scope: &ResourceScope,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetId>, ProductWorkflowError> {
        let caller = Self::outbound_scope(scope.tenant_id.clone(), scope.user_id.clone());
        self.targets
            .resolve_reply_target_binding(&caller, target)
            .await
            .map(|entry| entry.map(|entry| entry.summary.target_id))
            .map_err(map_current_target_error)
    }
}

fn map_current_target_error(error: OutboundError) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: format!("current delivery target lookup failed: {error}"),
    }
}

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
    pub(crate) admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
    /// Durable caller-membership authority. The active snapshot and
    /// administrator configuration are tenant-global and therefore cannot
    /// authorize a requesting user's personal target access.
    pub(crate) installation_store: Arc<dyn ExtensionInstallationStore>,
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

    /// Per-request contexts for every active channel extension the caller is
    /// currently a durable member of, with a registered preference-target
    /// codec, in extension-id order.
    async fn contexts(
        &self,
        caller: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<ChannelTargetContext>, OutboundError> {
        let visible_installations = self
            .deps
            .installation_store
            .list_installations()
            .await
            .map_err(|error| {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_outbound_targets",
                    %error,
                    "installation membership unavailable while resolving outbound targets"
                );
                OutboundError::Backend
            })?
            .into_iter()
            .filter(|installation| installation.owner().visible_to(&caller.user_id))
            .fold(
                BTreeMap::<String, BTreeSet<String>>::new(),
                |mut visible, installation| {
                    visible
                        .entry(installation.extension_id().as_str().to_string())
                        .or_default()
                        .insert(installation.installation_id().as_str().to_string());
                    visible
                },
            );
        let snapshot = self.deps.watch.current();
        let mut contexts = Vec::new();
        for extension_id in snapshot.extension_ids() {
            let Some(active) = snapshot.extension(&extension_id) else {
                continue;
            };
            if !visible_installations
                .get(&active.extension_id)
                .is_some_and(|installation_ids| installation_ids.contains(&active.installation_id))
            {
                continue;
            }
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
        if let Some(field) = active
            .resolved
            .admin_configuration
            .iter()
            .flat_map(|descriptor| &descriptor.fields)
            .filter(|field| !field.secret)
            .find(|field| handle_declares_field(field.handle.as_str(), "team_id"))
        {
            space_id = self
                .config_value(&extension_id, field.handle.as_str())
                .await?
                .filter(|value| !value.trim().is_empty());
        }

        let mut subject_routes = BTreeMap::new();
        let handles = shared_channel_admission_handles(&active.resolved.admin_configuration);
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
            .admin_configuration_resolver
            .non_secret_value(extension_id, handle)
            .await
            .map_err(|error| {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_outbound_targets",
                    extension_id = %extension_id,
                    handle,
                    %error,
                    "administrator configuration unavailable while resolving outbound targets"
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
            destination: ironclaw_outbound::RunFinalReplyDestination::External {
                reply_target_binding_ref,
            },
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
        let (record_space_id, conversation_id) = dm_record_conversation(record)?;
        let space_id = record_space_id.or_else(|| context.space_id.clone());
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
            destination: ironclaw_outbound::RunFinalReplyDestination::External {
                reply_target_binding_ref,
            },
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
        for context in self.contexts(caller).await? {
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
        for context in self.contexts(caller).await? {
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
        for context in self.contexts(caller).await? {
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
