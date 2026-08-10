//! The channel host's per-extension product cone, built here rather than there
//! (PROPOSAL §12.11 D-A).
//!
//! `ironclaw_extension_host` owns the generic ingress reconciler; what it used
//! to also own was the *construction* of product's concrete stack —
//! `DefaultProductSurface`, `DefaultInboundTurnService`,
//! `RebornFilesystemIdempotencyLedger`, `StaticProductInstallationResolver`,
//! `DirectConversationCommandAdmission`, `ProductConversationBindingService`
//! and `RunDeliveryObserver` — inline, which is the upward edge the
//! `products → loops` re-layer cannot carry. This module is the implementation
//! half of [`ChannelWorkflowFactory`]: the host declares the shape it needs,
//! composition wires this factory into the host's dependency bundle, and the
//! concrete stack never crosses the boundary in either direction.
//!
//! **What deliberately does not cross.** The durable
//! [`RebornFilesystemConversationServices`] the graph is built over is a
//! concrete `ironclaw_conversations` type, and `ironclaw_product_contracts`
//! may name only `ironclaw_host_api` + `ironclaw_extension_contracts`. It is
//! therefore constructed here, consumed here, and returned to nobody — the
//! host only ever held it in order to hand it straight back to product.
//! [`ChannelWorkflowStorageRoots`] crosses instead, because placement is the
//! host's policy and it is `VirtualPath`-shaped.

use std::num::NonZeroUsize;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::InboundAttachmentLander;
use ironclaw_conversations::RebornFilesystemConversationServices;
use ironclaw_extension_contracts::preference_target::ActivePreferenceTargetCodecs;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_loop_host::HostInputEnqueuePort;

/// Attribution bucket for the background-run notifier's own run-delivery
/// services. Never a channel selector — the notifier picks the delivering
/// extension per notification target from that target's catalog entry.
const BACKGROUND_RUN_NOTIFIER_ID: &str = "background-run-notifier";
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveredGateRouteStore, OutboundDeliveryTargetProvider,
    OutboundStateStorePort, TriggeredRunDelivery, TriggeredRunDeliveryStore,
};
use ironclaw_product_contracts::binding::ProductBindingResolver;
use ironclaw_product_contracts::channel_workflow::{
    ChannelRunDeliveryObserver, ChannelWorkflowFactory, ChannelWorkflowGraph,
    ChannelWorkflowRequest, ChannelWorkflowStorageRoots,
};
use ironclaw_product_contracts::prompt_source::{
    ApprovalPromptContextSource, BlockedAuthPromptSource,
};
use ironclaw_product_contracts::surface::ChannelInboundProductSurface;
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::{TurnCoordinator, TurnScope};

use crate::approval_interaction::ApprovalInteractionService;
use crate::auth_interaction::AuthInteractionService;
use crate::command_admission::DirectConversationCommandAdmission;
use crate::conversation_binding::{
    ProductConversationBindingService, ProductInstallationKey, ProductInstallationScope,
    StaticProductInstallationResolver,
};
use crate::delivery_coordinator::DeliveryCoordinator;
use crate::filesystem_ledger::RebornFilesystemIdempotencyLedger;
use crate::inbound_turn::DefaultInboundTurnService;
use crate::ledger::IdempotencyLedger;
use crate::reborn_services::ProjectFilesystemReader;
use crate::run_delivery::{
    RunDeliveryObserver, RunDeliveryServices, RunDeliverySettings, TriggeredRunDeliveryDriver,
};
use crate::workflow::DefaultProductSurface;

/// Settled-entry bounds for the per-extension durable idempotency ledger.
///
/// Moved with the ledger construction rather than left behind: they are
/// properties of *this* ledger's retention, and the host no longer builds one.
const CHANNEL_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;
const CHANNEL_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL: usize = 1_000;

/// The deployment identity every per-extension workflow binds under: the
/// composed runtime's tenant/agent/project plus the fallback operator user
/// host-initiated work is attributed to (notice-thread scopes, the durable
/// idempotency-ledger scope). Inbound conversations run as their invoking
/// actor, not this user.
#[derive(Clone)]
pub struct ChannelWorkflowIdentity {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub operator_user_id: UserId,
}

/// The outbound-delivery half of the factory's dependencies. Absent when the
/// composed runtime has no delivery coordinator — graphs are then ingress-only
/// (turns run; nothing watches them for channel replies).
pub struct ChannelWorkflowDeliveryServices {
    pub coordinator: Arc<DeliveryCoordinator>,
    /// Canonical project-filesystem authority the delivery coordinator
    /// materializes `/workspace/...` references through.
    pub project_filesystem: Arc<dyn ProjectFilesystemReader>,
    pub outbound_store: Arc<dyn OutboundStateStorePort>,
    pub route_store: Arc<dyn DeliveredGateRouteStore>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    /// The owner-scoped outbound target catalog. The background-run notifier
    /// resolves the creator's stored notification-channel ids through it at
    /// fire time.
    pub delivery_targets: Arc<dyn OutboundDeliveryTargetProvider>,
    pub approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
    pub blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    pub auth_flow_cancel: Option<Arc<dyn ironclaw_auth::product_prompt::BlockedAuthFlowCanceller>>,
    pub settings: RunDeliverySettings,
    /// Durable outcome record for proactive (trigger-fired) deliveries.
    pub triggered_delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
}

/// Everything the factory composes per-extension graphs from. Supplied once by
/// composition; nothing here varies per extension.
pub struct RebornChannelWorkflowServices {
    /// Substrate the per-extension durable workflow state is mounted on.
    pub filesystem: Arc<dyn RootFilesystem>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    /// Lands inbound attachment bytes into the project filesystem before the
    /// turn starts, so the turn itself begins byte-free with workspace refs.
    pub inbound_attachments: Arc<dyn InboundAttachmentLander>,
    /// Enqueues a message arriving on a busy thread as steering input for the
    /// active run instead of rejecting it.
    pub input_enqueue: Arc<dyn HostInputEnqueuePort>,
    pub approval_interaction: Option<Arc<dyn ApprovalInteractionService>>,
    pub auth_interaction: Option<Arc<dyn AuthInteractionService>>,
    pub identity: ChannelWorkflowIdentity,
    pub delivery: Option<ChannelWorkflowDeliveryServices>,
}

/// The [`ChannelWorkflowFactory`] over a composed runtime's substrate.
pub struct RebornChannelWorkflowFactory {
    services: RebornChannelWorkflowServices,
}

impl RebornChannelWorkflowFactory {
    pub fn new(services: RebornChannelWorkflowServices) -> Self {
        Self { services }
    }

    /// The exact attachment lander this factory lands inbound bytes through.
    ///
    /// Same shape and same reason as
    /// `TriggeredRunDeliveryDriver::communication_preferences`: production
    /// wiring must hand this factory the read-write workspace mount, and the
    /// only way to hold that to account is to read back the handle it actually
    /// captured. Ungated because it is an ordinary accessor for a dependency
    /// this type owns, not a seam that exists only under `cfg(test)`.
    pub fn inbound_attachment_lander(&self) -> Arc<dyn InboundAttachmentLander> {
        Arc::clone(&self.services.inbound_attachments)
    }

    /// The single background-run notifier, or `None` when the composed
    /// runtime has no delivery coordinator (nothing can notify).
    ///
    /// One notifier serves every channel extension: it resolves the creator's
    /// stored notification channels at fire time and decodes each target
    /// through the LIVE codec view supplied here (§12.11 D-A: the hook in
    /// `ironclaw_extension_host` names no product type, so composition builds
    /// the notifier through this factory). Binding-free by construction: a
    /// notification never resolves from a conversation binding, so the
    /// notifier is wired with the no-op binding service below.
    pub fn background_run_notifier(
        &self,
        target_codecs: Arc<dyn ActivePreferenceTargetCodecs>,
    ) -> Option<Arc<dyn TriggeredRunDelivery>> {
        let delivery = self.services.delivery.as_ref()?;
        let identity = &self.services.identity;
        let notice_thread_id =
            match ThreadId::new(format!("{BACKGROUND_RUN_NOTIFIER_ID}-channel-notices")) {
                Ok(thread_id) => thread_id,
                Err(error) => {
                    tracing::warn!(
                        target: "ironclaw::reborn::channel_workflow",
                        %error,
                        "invalid channel-notice thread id; background run notification unavailable"
                    );
                    return None;
                }
            };
        let services = RunDeliveryServices {
            project_filesystem: Arc::clone(&delivery.project_filesystem),
            binding_service: Arc::new(TriggeredNoopConversationBindingService),
            thread_service: Arc::clone(&self.services.thread_service),
            turn_coordinator: Arc::clone(&self.services.turn_coordinator),
            outbound_store: Arc::clone(&delivery.outbound_store),
            route_store: Arc::clone(&delivery.route_store),
            communication_preferences: Arc::clone(&delivery.communication_preferences),
            delivery_targets: Arc::clone(&delivery.delivery_targets),
            coordinator: Arc::clone(&delivery.coordinator),
            extension_id: BACKGROUND_RUN_NOTIFIER_ID.to_string(),
            fallback_notice_scope: self.notice_scope(notice_thread_id),
            approval_context: delivery.approval_context.clone(),
            blocked_auth_prompts: delivery.blocked_auth_prompts.clone(),
            auth_flow_cancel: delivery.auth_flow_cancel.clone(),
        };
        Some(Arc::new(TriggeredRunDeliveryDriver::with_settings(
            services,
            crate::run_delivery::triggered_run_delivery_settings(),
            Arc::clone(&delivery.triggered_delivery_store),
            target_codecs,
            identity.agent_id.clone(),
        )) as Arc<dyn TriggeredRunDelivery>)
    }

    fn notice_scope(&self, notice_thread_id: ThreadId) -> TurnScope {
        let identity = &self.services.identity;
        TurnScope::new_with_owner(
            identity.tenant_id.clone(),
            Some(identity.agent_id.clone()),
            identity.project_id.clone(),
            notice_thread_id,
            Some(identity.operator_user_id.clone()),
        )
    }
}

/// Mount one extension's storage roots at the fixed aliases the durable ledger
/// and conversation store resolve (`/engine/product_surface/idempotency`,
/// `/conversations`), then build both over them.
async fn build_channel_workflow_state(
    filesystem: &Arc<dyn RootFilesystem>,
    roots: &ChannelWorkflowStorageRoots,
    ledger_scope: ResourceScope,
) -> Result<ChannelWorkflowState, String> {
    let mount = |alias: &str, target: &VirtualPath| -> Result<MountGrant, String> {
        Ok(MountGrant::new(
            MountAlias::new(alias)
                .map_err(|error| format!("invalid workflow state mount alias: {error}"))?,
            target.clone(),
            MountPermissions::read_write_list_delete(),
        ))
    };
    let view = MountView::new(vec![
        mount("/engine/product_surface/idempotency", &roots.idempotency)?,
        mount("/conversations", &roots.conversations)?,
    ])
    .map_err(|error| format!("invalid workflow state mount view: {error}"))?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(filesystem),
        view,
    ));
    let settled_limit = NonZeroUsize::new(CHANNEL_IDEMPOTENCY_LEDGER_SETTLED_LIMIT)
        .ok_or_else(|| "settled entry limit must be non-zero".to_string())?;
    let prune_interval = NonZeroUsize::new(CHANNEL_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL)
        .ok_or_else(|| "settled prune interval must be non-zero".to_string())?;
    let ledger = RebornFilesystemIdempotencyLedger::new(Arc::clone(&scoped), ledger_scope)
        .with_settled_entry_limit(settled_limit)
        .with_settled_prune_interval(prune_interval);
    let conversations = RebornFilesystemConversationServices::new(scoped)
        .await
        .map_err(|error| format!("durable conversation store unavailable: {error}"))?;
    Ok(ChannelWorkflowState {
        ledger: Arc::new(ledger),
        conversations: Arc::new(conversations),
    })
}

/// The durable conversation services at one extension's storage roots.
///
/// Exposed for the pairing lane, which binds external actors into the *same*
/// durable conversation store the workflow resolves through — same roots, same
/// mount aliases, same construction as [`ChannelWorkflowFactory`] uses, so the
/// two cannot drift into two different stores. Returns the concrete
/// `ironclaw_conversations` handle deliberately: the only caller is
/// composition, which may name it, and narrowing it to a port here would fork
/// the store's identity.
pub async fn channel_conversation_services(
    filesystem: &Arc<dyn RootFilesystem>,
    roots: &ChannelWorkflowStorageRoots,
    ledger_scope: ResourceScope,
) -> Result<Arc<RebornFilesystemConversationServices>, String> {
    Ok(
        build_channel_workflow_state(filesystem, roots, ledger_scope)
            .await?
            .conversations,
    )
}

/// The durable per-extension workflow state, private to this module: the
/// conversations half must not leave it (see the module doc).
struct ChannelWorkflowState {
    ledger: Arc<dyn IdempotencyLedger>,
    conversations: Arc<RebornFilesystemConversationServices>,
}

#[async_trait]
impl ChannelWorkflowFactory for RebornChannelWorkflowFactory {
    async fn build_channel_workflow(
        &self,
        request: ChannelWorkflowRequest,
    ) -> Result<ChannelWorkflowGraph, String> {
        let identity = &self.services.identity;
        let extension_id = request.adapter_id.as_str().to_string();
        let ledger_scope = ResourceScope {
            tenant_id: identity.tenant_id.clone(),
            user_id: identity.operator_user_id.clone(),
            agent_id: Some(identity.agent_id.clone()),
            project_id: identity.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        };
        let workflow_state = build_channel_workflow_state(
            &self.services.filesystem,
            &request.storage_roots,
            ledger_scope,
        )
        .await?;

        let mut scope = ProductInstallationScope::with_default_scope(
            identity.tenant_id.clone(),
            identity.agent_id.clone(),
            identity.project_id.clone(),
        );
        // Generic shared-conversation admission (§5.3), fail-closed either
        // way: without a resolver every shared conversation is rejected; with
        // one, exactly the connected conversations are admitted. A run acts
        // as the user who invoked it, so there is no subject to default.
        if let Some(admission) = request.shared_admission {
            scope = scope.with_shared_conversation_admission(admission);
        }
        let scope = scope.with_actor_user_resolver(
            request.actor_user_resolver,
            Arc::clone(&workflow_state.conversations)
                as Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
        );
        let resolver = StaticProductInstallationResolver::new([(
            ProductInstallationKey::new(
                request.adapter_id.clone(),
                request.installation_id.clone(),
            ),
            scope,
        )]);
        let binding = Arc::new(ProductConversationBindingService::new(
            Arc::clone(&workflow_state.conversations)
                as Arc<dyn ironclaw_conversations::ConversationBindingService>,
            resolver,
        )) as Arc<dyn ProductBindingResolver>;

        let inbound = Arc::new(
            DefaultInboundTurnService::new(
                Arc::clone(&binding),
                Arc::clone(&self.services.thread_service),
                Arc::clone(&self.services.turn_coordinator),
                Arc::clone(&self.services.input_enqueue),
            )
            .with_inbound_attachments(Arc::clone(&self.services.inbound_attachments)),
        );
        let mut workflow = DefaultProductSurface::new(
            inbound,
            Arc::clone(&workflow_state.ledger),
            Arc::clone(&binding),
        );
        if let Some(approval) = &self.services.approval_interaction {
            workflow = workflow.with_approval_interaction_service(Arc::clone(approval));
        }
        if let Some(auth) = &self.services.auth_interaction {
            workflow = workflow.with_auth_interaction_service(Arc::clone(auth));
        }
        if let Some(delivery) = &self.services.delivery {
            workflow = workflow.with_delivered_gate_routes(Arc::clone(&delivery.route_store));
        }
        workflow = workflow
            .with_product_command_admission_service(Arc::new(
                DirectConversationCommandAdmission::new(
                    request.commands.iter().map(String::as_str),
                    request.command_roles,
                )
                .map_err(|error| {
                    format!("invalid command declaration for extension `{extension_id}`: {error}")
                })?,
            ))
            .with_product_command_surface(request.command_surface);

        let observer = match &self.services.delivery {
            Some(delivery) => Some(self.build_observer(
                &extension_id,
                delivery,
                Arc::clone(&binding),
                &request.commands,
                request.command_prefix.as_deref(),
                request.connection_notices,
            )?),
            None => None,
        };

        Ok(ChannelWorkflowGraph {
            surface: Arc::new(workflow) as Arc<dyn ChannelInboundProductSurface>,
            binding,
            observer,
        })
    }
}

impl RebornChannelWorkflowFactory {
    /// The generic run-delivery observer for one extension's live inbound
    /// conversations.
    fn build_observer(
        &self,
        extension_id: &str,
        delivery: &ChannelWorkflowDeliveryServices,
        binding: Arc<dyn ProductBindingResolver>,
        commands: &[String],
        command_prefix: Option<&str>,
        connection_notices: ironclaw_product_contracts::account_setup::ChannelConnectionNoticePolicy,
    ) -> Result<Arc<dyn ChannelRunDeliveryObserver>, String> {
        let notice_thread_id = ThreadId::new(format!("{extension_id}-channel-notices"))
            .map_err(|error| format!("invalid channel-notice thread id: {error}"))?;
        let services = RunDeliveryServices {
            project_filesystem: Arc::clone(&delivery.project_filesystem),
            binding_service: binding,
            thread_service: Arc::clone(&self.services.thread_service),
            turn_coordinator: Arc::clone(&self.services.turn_coordinator),
            outbound_store: Arc::clone(&delivery.outbound_store),
            route_store: Arc::clone(&delivery.route_store),
            communication_preferences: Arc::clone(&delivery.communication_preferences),
            delivery_targets: Arc::clone(&delivery.delivery_targets),
            coordinator: Arc::clone(&delivery.coordinator),
            extension_id: extension_id.to_string(),
            fallback_notice_scope: self.notice_scope(notice_thread_id),
            approval_context: delivery.approval_context.clone(),
            blocked_auth_prompts: delivery.blocked_auth_prompts.clone(),
            auth_flow_cancel: delivery.auth_flow_cancel.clone(),
        };
        Ok(Arc::new(
            RunDeliveryObserver::with_settings_and_connection_notices(
                services,
                delivery.settings,
                connection_notices,
            )
            .with_enabled_commands(commands.iter().map(String::as_str), command_prefix),
        ) as Arc<dyn ChannelRunDeliveryObserver>)
    }
}

/// No-op [`ProductBindingResolver`] for the triggered-delivery services:
/// the triggered path receives its `TurnScope` from the poller and resolves
/// its target from the creator's stored preference, so no binding is ever
/// resolved. This stub satisfies the type system without an unnecessary
/// installation-level conversation registry.
struct TriggeredNoopConversationBindingService;

#[async_trait]
impl ProductBindingResolver for TriggeredNoopConversationBindingService {
    async fn resolve_binding(
        &self,
        _request: ironclaw_product_contracts::binding::ResolveBindingRequest,
    ) -> Result<
        ironclaw_product_contracts::binding::ResolvedBinding,
        ironclaw_product_contracts::error::ProductOperationFailure,
    > {
        Err(
            ironclaw_product_contracts::error::ProductOperationFailure::BindingResolutionFailed {
                reason: "conversation bindings are not supported in triggered delivery".to_string(),
            },
        )
    }

    async fn lookup_binding(
        &self,
        _request: ironclaw_product_contracts::binding::ResolveBindingRequest,
    ) -> Result<
        ironclaw_product_contracts::binding::ResolvedBinding,
        ironclaw_product_contracts::error::ProductOperationFailure,
    > {
        Err(
            ironclaw_product_contracts::error::ProductOperationFailure::BindingResolutionFailed {
                reason: "conversation bindings are not supported in triggered delivery".to_string(),
            },
        )
    }
}
