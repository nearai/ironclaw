//! Generic per-extension channel host assembly (extension-runtime §5.3–§5.5).
//!
//! [`GenericChannelHostAssembly`] reconciles the [`ExtensionIngressRegistry`]
//! against manifest-declared deployment channels plus an active-snapshot
//! compatibility lane. Every discovered extension whose resolved contract
//! declares inbound channel ingress gets one registration — a dynamic
//! verification-secrets port over the manifest-declared administrator secrets
//! plus a [`GenericChannelInboundSink`] over per-extension ProductSurface
//! admission (durable idempotency ledger
//! and durable conversation binding at extension-keyed storage roots),
//! observed by the generic run-delivery observer when the composed runtime
//! has a delivery coordinator. Deployment registrations remain independent
//! of user activation; active-only registrations follow lifecycle changes.
//! Replacement is race-safe with in-flight requests (the registry swaps
//! `Arc` entries under one lock).
//!
//! Vendor residue that is not yet host-generic enters only through
//! [`ChannelExtras`]: the preference-target codec for the triggered delivery
//! driver and an optional shared-conversation route resolver.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use ironclaw_extension_host::active::{ActiveExtension, ActiveSnapshot};
use ironclaw_extension_host::ingress::{
    IngressPortError, IngressSecretsPort, VerificationCandidate,
};
use ironclaw_extension_host::{DeploymentChannelBinding, DeploymentChannelRegistry, SnapshotWatch};
use ironclaw_host_api::ChannelInboundProductSurface;
use ironclaw_host_api::recipe::IngressVerificationRecipe;
use ironclaw_host_api::{
    AgentId, ExtensionId, ProjectId, ResourceScope, SecretHandle, TenantId, ThreadId, UserId,
};
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveredGateRouteStore, OutboundStateStore,
};
use ironclaw_product::{
    AdapterInstallationId, ExternalConversationRef, ExternalEventId, ProductAdapterId,
    ProductInboundAck, ProductInboundEnvelope,
};
use ironclaw_product::{
    ApprovalInteractionService, ApprovalPromptContextSource, AuthInteractionService,
    BlockedAuthFlowCanceller, BlockedAuthPromptSource, ChannelConnectionNoticePolicy,
    ChannelPairingConsumeOutcome, ChannelPairingRegistry, ChannelWorkflowState,
    ChannelWorkflowStateService, ConversationBindingService, DefaultInboundTurnService,
    DefaultProductSurface, DeliveryCoordinator, PreferenceTargetCodec,
    ProductActorUserResolutionRequest, ProductActorUserResolver,
    ProductConversationSubjectRouteResolver, ProductInstallationKey, ProductInstallationScope,
    ProductWorkflowError, ResolvedProductActorUser, RunDeliveryObserver, RunDeliveryServices,
    StaticProductInstallationResolver, TriggeredRunDeliveryChannel,
};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::{TurnCoordinator, TurnScope};

use crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver;
use crate::extension_host::extension_ingress::{
    ChannelInboundSinkConfig, ChannelIngressDrain, ChannelIngressRegistration,
    ChannelPairingOutcomeObserver, ExtensionIngressRegistry, GenericChannelInboundSink,
    ManagedRegistrationOutcome, PostAdmissionObserver, VerifiedEvidenceMint,
};

/// Derive the trusted-evidence shape the generic inbound sink mints from the
/// resolved contract's ingress verification recipe — the mint mirrors the
/// recipe the generic router executed. `None` for `kind = "none"` recipes:
/// with no verification there is no trusted claim to mint, so the assembly
/// registers nothing and the route fails closed.
pub(crate) fn evidence_mint_for_verification(
    recipe: &IngressVerificationRecipe,
) -> Option<VerifiedEvidenceMint> {
    match recipe {
        IngressVerificationRecipe::HmacSha256(recipe) => {
            Some(VerifiedEvidenceMint::RequestSignature {
                signature_header: recipe.signature_header.clone(),
                timestamp_header: recipe.timestamp_header.clone(),
            })
        }
        IngressVerificationRecipe::SharedSecretHeader(recipe) => {
            Some(VerifiedEvidenceMint::SharedSecretHeader {
                header: recipe.header.clone(),
            })
        }
        IngressVerificationRecipe::None => None,
    }
}

/// Per-extension vendor ports that are not yet host-generic. Populated by
/// the extension's composition lane; a pure-manifest channel package
/// registers none.
pub(crate) struct ChannelExtras {
    /// The vendor half of the triggered-delivery driver; consumed by the
    /// lane that builds the triggered hook.
    pub(crate) preference_target_codec: Option<Arc<dyn PreferenceTargetCodec>>,
    /// Optional shared-channel subject-route resolver override. Absent, the
    /// assembly installs the DEFAULT generic resolver over the extension's
    /// `*_allowed_channels` / `*_subject_routes` administrator values
    /// when the manifest declares either handle.
    pub(crate) subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
}

/// The extras retained after registration.
#[derive(Clone, Default)]
struct StoredChannelExtras {
    preference_target_codec: Option<Arc<dyn PreferenceTargetCodec>>,
    subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
}

/// The deployment identity every per-extension workflow binds under: the
/// composed runtime's tenant/agent/project plus the operator user inbound
/// conversations default their subject to.
#[derive(Clone)]
pub struct ChannelHostIdentity {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub operator_user_id: UserId,
}

/// The outbound-delivery half of the assembly's dependencies. Absent when
/// the composed runtime has no delivery coordinator — registrations are
/// then ingress-only (turns run; no channel lifecycle output is delivered).
pub(crate) struct ChannelHostDeliveryDeps {
    pub(crate) coordinator: Arc<DeliveryCoordinator>,
    pub(crate) outbound_store: Arc<dyn OutboundStateStore>,
    pub(crate) route_store: Arc<dyn DeliveredGateRouteStore>,
    pub(crate) communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub(crate) current_delivery_targets: Arc<dyn ironclaw_product::CurrentDeliveryTargetResolver>,
    pub(crate) approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
    pub(crate) blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    pub(crate) auth_flow_cancel: Option<Arc<dyn BlockedAuthFlowCanceller>>,
    pub(crate) event_router: Arc<ironclaw_product::RunDeliveryEventRouter>,
}

/// Everything the assembly composes per-extension graphs from.
pub(crate) struct GenericChannelHostDeps {
    pub(crate) watch: SnapshotWatch,
    pub(crate) deployment_channels: Arc<DeploymentChannelRegistry>,
    pub(crate) registry: Arc<ExtensionIngressRegistry>,
    pub(crate) admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
    pub(crate) workflow_state: Arc<ChannelWorkflowStateService>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) turn_coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) approval_interaction: Option<Arc<dyn ApprovalInteractionService>>,
    pub(crate) auth_interaction: Option<Arc<dyn AuthInteractionService>>,
    pub(crate) identity: ChannelHostIdentity,
    /// The generic channel-identity binding store: verified inbound actors
    /// on auth-declaring channel extensions resolve through it. `None`
    /// (composition paths without the durable store) falls back to the
    /// operator-actor policy.
    pub(crate) identity_lookup: Option<Arc<dyn crate::provider_identity::RebornUserIdentityLookup>>,
    pub(crate) delivery: Option<ChannelHostDeliveryDeps>,
    /// Pairing services for `WebGeneratedCode` channel extensions: drives the
    /// sink's pre-admission consume gate and identity-based actor resolution
    /// for extensions that pair without an OAuth vendor.
    pub(crate) channel_pairing: Option<Arc<ChannelPairingRegistry>>,
}

/// What the assembly last reconciled for one extension id.
enum ReconciledChannel {
    /// Assembly-built generic graph for exactly this channel source.
    Generic {
        source: HostedChannelSource,
        #[cfg(feature = "test-support")]
        conversation_binding: Arc<dyn ConversationBindingService>,
        /// The post-admission observer registered with the sink (test seam:
        /// gate-resolution acks arriving from non-channel surfaces are
        /// injected through the SAME observer instance the sink drives).
        #[cfg(feature = "test-support")]
        observer: Option<Arc<dyn PostAdmissionObserver>>,
    },
    /// Nothing registered for this entry (unmanaged registration, no
    /// verification recipe, or a build failure already logged); skipped
    /// until the active-set entry changes.
    Untouched { source: HostedChannelSource },
}

#[derive(Clone)]
enum HostedChannelSource {
    Deployment(Arc<DeploymentChannelBinding>),
    Active(Arc<ActiveExtension>),
}

impl HostedChannelSource {
    fn extension_id(&self) -> &str {
        match self {
            Self::Deployment(binding) => &binding.extension_id,
            Self::Active(active) => &active.extension_id,
        }
    }

    fn installation_id(&self) -> &str {
        match self {
            // Deployment ingress is not a user installation. Its stable
            // host-owned identity is the extension id itself.
            Self::Deployment(binding) => &binding.extension_id,
            Self::Active(active) => &active.installation_id,
        }
    }

    fn resolved(&self) -> &ironclaw_extensions::ResolvedExtensionManifest {
        match self {
            Self::Deployment(binding) => binding.resolved.as_ref(),
            Self::Active(active) => active.resolved.as_ref(),
        }
    }

    fn same_source(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Deployment(left), Self::Deployment(right)) => Arc::ptr_eq(left, right),
            (Self::Active(left), Self::Active(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

struct BuiltGenericChannelGraph {
    registration: ChannelIngressRegistration,
    #[cfg(feature = "test-support")]
    conversation_binding: Arc<dyn ConversationBindingService>,
    #[cfg(feature = "test-support")]
    observer: Option<Arc<dyn PostAdmissionObserver>>,
}

/// The generic channel host assembly: one per composed runtime with a
/// generic extension host. Owns a reconcile loop over the snapshot watch.
pub struct GenericChannelHostAssembly {
    deps: GenericChannelHostDeps,
    extras: StdMutex<HashMap<String, StoredChannelExtras>>,
    reconciled: tokio::sync::Mutex<HashMap<String, ReconciledChannel>>,
    reconcile_loop: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl GenericChannelHostAssembly {
    /// Build the assembly, reconcile the current snapshot, and spawn the
    /// watch loop. The loop holds only a weak handle: dropping the returned
    /// `Arc` ends it (and `Drop` aborts it eagerly).
    pub(crate) fn start(deps: GenericChannelHostDeps) -> Arc<Self> {
        let mut subscription = deps.watch.subscribe();
        let assembly = Arc::new(Self {
            deps,
            extras: StdMutex::new(HashMap::new()),
            reconciled: tokio::sync::Mutex::new(HashMap::new()),
            reconcile_loop: StdMutex::new(None),
        });
        let weak = Arc::downgrade(&assembly);
        let handle = tokio::spawn(async move {
            loop {
                {
                    let Some(assembly) = weak.upgrade() else {
                        break;
                    };
                    let snapshot = assembly.deps.watch.current();
                    assembly.reconcile(snapshot).await;
                }
                if subscription.changed().await.is_err() {
                    break;
                }
            }
        });
        if let Ok(mut slot) = assembly.reconcile_loop.lock() {
            *slot = Some(handle);
        }
        assembly
    }

    /// Register one extension's vendor extras, then re-reconcile the
    /// extension against the current snapshot so vendor extras apply to the
    /// next build.
    pub(crate) async fn register_extras(&self, extension_id: &str, extras: ChannelExtras) {
        let ChannelExtras {
            preference_target_codec,
            subject_route_resolver,
        } = extras;
        if let Ok(mut stored) = self.extras.lock() {
            stored.insert(
                extension_id.to_string(),
                StoredChannelExtras {
                    preference_target_codec,
                    subject_route_resolver,
                },
            );
        }
        let mut reconciled = self.reconciled.lock().await;
        reconciled.remove(extension_id);
        drop(reconciled);
        self.reconcile(self.deps.watch.current()).await;
    }

    /// The registered preference-target codec for one extension, if any —
    /// the triggered-delivery lane resolves its vendor codec through this.
    pub(crate) fn preference_target_codec(
        &self,
        extension_id: &str,
    ) -> Option<Arc<dyn PreferenceTargetCodec>> {
        self.extras
            .lock()
            .ok()?
            .get(extension_id)?
            .preference_target_codec
            .clone()
    }

    /// The snapshot watch the assembly reconciles over — shared with the
    /// generic outbound-target provider so both read the same active set.
    pub(crate) fn snapshot_watch(&self) -> SnapshotWatch {
        self.deps.watch.clone()
    }

    /// The deployment identity per-extension workflows bind under.
    pub(crate) fn identity(&self) -> &ChannelHostIdentity {
        &self.deps.identity
    }

    /// Every ACTIVE channel extension with a registered preference-target
    /// codec, in extension-id order — the generic triggered-delivery hook
    /// routes stored preference refs across these.
    pub(crate) fn active_preference_codecs(&self) -> Vec<(String, Arc<dyn PreferenceTargetCodec>)> {
        let snapshot = self.deps.watch.current();
        snapshot
            .extension_ids()
            .into_iter()
            .filter(|extension_id| {
                snapshot
                    .extension(extension_id)
                    .is_some_and(|active| active.resolved.channel.is_some())
            })
            .filter_map(|extension_id| {
                self.preference_target_codec(&extension_id)
                    .map(|codec| (extension_id, codec))
            })
            .collect()
    }

    /// Currently assembled channel lanes for product-owned triggered-run
    /// routing. Composition contributes dependencies only; it does not choose
    /// a target or define fallback/failure semantics.
    pub(crate) fn active_triggered_delivery_channels(&self) -> Vec<TriggeredRunDeliveryChannel> {
        self.active_preference_codecs()
            .into_iter()
            .filter_map(|(extension_id, preference_target_codec)| {
                self.triggered_run_delivery_services(&extension_id)
                    .map(|services| TriggeredRunDeliveryChannel {
                        preference_target_codec,
                        services,
                    })
            })
            .collect()
    }

    /// Generic run-delivery services for the triggered-delivery driver on
    /// one extension. Binding-free: the triggered path resolves its target
    /// from the creator's stored preference, never from a conversation
    /// binding. `None` when the composed runtime has no delivery
    /// coordinator (nothing can deliver).
    pub(crate) fn triggered_run_delivery_services(
        &self,
        extension_id: &str,
    ) -> Option<RunDeliveryServices> {
        let delivery = self.deps.delivery.as_ref()?;
        let identity = &self.deps.identity;
        let notice_thread_id = match ThreadId::new(format!("{extension_id}-channel-notices")) {
            Ok(thread_id) => thread_id,
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_host",
                    extension_id,
                    %error,
                    "invalid channel-notice thread id; triggered delivery unavailable"
                );
                return None;
            }
        };
        let fallback_notice_scope = TurnScope::new_with_owner(
            identity.tenant_id.clone(),
            Some(identity.agent_id.clone()),
            identity.project_id.clone(),
            notice_thread_id,
            Some(identity.operator_user_id.clone()),
        );
        Some(RunDeliveryServices {
            binding_service: Arc::new(TriggeredNoopConversationBindingService),
            thread_service: Arc::clone(&self.deps.thread_service),
            turn_coordinator: Arc::clone(&self.deps.turn_coordinator),
            outbound_store: Arc::clone(&delivery.outbound_store),
            route_store: Arc::clone(&delivery.route_store),
            communication_preferences: Arc::clone(&delivery.communication_preferences),
            coordinator: Arc::clone(&delivery.coordinator),
            extension_id: extension_id.to_string(),
            fallback_notice_scope,
            approval_context: delivery.approval_context.clone(),
            blocked_auth_prompts: delivery.blocked_auth_prompts.clone(),
            auth_flow_cancel: delivery.auth_flow_cancel.clone(),
        })
    }

    fn stored_extras(&self, extension_id: &str) -> StoredChannelExtras {
        self.extras
            .lock()
            .ok()
            .and_then(|stored| stored.get(extension_id).cloned())
            .unwrap_or_default()
    }

    /// Reconcile deployment-owned channels plus the active-snapshot
    /// compatibility set. Deployment bindings win for the same extension id,
    /// so user install/deactivation never removes an operator-owned route.
    async fn reconcile(&self, snapshot: Arc<ActiveSnapshot>) {
        let mut reconciled = self.reconciled.lock().await;

        let mut desired: BTreeMap<String, HostedChannelSource> = BTreeMap::new();
        for extension_id in self.deps.deployment_channels.extension_ids() {
            if let Some(binding) = self.deps.deployment_channels.extension(&extension_id) {
                desired.insert(extension_id, HostedChannelSource::Deployment(binding));
            }
        }
        for extension_id in snapshot.extension_ids() {
            if let Some(active) = snapshot.extension(&extension_id)
                && let Some(channel) = active.resolved.channel.as_ref()
                && channel.inbound
                && channel.ingress.is_some()
            {
                desired
                    .entry(extension_id)
                    .or_insert(HostedChannelSource::Active(active));
            }
        }

        let removed_sources: Vec<String> = reconciled
            .iter()
            .filter(|(extension_id, _)| !desired.contains_key(*extension_id))
            .map(|(extension_id, _)| extension_id.clone())
            .collect();
        for extension_id in removed_sources {
            if let Some(ReconciledChannel::Generic { .. }) = reconciled.remove(&extension_id)
                && let Some(removed) = self.deps.registry.unregister_managed(&extension_id)
            {
                spawn_drain(removed.drain.clone());
            }
        }

        for (extension_id, source) in desired {
            match reconciled.get(&extension_id) {
                Some(ReconciledChannel::Generic {
                    source: previous, ..
                })
                | Some(ReconciledChannel::Untouched { source: previous })
                    if previous.same_source(&source) =>
                {
                    continue;
                }
                _ => {}
            }
            let extras = self.stored_extras(&extension_id);
            match self.build_generic_graph(&source, &extras).await {
                Ok(Some(graph)) => {
                    match self
                        .deps
                        .registry
                        .register_managed(&extension_id, graph.registration)
                    {
                        ManagedRegistrationOutcome::Registered { replaced } => {
                            if let Some(replaced) = replaced {
                                spawn_drain(replaced.drain.clone());
                            }
                            reconciled.insert(
                                extension_id.clone(),
                                ReconciledChannel::Generic {
                                    source,
                                    #[cfg(feature = "test-support")]
                                    conversation_binding: graph.conversation_binding,
                                    #[cfg(feature = "test-support")]
                                    observer: graph.observer,
                                },
                            );
                        }
                        ManagedRegistrationOutcome::SkippedUnmanaged => {
                            reconciled.insert(
                                extension_id.clone(),
                                ReconciledChannel::Untouched { source },
                            );
                        }
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        target = "ironclaw::reborn::channel_host",
                        extension_id = %extension_id,
                        "active channel declares no verifiable ingress; nothing registered"
                    );
                    reconciled.insert(
                        extension_id.clone(),
                        ReconciledChannel::Untouched { source },
                    );
                }
                Err(reason) => {
                    tracing::warn!(
                        target = "ironclaw::reborn::channel_host",
                        extension_id = %extension_id,
                        %reason,
                        "channel ingress graph could not be built; route fails closed"
                    );
                    reconciled.insert(
                        extension_id.clone(),
                        ReconciledChannel::Untouched { source },
                    );
                }
            }
        }
    }

    /// Build one extension's generic inbound graph: dynamic verification
    /// secrets over the administrator configuration service, per-extension
    /// ProductSurface admission, and (with a coordinator) the run-delivery
    /// observer.
    async fn build_generic_graph(
        &self,
        source: &HostedChannelSource,
        extras: &StoredChannelExtras,
    ) -> Result<Option<BuiltGenericChannelGraph>, String> {
        let Some(channel) = source.resolved().channel.as_ref() else {
            return Ok(None);
        };
        let Some(ingress) = channel.ingress.as_ref() else {
            return Ok(None);
        };
        let Some(evidence) = evidence_mint_for_verification(&ingress.verification) else {
            return Ok(None);
        };
        let Some(secret_handle) = ingress.verification.secret_handle() else {
            return Ok(None);
        };

        let secrets = Arc::new(AdminConfigurationIngressSecrets {
            admin_configuration_resolver: Arc::clone(&self.deps.admin_configuration_resolver),
            extension_id: ExtensionId::new(source.extension_id())
                .map_err(|error| format!("invalid extension id: {error}"))?,
            handle: secret_handle.clone(),
            installation_id: source.installation_id().to_string(),
        });

        let (binding, workflow_state) = self.build_binding(source, extras).await?;

        let inbound = Arc::new(DefaultInboundTurnService::new(
            Arc::clone(&binding),
            Arc::clone(&self.deps.thread_service),
            Arc::clone(&self.deps.turn_coordinator),
        ));
        let mut workflow = DefaultProductSurface::new(
            inbound,
            Arc::clone(&workflow_state.ledger),
            Arc::clone(&binding),
        );
        if let Some(approval) = &self.deps.approval_interaction {
            workflow = workflow.with_approval_interaction_service(Arc::clone(approval));
        }
        if let Some(auth) = &self.deps.auth_interaction {
            workflow = workflow.with_auth_interaction_service(Arc::clone(auth));
        }
        if let Some(delivery) = &self.deps.delivery {
            workflow = workflow.with_delivered_gate_routes(Arc::clone(&delivery.route_store));
        }

        let observer = match &self.deps.delivery {
            Some(delivery) => Some(self.build_observer(source, delivery, Arc::clone(&binding))?),
            None => None,
        };

        let adapter_id = ProductAdapterId::new(source.extension_id())
            .map_err(|error| format!("invalid adapter id: {error}"))?;
        let pairing = self
            .deps
            .channel_pairing
            .as_ref()
            .and_then(|registry| registry.get(source.extension_id()))
            .map(|service| service as Arc<dyn ironclaw_product::ChannelPairingInterceptor>);
        let surface = Arc::new(workflow) as Arc<dyn ChannelInboundProductSurface>;
        let mut sink = GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id,
            evidence,
            surface,
            observer: observer
                .clone()
                .map(|observer| observer as Arc<dyn PostAdmissionObserver>),
        });
        if let Some(pairing) = pairing {
            sink = sink.with_pairing(
                pairing,
                observer
                    .clone()
                    .map(ChannelPairingOutcomeObserver::RunDelivery),
            );
        }
        let sink = Arc::new(sink);
        let registration = ChannelIngressRegistration {
            secrets,
            sink: Arc::clone(&sink) as Arc<dyn ironclaw_extension_host::ingress::InboundSink>,
            drain: Some(sink as Arc<dyn ChannelIngressDrain>),
        };
        Ok(Some(BuiltGenericChannelGraph {
            registration,
            #[cfg(feature = "test-support")]
            conversation_binding: binding,
            #[cfg(feature = "test-support")]
            observer: observer.map(|observer| observer as Arc<dyn PostAdmissionObserver>),
        }))
    }

    /// The per-extension conversation-binding service over durable state at
    /// the extension's storage roots, bound under the deployment identity.
    async fn build_binding(
        &self,
        source: &HostedChannelSource,
        extras: &StoredChannelExtras,
    ) -> Result<(Arc<dyn ConversationBindingService>, ChannelWorkflowState), String> {
        let identity = &self.deps.identity;
        let ledger_scope = ResourceScope {
            tenant_id: identity.tenant_id.clone(),
            user_id: identity.operator_user_id.clone(),
            agent_id: Some(identity.agent_id.clone()),
            project_id: identity.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let extension_id = ExtensionId::new(source.extension_id())
            .map_err(|error| format!("invalid extension id: {error}"))?;
        let workflow_state = self
            .deps
            .workflow_state
            .build_for_extension(&extension_id, ledger_scope)
            .await
            .map_err(|error| error.to_string())?;

        let adapter_id = ProductAdapterId::new(source.extension_id())
            .map_err(|error| format!("invalid adapter id: {error}"))?;
        let installation_id = AdapterInstallationId::new(source.installation_id())
            .map_err(|error| format!("invalid installation id: {error}"))?;
        // Auth-declaring channel extensions resolve verified inbound actors
        // through the generic installation-scoped identity bindings written
        // by the post-OAuth channel-identity hook; unbound actors fall to
        // the pairing service (fail-closed pairing flow). Extensions without
        // an auth vendor keep the operator-actor policy: the ingress
        // verification secret gates who reaches the installation and no
        // binding can exist to resolve.
        let pairing_service = self
            .deps
            .channel_pairing
            .as_ref()
            .and_then(|registry| registry.get(source.extension_id()));
        let actor_user_resolver: Arc<dyn ProductActorUserResolver> = match (
            self.deps.identity_lookup.as_ref(),
            source.resolved().auth.first(),
            pairing_service,
        ) {
            (Some(lookup), Some(auth), _) => Arc::new(
                crate::provider_identity::ProviderIdentityActorResolver::for_any_actor_kind(
                    auth.vendor.as_str(),
                    source.extension_id(),
                    Arc::clone(lookup),
                ),
            ),
            // Pairing-strategy channels have no OAuth vendor; verified
            // inbound actors resolve through the bindings the pairing
            // consume wrote, keyed by the extension id as provider. Unbound
            // actors fail closed instead of inheriting the operator. The
            // pairing service also returns its durable binding epoch so the
            // conversation layer preserves exact-generation unpair fencing.
            (_, None, Some(pairing)) => pairing,
            _ => Arc::new(OperatorActorUserResolver {
                operator_user_id: identity.operator_user_id.clone(),
            }),
        };
        let mut scope = ProductInstallationScope::with_default_scope(
            identity.tenant_id.clone(),
            identity.agent_id.clone(),
            identity.project_id.clone(),
        )
        .with_default_subject_user_id(identity.operator_user_id.clone());
        // Generic shared-channel admission (§5.3): with a subject-route
        // resolver installed, unrouted shared conversations fail closed —
        // an extras override wins; otherwise a manifest declaring the
        // `*_allowed_channels` / `*_subject_routes` administrator convention
        // gets the default resolver over those values.
        let subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>> =
            match &extras.subject_route_resolver {
                Some(resolver) => Some(Arc::clone(resolver)),
                None => {
                    let handles = crate::extension_host::channel_subject_routes::
                        shared_channel_admission_handles(
                            &source.resolved().admin_configuration,
                        );
                    if handles.declared() {
                        let extension_id = ExtensionId::new(source.extension_id())
                            .map_err(|error| format!("invalid extension id: {error}"))?;
                        Some(Arc::new(
                            crate::extension_host::channel_subject_routes::
                                AdminConfigurationSubjectRouteResolver::new(
                                    adapter_id.clone(),
                                    installation_id.clone(),
                                    identity.tenant_id.clone(),
                                    extension_id,
                                    handles,
                                    Arc::clone(&self.deps.admin_configuration_resolver),
                                ),
                        ))
                    } else {
                        None
                    }
                }
            };
        if let Some(resolver) = subject_route_resolver {
            scope = scope
                .with_conversation_subject_route_resolver(resolver)
                .without_default_subject_for_unrouted_shared_conversations();
        }
        let scope = scope.with_actor_user_resolver(
            actor_user_resolver,
            Arc::clone(&workflow_state.conversations)
                as Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
        );
        let resolver = StaticProductInstallationResolver::new([(
            ProductInstallationKey::new(adapter_id, installation_id),
            scope,
        )]);
        let conversations: Arc<dyn ironclaw_conversations::ConversationBindingService> =
            Arc::clone(&workflow_state.conversations)
                as Arc<dyn ironclaw_conversations::ConversationBindingService>;
        let binding =
            ironclaw_product::ProductConversationBindingService::new(conversations, resolver);
        Ok((
            Arc::new(binding) as Arc<dyn ConversationBindingService>,
            workflow_state,
        ))
    }

    /// The generic run-delivery observer for one extension's live inbound
    /// conversations, adapted onto the sink's post-admission seam.
    fn build_observer(
        &self,
        source: &HostedChannelSource,
        delivery: &ChannelHostDeliveryDeps,
        binding: Arc<dyn ConversationBindingService>,
    ) -> Result<Arc<RunDeliveryPostAdmissionObserver>, String> {
        let identity = &self.deps.identity;
        let notice_thread_id = ThreadId::new(format!(
            "{extension_id}-channel-notices",
            extension_id = source.extension_id()
        ))
        .map_err(|error| format!("invalid channel-notice thread id: {error}"))?;
        let fallback_notice_scope = TurnScope::new_with_owner(
            identity.tenant_id.clone(),
            Some(identity.agent_id.clone()),
            identity.project_id.clone(),
            notice_thread_id,
            Some(identity.operator_user_id.clone()),
        );
        let services = RunDeliveryServices {
            binding_service: binding,
            thread_service: Arc::clone(&self.deps.thread_service),
            turn_coordinator: Arc::clone(&self.deps.turn_coordinator),
            outbound_store: Arc::clone(&delivery.outbound_store),
            route_store: Arc::clone(&delivery.route_store),
            communication_preferences: Arc::clone(&delivery.communication_preferences),
            coordinator: Arc::clone(&delivery.coordinator),
            extension_id: source.extension_id().to_string(),
            fallback_notice_scope,
            approval_context: delivery.approval_context.clone(),
            blocked_auth_prompts: delivery.blocked_auth_prompts.clone(),
            auth_flow_cancel: delivery.auth_flow_cancel.clone(),
        };
        let connection_notices = self
            .deps
            .channel_pairing
            .as_ref()
            .and_then(|registry| registry.get(source.extension_id()))
            .map(|service| service.connection_notices().clone())
            .unwrap_or_else(|| ChannelConnectionNoticePolicy::generic(&source.resolved().name));
        let observer = Arc::new(RunDeliveryObserver::with_connection_notices(
            services.clone(),
            connection_notices.clone(),
        ));
        let mut event_handler = ironclaw_product::RunDeliveryEventHandler::new(
            services,
            source.extension_id(),
            source.installation_id(),
        );
        if self
            .preference_target_codec(source.extension_id())
            .is_some()
        {
            event_handler = event_handler
                .with_current_target_resolver(Arc::clone(&delivery.current_delivery_targets));
        }
        let event_handler = Arc::new(event_handler);
        delivery
            .event_router
            .register(source.extension_id(), &event_handler);
        Ok(Arc::new(RunDeliveryPostAdmissionObserver {
            observer,
            connection_notices,
            event_handler,
            event_router: Arc::clone(&delivery.event_router),
        }))
    }

    /// The live conversation-binding service the assembly registered for one
    /// extension — the SAME instance the registered sink resolves through,
    /// so a pre-resolved binding is the binding admission finds.
    #[cfg(feature = "test-support")]
    pub fn binding_service_for_extension_for_test(
        &self,
        extension_id: &str,
    ) -> Option<Arc<dyn ConversationBindingService>> {
        let reconciled = self.reconciled.try_lock().ok()?;
        match reconciled.get(extension_id)? {
            ReconciledChannel::Generic {
                conversation_binding,
                ..
            } => Some(Arc::clone(conversation_binding)),
            _ => None,
        }
    }

    /// The post-admission observer the assembly registered for one extension
    /// — the SAME instance the registered sink drives, so an ack injected
    /// from a non-channel surface (WebUI gate resolve) exercises the exact
    /// single-flight guard production runs.
    #[cfg(feature = "test-support")]
    pub fn post_admission_observer_for_extension_for_test(
        &self,
        extension_id: &str,
    ) -> Option<Arc<dyn PostAdmissionObserver>> {
        let reconciled = self.reconciled.try_lock().ok()?;
        match reconciled.get(extension_id)? {
            ReconciledChannel::Generic { observer, .. } => observer.clone(),
            _ => None,
        }
    }
}

impl Drop for GenericChannelHostAssembly {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.reconcile_loop.lock()
            && let Some(handle) = slot.take()
        {
            handle.abort();
        }
    }
}

fn spawn_drain(drain: Option<Arc<dyn ChannelIngressDrain>>) {
    if let Some(drain) = drain {
        tokio::spawn(async move {
            drain.drain().await;
        });
    }
}

/// No-op [`ConversationBindingService`] for the triggered-delivery
/// services: the triggered path receives its sealed `TurnScope` from the
/// trigger submission hook and resolves its target from current caller-scoped
/// outbound-target state, so no
/// binding is ever resolved. This stub satisfies the type system without
/// an unnecessary installation-level conversation registry.
struct TriggeredNoopConversationBindingService;

#[async_trait]
impl ConversationBindingService for TriggeredNoopConversationBindingService {
    async fn resolve_binding(
        &self,
        _request: ironclaw_product::ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "conversation bindings are not supported in triggered delivery".to_string(),
        })
    }

    async fn lookup_binding(
        &self,
        _request: ironclaw_product::ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "conversation bindings are not supported in triggered delivery".to_string(),
        })
    }
}

/// The generic actor policy for per-extension channel workflows: every
/// verified inbound actor resolves to the deployment's operator user (the
/// ingress verification secret gates who can reach the installation).
struct OperatorActorUserResolver {
    operator_user_id: UserId,
}

#[async_trait]
impl ProductActorUserResolver for OperatorActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        _request: ProductActorUserResolutionRequest,
    ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
        Ok(Some(ResolvedProductActorUser::new(
            self.operator_user_id.clone(),
        )))
    }
}

/// Dynamic verification-secrets port over administrator configuration
/// storage: the manifest-declared `verification.secret_handle` is resolved
/// per request, so a configure save takes effect on the next webhook with
/// no route rebuild. No stored secret -> no candidates -> the generic
/// router rejects 401.
struct AdminConfigurationIngressSecrets {
    admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
    extension_id: ExtensionId,
    handle: SecretHandle,
    installation_id: String,
}

#[async_trait]
impl IngressSecretsPort for AdminConfigurationIngressSecrets {
    async fn verification_candidates(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        _handle: Option<&SecretHandle>,
    ) -> Result<Vec<VerificationCandidate>, IngressPortError> {
        let material = self
            .admin_configuration_resolver
            .secret_material(&self.extension_id, &self.handle)
            .await
            .map_err(|error| IngressPortError {
                reason: format!("channel verification secret unavailable: {error}"),
            })?;
        Ok(match material {
            Some(material) => vec![VerificationCandidate {
                installation_id: self.installation_id.clone(),
                secret: secrecy::ExposeSecret::expose_secret(&material)
                    .as_bytes()
                    .to_vec(),
            }],
            None => Vec::new(),
        })
    }
}

/// Adapts the generic run-delivery observer onto the generic sink's
/// post-admission observer seam.
pub(super) struct RunDeliveryPostAdmissionObserver {
    observer: Arc<RunDeliveryObserver>,
    connection_notices: ChannelConnectionNoticePolicy,
    // Strong ownership keeps the router's weak registration live for exactly
    // as long as this reconciled channel graph remains installed.
    event_handler: Arc<ironclaw_product::RunDeliveryEventHandler>,
    event_router: Arc<ironclaw_product::RunDeliveryEventRouter>,
}

#[async_trait]
impl PostAdmissionObserver for RunDeliveryPostAdmissionObserver {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        self.observer
            .observe_ack(envelope.clone(), ack.clone())
            .await;
        if let Err(error) = self
            .event_handler
            .reconcile_accepted_user_message(self.event_router.as_ref(), &envelope, &ack)
            .await
        {
            tracing::debug!(
                target = "ironclaw::reborn::run_delivery",
                %error,
                "post-admission lifecycle reconciliation was not applicable"
            );
        }
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_product::ProductAdapterError,
    ) {
        self.observer.observe_error(envelope, error).await;
    }
}

impl RunDeliveryPostAdmissionObserver {
    pub(super) async fn observe_pairing_outcome(
        &self,
        conversation: ExternalConversationRef,
        event_id: ExternalEventId,
        outcome: ChannelPairingConsumeOutcome,
    ) {
        let text = match outcome {
            ChannelPairingConsumeOutcome::Paired { .. } => &self.connection_notices.paired,
            ChannelPairingConsumeOutcome::AlreadyPairedSameUser { .. } => {
                &self.connection_notices.already_paired_same_user
            }
            ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser => {
                &self.connection_notices.already_bound_to_other_user
            }
            ChannelPairingConsumeOutcome::ExpiredOrUnknown => {
                &self.connection_notices.expired_or_unknown
            }
        };
        self.observer
            .post_connection_status_notice(&conversation, &event_id, text)
            .await;
    }
}

// Lives under `channel_host/tests/` (the repo's test-only path convention the
// no-panics checker recognizes); `#[path]` keeps the module name and its
// `super::` references unchanged.
#[cfg(test)]
#[path = "channel_host/tests/e2e_tests.rs"]
mod e2e_tests;

#[cfg(test)]
mod tests {
    use ironclaw_host_api::recipe::{
        HmacSha256VerificationRecipe, SharedSecretHeaderRecipe, SignatureEncoding,
        SignedPayloadSegment,
    };

    use super::*;

    fn handle(value: &str) -> SecretHandle {
        SecretHandle::new(value).expect("valid secret handle")
    }

    #[test]
    fn hmac_recipe_mints_a_request_signature_claim() {
        let recipe = IngressVerificationRecipe::HmacSha256(HmacSha256VerificationRecipe {
            secret_handle: handle("vendorx_signing_secret"),
            signature_header: "X-VendorX-Signature".to_string(),
            signature_prefix: Some("v0=".to_string()),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_header: Some("X-VendorX-Timestamp".to_string()),
            max_age_seconds: Some(300),
            signed_payload: vec![SignedPayloadSegment::Body { body: true }],
        });
        match evidence_mint_for_verification(&recipe) {
            Some(VerifiedEvidenceMint::RequestSignature {
                signature_header,
                timestamp_header,
            }) => {
                assert_eq!(signature_header, "X-VendorX-Signature");
                assert_eq!(timestamp_header.as_deref(), Some("X-VendorX-Timestamp"));
            }
            other => panic!("expected a request-signature mint, got {other:?}"),
        }
    }

    #[test]
    fn hmac_recipe_without_timestamp_mints_no_timestamp_header() {
        let recipe = IngressVerificationRecipe::HmacSha256(HmacSha256VerificationRecipe {
            secret_handle: handle("vendorx_signing_secret"),
            signature_header: "X-VendorX-Signature".to_string(),
            signature_prefix: None,
            signature_encoding: SignatureEncoding::Hex,
            timestamp_header: None,
            max_age_seconds: None,
            signed_payload: vec![SignedPayloadSegment::Body { body: true }],
        });
        match evidence_mint_for_verification(&recipe) {
            Some(VerifiedEvidenceMint::RequestSignature {
                timestamp_header, ..
            }) => assert!(timestamp_header.is_none()),
            other => panic!("expected a request-signature mint, got {other:?}"),
        }
    }

    #[test]
    fn shared_secret_recipe_mints_a_shared_secret_header_claim() {
        let recipe = IngressVerificationRecipe::SharedSecretHeader(SharedSecretHeaderRecipe {
            secret_handle: handle("vendorx_webhook_secret"),
            header: "X-VendorX-Secret".to_string(),
        });
        match evidence_mint_for_verification(&recipe) {
            Some(VerifiedEvidenceMint::SharedSecretHeader { header }) => {
                assert_eq!(header, "X-VendorX-Secret");
            }
            other => panic!("expected a shared-secret-header mint, got {other:?}"),
        }
    }

    #[test]
    fn none_recipe_mints_nothing() {
        assert!(evidence_mint_for_verification(&IngressVerificationRecipe::None).is_none());
    }
}
