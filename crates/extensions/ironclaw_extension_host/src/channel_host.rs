//! Generic per-extension channel host assembly (extension-runtime §5.3–§5.5).
//!
//! [`GenericChannelHostAssembly`] reconciles the [`ExtensionIngressRegistry`]
//! against manifest-declared deployment channels plus an active-snapshot
//! compatibility lane. Every discovered extension whose resolved contract
//! declares inbound channel ingress gets one registration — a dynamic
//! verification-secrets port over the
//! `[channel.config]` secret storage plus a [`GenericChannelInboundSink`]
//! over per-extension ProductSurface admission (durable idempotency ledger
//! and durable conversation binding at extension-keyed storage roots),
//! observed by the generic run-delivery observer when the composed runtime
//! has a delivery coordinator. Deployment registrations remain independent
//! of user activation; active-only registrations follow lifecycle changes.
//! Replacement is race-safe with in-flight requests (the registry swaps
//! `Arc` entries under one lock).
//!
//! Vendor residue that is not yet host-generic enters only through
//! [`ChannelExtras`]: the preference-target codec for the triggered delivery
//! driver and an optional storage-root override for a channel whose durable
//! state predates the generic root scheme.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use ironclaw_extension_contracts::channel::{
    ChannelConnectionDescriptor, ChannelConnectionStrategy,
};
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_extension_contracts::external::{ExternalConversationRef, ExternalEventId};
use ironclaw_extension_contracts::preference_target::PreferenceTargetCodec;
use ironclaw_extension_contracts::recipe::IngressVerificationRecipe;
use ironclaw_extension_host::active::{ActiveExtension, ActiveSnapshot};
use ironclaw_extension_host::ingress::{
    IngressConfigurationPort, IngressPortError, IngressSecretsPort, VerificationCandidate,
};
use ironclaw_extension_host::{DeploymentChannelBinding, DeploymentChannelRegistry, SnapshotWatch};
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
use ironclaw_host_api::{
    ids::{AgentId, ExtensionId, ProjectId, SecretHandle, TenantId, UserId},
    path::VirtualPath,
};
use ironclaw_product_contracts::account_setup::ChannelConnectionNoticePolicy;
use ironclaw_product_contracts::actor_identity::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ResolvedProductActorUser,
};
// Only the test seam below names the binding port: production hands the
// factory-built service straight into the sink without holding it.
#[cfg(any(test, feature = "test-support"))]
use ironclaw_product_contracts::binding::ProductBindingResolver;
use ironclaw_product_contracts::channel_workflow::{
    ChannelRunDeliveryObserver, ChannelWorkflowFactory, ChannelWorkflowRequest,
    ChannelWorkflowStorageRoots,
};
use ironclaw_product_contracts::inbound::{
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
};
use ironclaw_product_contracts::shared_admission::SharedConversationAdmission;

use crate::channel_dm_targets::{FilesystemChannelDmTargetStore, dm_target_payload};
use crate::channel_pairing::ChannelPairingConsumeOutcome;
use crate::extension_ingress::{
    ChannelInboundSinkConfig, ChannelIngressDrain, ChannelIngressRegistration,
    ChannelPairingOutcomeObserver, ExtensionIngressRegistry, GenericChannelInboundSink,
    ManagedRegistrationOutcome, PostAdmissionObserver, VerifiedEvidenceMint,
};
use ironclaw_extension_host::ChannelConfigService;
use ironclaw_product_contracts::admin_users::AdminUserService;

/// Characters that must be escaped when an extension id is interpolated into
/// the `connect` query value. Extension ids arrive as raw strings from durable
/// installation and deployment-binding records, so the id is encoded rather
/// than trusted to be URL-safe: a `&`, `#`, or space in an id would otherwise
/// truncate or re-target the link the notice advertises.
const CONNECT_QUERY_VALUE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// The channel's `connect_required` copy, with a one-click connect link
/// appended when three things hold: the strategy is OAuth, the deployment
/// configured a public web origin, and the channel's own reply adapter
/// declares that it delivers an ephemeral reply privately (#7681).
/// `/chat?connect=<extension>` is an authenticated route, so the link rides
/// the WebUI's existing login round-trip unchanged. The other strategies
/// carry their own `connection.deep_link_template`.
///
/// The privacy condition is the fail-closed half. This notice is the one a
/// shared conversation shows an unlinked sender, and it is kept out of the
/// room only by an `OutboundVisibility::EphemeralTo` request, which
/// [`ChannelReply`](ironclaw_extension_contracts::channel_adapter::ChannelReply)
/// documents as a hint: an adapter that cannot honor it posts publicly
/// instead. Appending a setup link on strategy alone would therefore broadcast
/// it to every member of the conversation the first time an OAuth-strategy
/// channel without ephemeral support ships. Adapters that have not declared
/// the capability keep the static, link-free manifest copy.
fn connect_required_notice(
    connection: &ChannelConnectionDescriptor,
    extension_id: &str,
    base_url: Option<&str>,
    supports_private_delivery: bool,
) -> String {
    let text = &connection.notices.connect_required;
    match base_url {
        Some(base)
            if supports_private_delivery
                && connection.strategy == ChannelConnectionStrategy::OAuth =>
        {
            let base = base.trim_end_matches('/');
            let extension_id =
                percent_encoding::utf8_percent_encode(extension_id, CONNECT_QUERY_VALUE);
            format!("{text} Or connect directly: {base}/chat?connect={extension_id}")
        }
        _ => text.clone(),
    }
}

/// Whether a channel's bound reply adapter delivers an
/// [`OutboundVisibility::EphemeralTo`](ironclaw_extension_contracts::channel_adapter::OutboundVisibility)
/// reply privately. A channel with no reply half emits nothing source-routed,
/// so it cannot deliver a notice privately either.
fn surfaces_support_private_delivery(
    surfaces: &ironclaw_extension_contracts::channel_adapter::ChannelSurfaces,
) -> bool {
    surfaces
        .reply
        .as_ref()
        .is_some_and(|reply| reply.supports_private_delivery())
}

/// The default admission resolver, or `None` (= reject every shared
/// conversation). Admission is presence-based: the channel's verified
/// ingress only delivers a conversation's events to this deployment because
/// the bot is in that conversation, so delivery for this installation is the
/// membership evidence and needs no operator configuration. `None` —
/// unconditionally — when the channel's actor identity is not per-user: an
/// operator-resolver channel that admitted a group would run every
/// participant as the operator, the exact exposure the run-acts-as-invoker
/// rule removed.
fn default_shared_admission(
    actor_identity_is_per_user: bool,
    adapter_id: &ProductAdapterId,
    installation_id: &AdapterInstallationId,
) -> Option<Arc<dyn SharedConversationAdmission>> {
    if !actor_identity_is_per_user {
        return None;
    }
    Some(Arc::new(
        ironclaw_extension_host::channel_shared_admission::PresenceSharedAdmission::new(
            adapter_id.clone(),
            installation_id.clone(),
        ),
    ))
}

/// Derive the trusted-evidence shape the generic inbound sink mints from the
/// resolved contract's ingress verification recipe — the mint mirrors the
/// recipe the generic router executed. `None` for `kind = "none"` recipes:
/// with no verification there is no trusted claim to mint, so the assembly
/// registers nothing and the route fails closed.
pub fn evidence_mint_for_verification(
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
        // No webhook-signature evidence is minted at this layer for either: a
        // `none` recipe has no trusted claim (route fails closed), and an
        // `authenticated_session` channel mounts no webhook route at all — its
        // T1 evidence is minted by the host's authenticated transport, not here.
        IngressVerificationRecipe::AuthenticatedSession | IngressVerificationRecipe::None => None,
    }
}

/// The generic root scheme: everything under one extension-keyed subtree.
pub fn default_channel_workflow_storage_roots(
    tenant_id: &TenantId,
    extension_id: &str,
) -> Result<ChannelWorkflowStorageRoots, String> {
    let tenant = ironclaw_host_api::resource::resource_scope_path_segment(tenant_id.as_str());
    let base = format!("/tenants/{tenant}/shared/channel-extensions/{extension_id}");
    Ok(ChannelWorkflowStorageRoots {
        idempotency: VirtualPath::new(format!("{base}/product-workflow/idempotency"))
            .map_err(|error| format!("invalid idempotency storage root: {error}"))?,
        conversations: VirtualPath::new(format!("{base}/conversations"))
            .map_err(|error| format!("invalid conversation storage root: {error}"))?,
    })
}

/// Per-extension vendor ports that are not yet host-generic. Populated by
/// the extension's composition lane; a pure-manifest channel package
/// registers none.
pub struct ChannelExtras {
    /// The vendor half of the triggered-delivery driver; consumed by the
    /// lane that builds the triggered hook.
    pub preference_target_codec: Option<Arc<dyn PreferenceTargetCodec>>,
    /// Optional shared-conversation admission override. Absent, the
    /// assembly installs the DEFAULT presence-based resolver: delivery
    /// through the channel's verified ingress for this installation is the
    /// membership evidence, so it admits with no configuration.
    pub shared_admission: Option<Arc<dyn SharedConversationAdmission>>,
    /// Legacy storage-root override for the per-extension workflow state.
    pub storage_roots: Option<ChannelWorkflowStorageRoots>,
}

/// The extras retained after registration.
#[derive(Clone, Default)]
struct StoredChannelExtras {
    preference_target_codec: Option<Arc<dyn PreferenceTargetCodec>>,
    shared_admission: Option<Arc<dyn SharedConversationAdmission>>,
    storage_roots: Option<ChannelWorkflowStorageRoots>,
}

/// The deployment identity every per-extension workflow binds under: the
/// composed runtime's tenant/agent/project plus the operator user that
/// host-initiated work is attributed to. Inbound conversations run as their
/// invoking actor, never as this operator identity.
#[derive(Clone)]
pub struct ChannelHostIdentity {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub operator_user_id: UserId,
}

/// Late-bound command execution surface shared by every channel workflow.
///
/// Channel graphs are reconciled while the runtime is still being assembled,
/// before the canonical product surface can be built. The composition fills
/// this handle once at the end of runtime construction. Until then, command
/// execution fails closed with retryable service unavailability.
#[derive(Clone, Default)]
struct SharedCommandSurface {
    cell: Arc<std::sync::OnceLock<Arc<dyn ironclaw_product_contracts::surface::ProductSurface>>>,
}

impl SharedCommandSurface {
    fn set(&self, surface: Arc<dyn ironclaw_product_contracts::surface::ProductSurface>) -> bool {
        self.cell.set(surface).is_ok()
    }
}

#[async_trait]
impl ironclaw_product_contracts::surface::ProductSurface for SharedCommandSurface {
    async fn invoke(
        &self,
        caller: ironclaw_product_contracts::surface::ProductSurfaceCaller,
        request: ironclaw_product_contracts::surface::ProductSurfaceInvokeRequest,
    ) -> Result<
        ironclaw_product_contracts::surface::ProductSurfaceInvokeResponse,
        ironclaw_product_contracts::surface::ProductSurfaceError,
    > {
        match self.cell.get() {
            Some(surface) => surface.invoke(caller, request).await,
            None => Err(
                ironclaw_product_contracts::surface::ProductSurfaceError::service_unavailable(true),
            ),
        }
    }

    async fn query(
        &self,
        caller: ironclaw_product_contracts::surface::ProductSurfaceCaller,
        request: ironclaw_product_contracts::surface::ProductSurfaceQueryRequest,
    ) -> Result<
        ironclaw_product_contracts::surface::ProductSurfaceQueryPage,
        ironclaw_product_contracts::surface::ProductSurfaceError,
    > {
        match self.cell.get() {
            Some(surface) => surface.query(caller, request).await,
            None => Err(
                ironclaw_product_contracts::surface::ProductSurfaceError::service_unavailable(true),
            ),
        }
    }

    async fn stream_events(
        &self,
        caller: ironclaw_product_contracts::surface::ProductSurfaceCaller,
        request: ironclaw_product_contracts::surface::ProductSurfaceStreamRequest,
    ) -> Result<
        ironclaw_product_contracts::surface::ProductSurfaceStreamResponse,
        ironclaw_product_contracts::surface::ProductSurfaceError,
    > {
        match self.cell.get() {
            Some(surface) => surface.stream_events(caller, request).await,
            None => Err(
                ironclaw_product_contracts::surface::ProductSurfaceError::service_unavailable(true),
            ),
        }
    }
}

/// Everything the assembly composes per-extension graphs from.
pub struct GenericChannelHostDeps {
    pub watch: SnapshotWatch,
    pub deployment_channels: Arc<DeploymentChannelRegistry>,
    pub registry: Arc<ExtensionIngressRegistry>,
    pub channel_config: Arc<ChannelConfigService>,
    /// Builds the per-extension product cone (§12.11 D-A). Injected so this
    /// module names no product type: the durable ledger and conversation
    /// store, the inbound-turn service, the surface, the command admission
    /// policy, and the run-delivery observer are all built behind it, and the
    /// runtime services they need (threads, turns, steering queue, attachment
    /// lander, interaction services, delivery coordinator) reach product
    /// directly from composition rather than through this bundle.
    pub channel_workflow: Arc<dyn ChannelWorkflowFactory>,
    pub identity: ChannelHostIdentity,
    /// The generic channel-identity binding store: verified inbound actors
    /// on auth-declaring channel extensions resolve through it. `None`
    /// (composition paths without the durable store) falls back to the
    /// operator-actor policy.
    pub identity_lookup:
        Option<Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityLookup>>,
    /// Durable owner-scoped catalog of proven personal channel destinations.
    /// A successfully admitted direct inbound message backfills this catalog,
    /// covering identities that were bound before DM-target provisioning was
    /// introduced or whose original bind callback did not carry a DM route.
    pub dm_targets: Option<Arc<FilesystemChannelDmTargetStore>>,
    /// Pairing services for `WebGeneratedCode` channel extensions: drives the
    /// sink's pre-admission consume gate and identity-based actor resolution
    /// for extensions that pair without an OAuth vendor.
    pub channel_pairing: Option<Arc<crate::channel_pairing::ChannelPairingRegistry>>,
    /// Admin-users directory backing channel-command role gating.
    pub admin_users: Arc<dyn AdminUserService>,
    /// The deployment's public web app origin (e.g.
    /// `https://ironclaw.example.com`). Backs the connect link appended to an
    /// OAuth-strategy channel's `connect_required` notice; `None` keeps that
    /// notice at its static, link-free manifest text.
    pub connect_link_base_url: Option<String>,
}

/// What the assembly last reconciled for one extension id.
enum ReconciledChannel {
    /// Assembly-built generic graph for exactly this channel source.
    Generic {
        source: HostedChannelSource,
        #[cfg(any(test, feature = "test-support"))]
        conversation_binding: Arc<dyn ProductBindingResolver>,
        /// The post-admission observer registered with the sink (test seam:
        /// gate-resolution acks arriving from non-channel surfaces are
        /// injected through the SAME observer instance the sink drives).
        #[cfg(any(test, feature = "test-support"))]
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

    fn resolved(&self) -> &ironclaw_extension_registry::ResolvedExtensionManifest {
        match self {
            Self::Deployment(binding) => binding.resolved.as_ref(),
            Self::Active(active) => active.resolved.as_ref(),
        }
    }

    /// Whether this channel's bound reply adapter delivers an ephemeral reply
    /// privately. Read from the adapter itself rather than the manifest: the
    /// manifest declares which surfaces exist, the implementation declares
    /// what it can actually do with them.
    fn supports_private_delivery(&self) -> bool {
        surfaces_support_private_delivery(match self {
            Self::Deployment(binding) => &binding.surfaces,
            Self::Active(active) => &active.channel,
        })
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
    #[cfg(any(test, feature = "test-support"))]
    conversation_binding: Arc<dyn ProductBindingResolver>,
    #[cfg(any(test, feature = "test-support"))]
    observer: Option<Arc<dyn PostAdmissionObserver>>,
}

/// The generic channel host assembly: one per composed runtime with a
/// generic extension host. Owns a reconcile loop over the snapshot watch.
pub struct GenericChannelHostAssembly {
    deps: GenericChannelHostDeps,
    command_surface: SharedCommandSurface,
    extras: StdMutex<HashMap<String, StoredChannelExtras>>,
    reconciled: tokio::sync::Mutex<HashMap<String, ReconciledChannel>>,
    reconcile_loop: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl GenericChannelHostAssembly {
    /// Build the assembly, reconcile the current snapshot, and spawn the
    /// watch loop. The loop holds only a weak handle: dropping the returned
    /// `Arc` ends it (and `Drop` aborts it eagerly).
    pub fn start(deps: GenericChannelHostDeps) -> Arc<Self> {
        let mut subscription = deps.watch.subscribe();
        let assembly = Arc::new(Self {
            deps,
            command_surface: SharedCommandSurface::default(),
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

    /// Fill the command surface after the canonical runtime product surface
    /// exists. First write wins so a later construction path cannot swap
    /// authority under already-running channel graphs.
    pub fn set_product_command_surface(
        &self,
        surface: Arc<dyn ironclaw_product_contracts::surface::ProductSurface>,
    ) -> bool {
        self.command_surface.set(surface)
    }

    /// Register one extension's vendor extras, then re-reconcile the
    /// extension against the current snapshot so the remaining extras apply
    /// to the next build.
    pub async fn register_extras(&self, extension_id: &ExtensionId, extras: ChannelExtras) {
        let ChannelExtras {
            preference_target_codec,
            shared_admission,
            storage_roots,
        } = extras;
        if let Ok(mut stored) = self.extras.lock() {
            stored.insert(
                extension_id.as_str().to_owned(),
                StoredChannelExtras {
                    preference_target_codec,
                    shared_admission,
                    storage_roots,
                },
            );
        }
        let mut reconciled = self.reconciled.lock().await;
        reconciled.remove(extension_id.as_str());
        drop(reconciled);
        self.reconcile(self.deps.watch.current()).await;
    }

    /// The registered preference-target codec for one extension, if any —
    /// the triggered-delivery lane resolves its vendor codec through this.
    pub fn preference_target_codec(
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
    pub fn snapshot_watch(&self) -> SnapshotWatch {
        self.deps.watch.clone()
    }

    /// The deployment identity per-extension workflows bind under.
    pub fn identity(&self) -> &ChannelHostIdentity {
        &self.deps.identity
    }

    /// Every ACTIVE channel extension with a registered preference-target
    /// codec, in extension-id order — the generic triggered-delivery hook
    /// routes stored preference refs across these.
    pub fn active_preference_codecs(&self) -> Vec<(String, Arc<dyn PreferenceTargetCodec>)> {
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
                && channel.supports_inbound()
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
                                    #[cfg(any(test, feature = "test-support"))]
                                    conversation_binding: graph.conversation_binding,
                                    #[cfg(any(test, feature = "test-support"))]
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
                        target: "ironclaw::reborn::channel_host",
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
                        target: "ironclaw::reborn::channel_host",
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
    /// secrets over the `[channel.config]` secret storage, per-extension
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

        let secrets = Arc::new(ChannelConfigIngressSecrets {
            channel_config: Arc::clone(&self.deps.channel_config),
            extension_id: ExtensionId::new(source.extension_id())
                .map_err(|error| format!("invalid extension id: {error}"))?,
            handle: secret_handle.clone(),
            installation_id: source.installation_id().to_string(),
        });
        let configuration = Arc::new(ChannelConfigIngressConfiguration {
            channel_config: Arc::clone(&self.deps.channel_config),
            extension_id: ExtensionId::new(source.extension_id())
                .map_err(|error| format!("invalid extension id: {error}"))?,
            installation_id: source.installation_id().to_string(),
        });

        // Channel-command admission's admin-audience gate: the same
        // per-extension provider identity the workflow's actor resolver uses
        // below, so the actor a command's role is resolved for is always the
        // same actor the conversation binding resolved.
        let (command_role_provider, command_role_keyspace, command_role_lookup) =
            self.provider_identity_lookup(source);
        let command_roles = Arc::new(
            crate::channel_command_roles::ChannelActorRoleResolver::new(
                command_role_provider,
                command_role_lookup,
                Arc::clone(&self.deps.admin_users),
                self.deps.identity.tenant_id.clone(),
                self.deps.identity.operator_user_id.clone(),
            )
            .with_identity_keyspace(command_role_keyspace),
        );

        let adapter_id = ProductAdapterId::new(source.extension_id())
            .map_err(|error| format!("invalid adapter id: {error}"))?;
        let actor_user_resolver = self.actor_user_resolver(source);
        // Per-user identity iff a provider lookup exists (OAuth vendor or
        // pairing strategy); the `None` arm is the operator resolver, which
        // must never be combined with shared-conversation admission.
        let actor_identity_is_per_user = self.provider_identity_lookup(source).2.is_some();
        let graph = self
            .deps
            .channel_workflow
            .build_channel_workflow(self.workflow_request(
                source,
                extras,
                adapter_id.clone(),
                command_roles,
                Arc::clone(&actor_user_resolver),
                actor_identity_is_per_user,
            )?)
            .await?;
        let dm_target_backfill =
            self.deps
                .dm_targets
                .as_ref()
                .map(|store| DirectInboundDmTargetBackfill {
                    extension_id: source.extension_id().to_string(),
                    actor_user_resolver,
                    store: Arc::clone(store),
                });
        let observer = if graph.observer.is_some() || dm_target_backfill.is_some() {
            Some(Arc::new(RunDeliveryPostAdmissionObserver::new(
                graph.observer,
                self.connection_notices(source),
                dm_target_backfill,
            )))
        } else {
            None
        };

        let pairing = self
            .deps
            .channel_pairing
            .as_ref()
            .and_then(|registry| registry.get(source.extension_id()))
            .map(|service| service as Arc<dyn crate::extension_ingress::ChannelPairingInterceptor>);
        let mut sink = GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id,
            evidence,
            surface: graph.surface,
            observer: observer
                .clone()
                .map(|observer| observer as Arc<dyn PostAdmissionObserver>),
        });
        if let Some(pairing) = pairing {
            sink = sink.with_pairing(
                pairing,
                observer
                    .clone()
                    .map(|observer| observer as Arc<dyn ChannelPairingOutcomeObserver>),
            );
        }
        let sink = Arc::new(sink);
        let registration = ChannelIngressRegistration {
            secrets,
            configuration,
            sink: Arc::clone(&sink) as Arc<dyn ironclaw_extension_host::ingress::InboundSink>,
            drain: Some(sink as Arc<dyn ChannelIngressDrain>),
        };
        Ok(Some(BuiltGenericChannelGraph {
            registration,
            #[cfg(any(test, feature = "test-support"))]
            conversation_binding: graph.binding,
            #[cfg(any(test, feature = "test-support"))]
            observer: observer.map(|observer| observer as Arc<dyn PostAdmissionObserver>),
        }))
    }

    /// Provider identity + optional identity-lookup binding used to resolve a
    /// verified inbound actor to a bound IronClaw user for this extension:
    /// the OAuth vendor when the extension declares vendor auth, the
    /// extension id when it pairs via `WebGeneratedCode` without a vendor, or
    /// no lookup (the operator-actor policy) otherwise. Shared by
    /// [`Self::build_binding`]'s conversation-binding actor resolver and
    /// [`Self::build_generic_graph`]'s channel-command role resolver so both
    /// read exactly the same per-extension policy instead of two copies that
    /// can drift.
    fn provider_identity_lookup(
        &self,
        source: &HostedChannelSource,
    ) -> (
        String,
        crate::channel_identity::ChannelIdentityKeyspace,
        Option<Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityLookup>>,
    ) {
        let identity_keyspace = crate::channel_identity::ChannelIdentityKeyspace::for_strategy(
            source
                .resolved()
                .channel
                .as_ref()
                .and_then(|channel| channel.connection.as_ref())
                .map(|connection| connection.strategy),
        );
        let pairing_extension = self
            .deps
            .channel_pairing
            .as_ref()
            .is_some_and(|registry| registry.get(source.extension_id()).is_some());
        match (
            self.deps.identity_lookup.as_ref(),
            source.resolved().auth.first(),
        ) {
            (Some(lookup), Some(auth)) => (
                auth.vendor.as_str().to_string(),
                identity_keyspace,
                Some(Arc::clone(lookup)),
            ),
            // Pairing-strategy channels have no OAuth vendor; verified
            // inbound actors resolve through the bindings the pairing
            // consume wrote, keyed by the extension id as provider. Unbound
            // actors fail closed instead of inheriting the operator.
            (Some(lookup), None) if pairing_extension => (
                source.extension_id().to_string(),
                crate::channel_identity::ChannelIdentityKeyspace::Unversioned,
                Some(Arc::clone(lookup)),
            ),
            _ => (
                source.extension_id().to_string(),
                crate::channel_identity::ChannelIdentityKeyspace::Unversioned,
                None,
            ),
        }
    }

    /// Build the one actor resolver shared by conversation admission and the
    /// post-admission DM-target backfill. Sharing the instance is important:
    /// the admission path has already resolved and freshness-checked the
    /// actor, so the observer can reuse that positive resolution without a
    /// third identity-store read after the acknowledgement is committed.
    fn actor_user_resolver(
        &self,
        source: &HostedChannelSource,
    ) -> Arc<dyn ProductActorUserResolver> {
        let (provider, identity_keyspace, provider_lookup) = self.provider_identity_lookup(source);
        match provider_lookup {
            Some(lookup) => Arc::new(
                crate::provider_identity::ProviderIdentityActorResolver::for_any_actor_kind(
                    provider,
                    source.extension_id(),
                    lookup,
                )
                .with_identity_keyspace(identity_keyspace),
            ),
            None => Arc::new(OperatorActorUserResolver {
                operator_user_id: self.deps.identity.operator_user_id.clone(),
            }),
        }
    }

    /// The per-extension inputs the product-side workflow factory needs.
    ///
    /// Everything here is host policy: where the durable state lives, which
    /// identity policy resolves a verified actor, whether shared conversations
    /// are admitted (presence through the channel's verified ingress), which
    /// commands the manifest declares, and how the channel words its connect
    /// notices. What product does with them — the installation scope, the
    /// binding service, the surface, the observer — is behind the port.
    fn workflow_request(
        &self,
        source: &HostedChannelSource,
        extras: &StoredChannelExtras,
        adapter_id: ProductAdapterId,
        command_roles: Arc<dyn ironclaw_product_contracts::command::CommandActorRoleResolver>,
        actor_user_resolver: Arc<dyn ProductActorUserResolver>,
        actor_identity_is_per_user: bool,
    ) -> Result<ChannelWorkflowRequest, String> {
        let identity = &self.deps.identity;
        let storage_roots = match &extras.storage_roots {
            Some(roots) => roots.clone(),
            None => {
                default_channel_workflow_storage_roots(&identity.tenant_id, source.extension_id())?
            }
        };
        let installation_id = AdapterInstallationId::new(source.installation_id())
            .map_err(|error| format!("invalid installation id: {error}"))?;
        // Generic shared-conversation admission (§5.3): an extras override
        // wins; otherwise the default resolver admits by PRESENCE — the
        // conversation's events reached this installation through the
        // channel's verified ingress because the bot is in the conversation,
        // and that delivery is the whole admission. A channel WITHOUT
        // per-user actor identity (no OAuth vendor, no pairing strategy)
        // never admits shared conversations at all and gets `None`, which
        // rejects every shared conversation: every participant there
        // resolves to the operator, which is exactly the exposure
        // run-acts-as-invoker removed — closed structurally rather than by
        // manifest inventory.
        let shared_admission: Option<Arc<dyn SharedConversationAdmission>> = match &extras
            .shared_admission
        {
            Some(resolver) => Some(Arc::clone(resolver)),
            None => {
                default_shared_admission(actor_identity_is_per_user, &adapter_id, &installation_id)
            }
        };
        let channel = source.resolved().channel.as_ref();
        Ok(ChannelWorkflowRequest {
            adapter_id,
            installation_id,
            storage_roots,
            actor_user_resolver,
            shared_admission,
            commands: channel
                .map(|channel| channel.commands.clone())
                .unwrap_or_default(),
            command_roles,
            command_surface: Arc::new(self.command_surface.clone()),
            command_prefix: channel.and_then(|channel| channel.presentation.command_prefix.clone()),
            connection_notices: self.connection_notices(source),
        })
    }

    /// The connect notice wording for one extension: the pairing service's
    /// own policy when it has one, otherwise the manifest connection policy
    /// for non-pairing strategies such as `device_link`, then generic copy.
    fn connection_notices(&self, source: &HostedChannelSource) -> ChannelConnectionNoticePolicy {
        self.deps
            .channel_pairing
            .as_ref()
            .and_then(|registry| registry.get(source.extension_id()))
            .map(|service| service.connection_notices().clone())
            .or_else(|| {
                source
                    .resolved()
                    .channel
                    .as_ref()
                    .and_then(|channel| channel.connection.as_ref())
                    .map(|connection| ChannelConnectionNoticePolicy {
                        connect_required: connect_required_notice(
                            connection,
                            source.extension_id(),
                            self.deps.connect_link_base_url.as_deref(),
                            source.supports_private_delivery(),
                        ),
                        paired: connection.notices.paired.clone(),
                        already_paired_same_user: connection
                            .notices
                            .already_paired_same_user
                            .clone(),
                        already_bound_to_other_user: connection
                            .notices
                            .already_bound_to_other_user
                            .clone(),
                        expired_or_unknown: connection.notices.expired_or_unknown.clone(),
                    })
            })
            .unwrap_or_else(|| ChannelConnectionNoticePolicy::generic(&source.resolved().name))
    }

    /// The live conversation-binding service the assembly registered for one
    /// extension — the SAME instance the registered sink resolves through,
    /// so a pre-resolved binding is the binding admission finds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn binding_service_for_extension_for_test(
        &self,
        extension_id: &str,
    ) -> Option<Arc<dyn ProductBindingResolver>> {
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
    #[cfg(any(test, feature = "test-support"))]
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
    ) -> Result<
        Option<ResolvedProductActorUser>,
        ironclaw_product_contracts::error::ProductOperationFailure,
    > {
        Ok(Some(ResolvedProductActorUser::new(
            self.operator_user_id.clone(),
        )))
    }
}

/// Dynamic verification-secrets port over the `[channel.config]` secret
/// storage: the manifest-declared `verification.secret_handle` is resolved
/// per request, so a configure save takes effect on the next webhook with
/// no route rebuild. No stored secret -> no candidates -> the generic
/// router rejects 401.
struct ChannelConfigIngressSecrets {
    channel_config: Arc<ChannelConfigService>,
    extension_id: ExtensionId,
    handle: SecretHandle,
    installation_id: String,
}

#[async_trait]
impl IngressSecretsPort for ChannelConfigIngressSecrets {
    async fn verification_candidates(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        _handle: Option<&SecretHandle>,
    ) -> Result<Vec<VerificationCandidate>, IngressPortError> {
        let material = self
            .channel_config
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

/// Dynamic non-secret installation configuration. Values are resolved only
/// after the request has verified against this registration's installation.
struct ChannelConfigIngressConfiguration {
    channel_config: Arc<ChannelConfigService>,
    extension_id: ExtensionId,
    installation_id: String,
}

#[async_trait]
impl IngressConfigurationPort for ChannelConfigIngressConfiguration {
    async fn non_secret_config(
        &self,
        extension_id: &str,
        installation_id: &str,
    ) -> Result<Vec<(String, String)>, IngressPortError> {
        if extension_id != self.extension_id.as_str() || installation_id != self.installation_id {
            return Err(IngressPortError {
                reason: "verified installation does not match channel configuration scope"
                    .to_string(),
            });
        }
        self.channel_config
            .effective_non_secret_config(&self.extension_id)
            .await
            .map_err(|error| IngressPortError {
                reason: format!("channel non-secret configuration unavailable: {error}"),
            })
    }
}

/// Adapts the generic run-delivery observer onto the generic sink's
/// post-admission observer seam.
pub struct RunDeliveryPostAdmissionObserver {
    observer: Option<Arc<dyn ChannelRunDeliveryObserver>>,
    connection_notices: ChannelConnectionNoticePolicy,
    dm_target_backfill: Option<DirectInboundDmTargetBackfill>,
}

struct DirectInboundDmTargetBackfill {
    extension_id: String,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    store: Arc<FilesystemChannelDmTargetStore>,
}

impl RunDeliveryPostAdmissionObserver {
    fn new(
        observer: Option<Arc<dyn ChannelRunDeliveryObserver>>,
        connection_notices: ChannelConnectionNoticePolicy,
        dm_target_backfill: Option<DirectInboundDmTargetBackfill>,
    ) -> Self {
        Self {
            observer,
            connection_notices,
            dm_target_backfill,
        }
    }

    async fn backfill_direct_dm_target(
        backfill: &DirectInboundDmTargetBackfill,
        envelope: &ProductInboundEnvelope,
    ) {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return;
        };
        if payload.trigger != ProductTriggerReason::DirectChat {
            return;
        }

        let request = ProductActorUserResolutionRequest::new(
            envelope.adapter_id().clone(),
            envelope.installation_id().clone(),
            envelope.external_actor_ref().clone(),
        );
        let user_id = match backfill
            .actor_user_resolver
            .resolve_product_actor_user(request)
            .await
        {
            Ok(Some(actor)) => actor.user_id,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    target: "ironclaw::reborn::channel_host",
                    extension_id = %backfill.extension_id,
                    %error,
                    "direct channel target backfill actor resolution failed"
                );
                return;
            }
        };
        let conversation = envelope.external_conversation_ref();
        if let Err(error) = backfill
            .store
            .upsert(
                &backfill.extension_id,
                &user_id,
                envelope.external_actor_ref().id().to_string(),
                dm_target_payload(conversation.space_id(), conversation.conversation_id()),
            )
            .await
        {
            tracing::warn!(
                target: "ironclaw::reborn::channel_host",
                extension_id = %backfill.extension_id,
                %error,
                "direct channel target backfill failed"
            );
        }
    }
}

fn ack_confirms_direct_admission(ack: &ProductInboundAck) -> bool {
    match ack {
        ProductInboundAck::Accepted { .. }
        | ProductInboundAck::DeferredBusy { .. }
        | ProductInboundAck::RejectedBusy { .. } => true,
        ProductInboundAck::Duplicate { prior } => ack_confirms_direct_admission(prior),
        ProductInboundAck::Rejected(_)
        | ProductInboundAck::CommandResult { .. }
        | ProductInboundAck::NoOp => false,
    }
}

#[async_trait]
impl PostAdmissionObserver for RunDeliveryPostAdmissionObserver {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        if ack_confirms_direct_admission(&ack)
            && let Some(backfill) = &self.dm_target_backfill
        {
            Self::backfill_direct_dm_target(backfill, &envelope).await;
        }
        if let Some(observer) = &self.observer {
            observer.observe_ack(envelope, ack).await;
        }
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_host_api::product_adapter_error::ProductAdapterError,
    ) {
        if let Some(observer) = &self.observer {
            observer.observe_error(envelope, error).await;
        }
    }
}

#[async_trait]
impl ChannelPairingOutcomeObserver for RunDeliveryPostAdmissionObserver {
    async fn observe_pairing_outcome(
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
        if let Some(observer) = &self.observer {
            observer
                .post_connection_status_notice(&conversation, &event_id, text)
                .await;
        }
    }
}

#[cfg(test)]
mod e2e_tests;

#[cfg(test)]
mod tests {
    use ironclaw_extension_contracts::recipe::{
        HmacSha256VerificationRecipe, SharedSecretHeaderRecipe, SignatureEncoding,
        SignedPayloadSegment,
    };
    use ironclaw_product_contracts::shared_admission::SharedConversationAdmissionRequest;

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

    fn connection_descriptor(strategy: ChannelConnectionStrategy) -> ChannelConnectionDescriptor {
        ChannelConnectionDescriptor {
            provider: ironclaw_host_api::ids::VendorId::new("vendorx").expect("vendor id"),
            strategy,
            instructions: "connect".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect".to_string(),
            error_message: "failed".to_string(),
            notices: ironclaw_extension_contracts::channel::ChannelConnectionNotices {
                connect_required: "Connect your account.".to_string(),
                paired: "paired".to_string(),
                already_paired_same_user: "already".to_string(),
                already_bound_to_other_user: "other".to_string(),
                expired_or_unknown: "expired".to_string(),
            },
            connection_success_message: "connected".to_string(),
            deep_link_template: None,
            inbound_code_prefixes: Vec::new(),
        }
    }

    /// #7681: the one-click link is appended only for an OAuth-strategy channel
    /// on a deployment that configured a public web origin, and only when the
    /// channel's reply adapter delivers ephemeral replies privately. Every
    /// other combination must return the manifest copy verbatim — a relative,
    /// vendor-mismatched, or room-visible link would be posted into a customer
    /// conversation.
    #[test]
    fn connect_notice_appends_the_link_only_for_oauth_with_a_base_url() {
        let oauth = connection_descriptor(ChannelConnectionStrategy::OAuth);

        assert_eq!(
            connect_required_notice(&oauth, "slack", Some("https://app.example.com"), true),
            "Connect your account. Or connect directly: https://app.example.com/chat?connect=slack"
        );
        // A trailing slash on the configured origin must not produce `//chat`.
        assert_eq!(
            connect_required_notice(&oauth, "slack", Some("https://app.example.com/"), true),
            "Connect your account. Or connect directly: https://app.example.com/chat?connect=slack"
        );
        // Unset origin ships dark: the notice stays link-free rather than
        // advertising an unreachable relative path.
        assert_eq!(
            connect_required_notice(&oauth, "slack", None, true),
            oauth.notices.connect_required
        );
        // A non-OAuth strategy carries its own deep link, so no link leaks
        // into its copy even when an origin IS configured.
        for strategy in [
            ChannelConnectionStrategy::DeviceLink,
            ChannelConnectionStrategy::WebGeneratedCode,
            ChannelConnectionStrategy::AdminManagedChannels,
        ] {
            let other = connection_descriptor(strategy);
            assert_eq!(
                connect_required_notice(&other, "slack", Some("https://app.example.com"), true),
                other.notices.connect_required,
                "{strategy:?} must return the manifest copy verbatim"
            );
        }
    }

    /// The fail-closed half: privacy for this notice comes from an
    /// `EphemeralTo` request the adapter may ignore, so an OAuth-strategy
    /// channel whose reply adapter has NOT declared private delivery must not
    /// carry a setup link — publishing it would show every member of a shared
    /// conversation the link meant for one unlinked sender.
    #[test]
    fn connect_notice_withholds_the_link_when_the_adapter_cannot_deliver_privately() {
        let oauth = connection_descriptor(ChannelConnectionStrategy::OAuth);
        assert_eq!(
            connect_required_notice(&oauth, "slack", Some("https://app.example.com"), false),
            oauth.notices.connect_required
        );
    }

    /// The gate input itself: the link decision reads what the bound reply
    /// adapter declares, so an adapter that never opted in (the default, e.g.
    /// Telegram's `sendMessage`-only reply) reports `false` and a channel with
    /// no reply half at all reports `false` too.
    #[test]
    fn private_delivery_support_is_read_from_the_bound_reply_adapter() {
        use ironclaw_extension_contracts::channel_adapter::{
            ChannelError, ChannelReply, ChannelSurfaces, DeliveryReport, OutboundEnvelope,
        };
        use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;

        struct PublicOnlyReply;
        #[async_trait::async_trait]
        impl ChannelReply for PublicOnlyReply {
            async fn send_reply(
                &self,
                _envelope: OutboundEnvelope,
                _egress: &dyn RestrictedEgress,
            ) -> Result<DeliveryReport, ChannelError> {
                Ok(DeliveryReport::default())
            }
        }

        struct EphemeralReply;
        #[async_trait::async_trait]
        impl ChannelReply for EphemeralReply {
            async fn send_reply(
                &self,
                _envelope: OutboundEnvelope,
                _egress: &dyn RestrictedEgress,
            ) -> Result<DeliveryReport, ChannelError> {
                Ok(DeliveryReport::default())
            }

            fn supports_private_delivery(&self) -> bool {
                true
            }
        }

        assert!(!surfaces_support_private_delivery(
            &ChannelSurfaces::default()
        ));
        assert!(!surfaces_support_private_delivery(
            &ChannelSurfaces::default().with_reply(Arc::new(PublicOnlyReply))
        ));
        assert!(surfaces_support_private_delivery(
            &ChannelSurfaces::default().with_reply(Arc::new(EphemeralReply))
        ));
    }

    /// The extension id comes from a durable record, so it is percent-encoded:
    /// an unescaped `&` would truncate the query and re-target the link.
    #[test]
    fn connect_notice_percent_encodes_the_extension_id() {
        let oauth = connection_descriptor(ChannelConnectionStrategy::OAuth);
        assert_eq!(
            connect_required_notice(
                &oauth,
                "sl ack&evil=1",
                Some("https://app.example.com"),
                true
            ),
            "Connect your account. Or connect directly: \
             https://app.example.com/chat?connect=sl%20ack%26evil%3D1"
        );
    }

    #[test]
    fn none_recipe_mints_nothing() {
        assert!(evidence_mint_for_verification(&IngressVerificationRecipe::None).is_none());
    }

    #[test]
    fn default_storage_roots_are_extension_keyed() {
        let tenant = TenantId::new("acme-tenant").expect("tenant id");
        let roots =
            default_channel_workflow_storage_roots(&tenant, "vendorx").expect("roots build");
        assert_eq!(
            roots.idempotency.as_str(),
            "/tenants/acme-tenant/shared/channel-extensions/vendorx/product-workflow/idempotency"
        );
        assert_eq!(
            roots.conversations.as_str(),
            "/tenants/acme-tenant/shared/channel-extensions/vendorx/conversations"
        );
    }

    /// Pin for the run-acts-as-invoker ruling (#7377): a channel WITHOUT
    /// per-user actor identity (no OAuth vendor, no pairing strategy) must get
    /// NO shared-conversation admission resolver at all — `None` fails every
    /// shared conversation closed in the product workflow. Such a channel
    /// resolves every participant to the operator, so admitting a group there
    /// would run every participant AS the operator — the exact exposure the
    /// ruling removed. The gate must hinge on `actor_identity_is_per_user`
    /// itself (an unconditional `if false { return None }` or an
    /// always-`Some` default both fail here).
    #[tokio::test]
    async fn operator_identity_channel_gets_no_shared_admission_resolver() {
        let adapter_id = ProductAdapterId::new("vendorx").expect("adapter id");
        let installation_id = AdapterInstallationId::new("vendorx-install-1").expect("install id");

        assert!(
            default_shared_admission(false, &adapter_id, &installation_id).is_none(),
            "an operator-identity channel must never receive an admission resolver"
        );

        // Per-user identity gets the presence default — and the resolver is
        // bound to THIS adapter installation, proven behaviorally (its fields
        // are private): it admits its own installation's conversation and
        // refuses a foreign installation's. A resolver constructed with the
        // wrong identity would invert one of these.
        let resolver = default_shared_admission(true, &adapter_id, &installation_id)
            .expect("per-user identity installs the presence-based default");
        let request = |installation: &str| SharedConversationAdmissionRequest {
            adapter_id: adapter_id.clone(),
            installation_id: AdapterInstallationId::new(installation).expect("install id"),
            route_key:
                ironclaw_product_contracts::shared_admission::ProductConversationRouteKey::new(
                    Some("S1".to_string()),
                    "C777".to_string(),
                )
                .expect("route key"),
        };
        assert!(
            resolver
                .shared_conversation_admitted(request("vendorx-install-1"))
                .await
                .expect("admission resolves"),
            "the default resolver admits its own installation by presence"
        );
        assert!(
            !resolver
                .shared_conversation_admitted(request("vendorx-install-2"))
                .await
                .expect("admission resolves"),
            "the default resolver is bound to its own installation, not admit-all"
        );
    }
}
