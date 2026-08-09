//! Adapter from product workflow binding requests to `ironclaw_conversations`.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_extension_contracts::external::ExternalActorRef;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};

use ironclaw_product_contracts::actor_identity::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ResolvedProductActorUser,
};
use ironclaw_product_contracts::binding::{
    ProductBindingResolver, ProductConversationRouteKind, ResetBindingOutcome, ResetBindingRequest,
    ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::shared_admission::{
    ProductConversationRouteKey, SharedConversationAdmission, SharedConversationAdmissionRequest,
};

/// Tenant-scoped installation identity used before external actor/conversation
/// refs enter the conversation binding layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductInstallationKey {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
}

impl ProductInstallationKey {
    pub fn new(adapter_id: ProductAdapterId, installation_id: AdapterInstallationId) -> Self {
        Self {
            adapter_id,
            installation_id,
        }
    }
}

/// Build a shared-conversation admission request from an inbound binding
/// request.
///
/// A free function rather than an associated one: the request type is declared
/// in `ironclaw_product_contracts` (so the extension host can implement the
/// resolver without depending on product), and `ResolveBindingRequest` is
/// product's own, so the bridge between them belongs here.
fn admission_request_from_binding_request(
    request: &ResolveBindingRequest,
) -> SharedConversationAdmissionRequest {
    SharedConversationAdmissionRequest {
        adapter_id: request.adapter_id.clone(),
        installation_id: request.installation_id.clone(),
        route_key: ProductConversationRouteKey::from_external_conversation_ref(
            &request.external_conversation_ref,
        ),
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaticProductActorUserResolver {
    bindings: HashMap<ExternalActorRef, UserId>,
}

impl StaticProductActorUserResolver {
    pub fn new(bindings: impl IntoIterator<Item = (ExternalActorRef, UserId)>) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
        }
    }
}

#[async_trait]
impl ProductActorUserResolver for StaticProductActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<ResolvedProductActorUser>, ProductOperationFailure> {
        Ok(self
            .bindings
            .get(&request.external_actor_ref)
            .cloned()
            .map(ResolvedProductActorUser::new))
    }
}

/// Host-owned actor binding policy for one adapter installation.
#[derive(Clone, Default)]
pub enum ProductActorBindingPolicy {
    /// Use the canonical conversations service's trusted installation path,
    /// creating the first external conversation binding for an already paired
    /// actor when needed.
    #[default]
    ExistingConversationPairings,
    /// Allow only actors resolved by this host-owned resolver and write their
    /// pairings into the canonical conversations service before resolving the
    /// external conversation binding.
    ResolveActor {
        resolver: Arc<dyn ProductActorUserResolver>,
        actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    },
}

impl std::fmt::Debug for ProductActorBindingPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExistingConversationPairings => {
                formatter.write_str("ExistingConversationPairings")
            }
            Self::ResolveActor { .. } => formatter.write_str("ResolveActor(..)"),
        }
    }
}

/// Trusted host configuration for one adapter installation.
#[derive(Debug, Clone)]
pub struct ProductInstallationScope {
    pub tenant_id: TenantId,
    pub default_agent_id: Option<AgentId>,
    pub default_project_id: Option<ProjectId>,
    /// Shared-conversation admission. Fail-closed either way: `None` rejects
    /// every shared conversation; `Some` admits exactly the conversations the
    /// resolver says are connected. There is no subject half — a run acts as
    /// the user who invoked it, on every route kind.
    pub shared_conversation_admission: Option<Arc<dyn SharedConversationAdmission>>,
    pub actor_binding_policy: ProductActorBindingPolicy,
}

impl ProductInstallationScope {
    pub fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            default_agent_id: None,
            default_project_id: None,
            shared_conversation_admission: None,
            actor_binding_policy: ProductActorBindingPolicy::default(),
        }
    }

    pub fn with_default_scope(
        tenant_id: TenantId,
        default_agent_id: AgentId,
        default_project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            default_agent_id: Some(default_agent_id),
            default_project_id,
            shared_conversation_admission: None,
            actor_binding_policy: ProductActorBindingPolicy::default(),
        }
    }

    pub fn with_shared_conversation_admission(
        mut self,
        admission: Arc<dyn SharedConversationAdmission>,
    ) -> Self {
        self.shared_conversation_admission = Some(admission);
        self
    }

    pub fn with_actor_binding_policy(mut self, policy: ProductActorBindingPolicy) -> Self {
        self.actor_binding_policy = policy;
        self
    }

    pub fn with_preconfigured_actor_bindings(
        self,
        bindings: impl IntoIterator<Item = (ExternalActorRef, UserId)>,
        actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    ) -> Self {
        self.with_actor_user_resolver(
            Arc::new(StaticProductActorUserResolver::new(bindings)),
            actor_pairings,
        )
    }

    pub fn with_preconfigured_actor_binding(
        self,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
        actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    ) -> Self {
        self.with_preconfigured_actor_bindings([(external_actor_ref, user_id)], actor_pairings)
    }

    pub fn with_actor_user_resolver(
        self,
        resolver: Arc<dyn ProductActorUserResolver>,
        actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    ) -> Self {
        self.with_actor_binding_policy(ProductActorBindingPolicy::ResolveActor {
            resolver,
            actor_pairings,
        })
    }

    /// Fail-closed shared-conversation admission: a `Shared` route resolves
    /// only when the installation's admission resolver says the conversation
    /// is connected. `Direct` routes are never gated here. Checked on every
    /// resolve, lookup, and reset, so a disconnected conversation stops
    /// routing immediately — the same admission semantics the retired
    /// subject-route requirement carried, without the subject.
    async fn ensure_shared_route_admitted(
        &self,
        request: &ResolveBindingRequest,
    ) -> Result<(), ProductOperationFailure> {
        if request.route_kind != ProductConversationRouteKind::Shared {
            return Ok(());
        }
        let admitted = match &self.shared_conversation_admission {
            None => false,
            Some(admission) => {
                admission
                    .shared_conversation_admitted(admission_request_from_binding_request(request))
                    .await?
            }
        };
        if admitted {
            Ok(())
        } else {
            Err(shared_conversation_not_connected_error())
        }
    }
}

/// Static tenant map for product adapter installations.
#[derive(Debug, Clone, Default)]
pub struct StaticProductInstallationResolver {
    scopes: HashMap<ProductInstallationKey, Arc<ProductInstallationScope>>,
}

impl StaticProductInstallationResolver {
    pub fn new(
        scopes: impl IntoIterator<Item = (ProductInstallationKey, ProductInstallationScope)>,
    ) -> Self {
        Self {
            scopes: scopes
                .into_iter()
                .map(|(key, scope)| (key, Arc::new(scope)))
                .collect(),
        }
    }

    pub fn insert(&mut self, key: ProductInstallationKey, scope: ProductInstallationScope) {
        self.scopes.insert(key, Arc::new(scope));
    }

    fn resolve(
        &self,
        adapter_id: &ProductAdapterId,
        installation_id: &AdapterInstallationId,
    ) -> Result<Arc<ProductInstallationScope>, ProductOperationFailure> {
        self.scopes
            .get(&ProductInstallationKey::new(
                adapter_id.clone(),
                installation_id.clone(),
            ))
            .cloned()
            .ok_or(ProductOperationFailure::UnknownInstallation)
    }
}

/// Product workflow binding service backed by the canonical conversations
/// service. Tenant selection comes only from trusted installation config.
#[derive(Clone)]
pub struct ProductConversationBindingService {
    conversations: Arc<dyn ironclaw_conversations::ConversationBindingService>,
    installations: StaticProductInstallationResolver,
}

impl ProductConversationBindingService {
    pub fn new(
        conversations: Arc<dyn ironclaw_conversations::ConversationBindingService>,
        installations: StaticProductInstallationResolver,
    ) -> Self {
        Self {
            conversations,
            installations,
        }
    }

    async fn apply_resolved_actor_binding(
        &self,
        installation_scope: &ProductInstallationScope,
        request: &ResolveBindingRequest,
        resolved_actor: &ResolvedProductActorUser,
    ) -> Result<(), ProductOperationFailure> {
        let ProductActorBindingPolicy::ResolveActor { actor_pairings, .. } =
            &installation_scope.actor_binding_policy
        else {
            return Ok(());
        };
        let tenant_id = installation_scope.tenant_id.clone();
        let adapter_kind = conversation_adapter_kind(&request.adapter_id)?;
        let installation_id = conversation_installation_id(&request.installation_id)?;
        let external_actor_ref = request.external_actor_ref.clone();
        match resolved_actor.binding_epoch.clone() {
            Some(binding_epoch) => {
                actor_pairings
                    .pair_external_actor_with_epoch(
                        tenant_id,
                        adapter_kind,
                        installation_id,
                        external_actor_ref,
                        resolved_actor.user_id.clone(),
                        binding_epoch,
                    )
                    .await
                    .map_err(map_conversation_error)?;
            }
            None => {
                actor_pairings
                    .pair_external_actor(
                        tenant_id,
                        adapter_kind,
                        installation_id,
                        external_actor_ref,
                        resolved_actor.user_id.clone(),
                    )
                    .await
                    .map_err(map_conversation_error)?;
            }
        }
        Ok(())
    }

    async fn ensure_resolved_actor_binding_still_current(
        &self,
        installation_scope: &ProductInstallationScope,
        request: &ResolveBindingRequest,
        expected_actor: Option<&ResolvedProductActorUser>,
    ) -> Result<(), ProductOperationFailure> {
        let Some(expected_actor) = expected_actor else {
            return Ok(());
        };
        let ProductActorBindingPolicy::ResolveActor {
            resolver,
            actor_pairings,
        } = &installation_scope.actor_binding_policy
        else {
            return Ok(());
        };
        if resolver
            .resolved_product_actor_user_is_current(
                &actor_user_resolution_request(request),
                expected_actor,
            )
            .await?
        {
            return Ok(());
        }
        actor_pairings
            .unpair_external_actor_if_owned_by(
                &installation_scope.tenant_id,
                &conversation_adapter_kind(&request.adapter_id)?,
                &conversation_installation_id(&request.installation_id)?,
                &request.external_actor_ref,
                &ironclaw_conversations::ExpectedExternalActorOwner {
                    user_id: expected_actor.user_id.clone(),
                    binding_epoch: expected_actor.binding_epoch.clone(),
                },
            )
            .await
            .map_err(map_conversation_error)?;
        Err(ProductOperationFailure::BindingRequired {
            reason: "external actor binding was revoked while resolving this message".into(),
        })
    }

    /// The reset authorization pair, shared by `reset_binding`'s pre- and
    /// post-rotation checks so the two cannot drift: resolved-actor match and
    /// resolver-backed actor currency. (Shared-conversation admission is
    /// checked once before any mutation; it is pure per-request state.)
    async fn ensure_reset_resolution_authorized(
        &self,
        installation_scope: &ProductInstallationScope,
        resolve_request: &ResolveBindingRequest,
        expected_actor: Option<&ResolvedProductActorUser>,
        resolution: &ironclaw_conversations::ConversationBindingResolution,
    ) -> Result<(), ProductOperationFailure> {
        ensure_resolved_actor_matches_expected_user(expected_actor, resolution)?;
        self.ensure_resolved_actor_binding_still_current(
            installation_scope,
            resolve_request,
            expected_actor,
        )
        .await
    }
}

fn actor_user_resolution_request(
    request: &ResolveBindingRequest,
) -> ProductActorUserResolutionRequest {
    ProductActorUserResolutionRequest::new(
        request.adapter_id.clone(),
        request.installation_id.clone(),
        request.external_actor_ref.clone(),
    )
}

async fn resolve_actor_user(
    installation_scope: &ProductInstallationScope,
    request: &ResolveBindingRequest,
) -> Result<Option<ResolvedProductActorUser>, ProductOperationFailure> {
    match &installation_scope.actor_binding_policy {
        ProductActorBindingPolicy::ExistingConversationPairings => Ok(None),
        ProductActorBindingPolicy::ResolveActor { resolver, .. } => resolver
            .resolve_product_actor_user(actor_user_resolution_request(request))
            .await?
            .map(Some)
            .ok_or_else(|| ProductOperationFailure::BindingRequired {
                reason: "external actor is not bound for this adapter installation".into(),
            }),
    }
}

fn ensure_resolved_actor_matches_expected_user(
    expected_actor: Option<&ResolvedProductActorUser>,
    resolution: &ironclaw_conversations::ConversationBindingResolution,
) -> Result<(), ProductOperationFailure> {
    if let Some(expected_actor) = expected_actor
        && (resolution.actor.user_id != expected_actor.user_id
            || resolution.binding_epoch != expected_actor.binding_epoch)
    {
        return Err(ProductOperationFailure::BindingAccessDenied);
    }
    Ok(())
}

#[async_trait]
impl ProductBindingResolver for ProductConversationBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
        let installation_scope = self
            .installations
            .resolve(&request.adapter_id, &request.installation_id)?;
        let conversation_request =
            conversation_request(&request, installation_scope.tenant_id.clone())?;
        installation_scope
            .ensure_shared_route_admitted(&request)
            .await?;
        let expected_actor = resolve_actor_user(&installation_scope, &request).await?;
        if let Some(resolved_actor) = expected_actor.as_ref() {
            self.apply_resolved_actor_binding(&installation_scope, &request, resolved_actor)
                .await?;
        }
        // No trusted owner: the conversations domain keys and owns shared
        // bindings by the paired actor (a run acts as the user who invoked
        // it), and Direct product bindings carry no explicit owner.
        let resolution = self
            .conversations
            .resolve_or_create_binding_with_trusted_scope(
                conversation_request,
                installation_scope.default_agent_id.clone(),
                installation_scope.default_project_id.clone(),
                None,
            )
            .await
            .map_err(map_conversation_error)?;
        ensure_resolved_actor_matches_expected_user(expected_actor.as_ref(), &resolution)?;
        self.ensure_resolved_actor_binding_still_current(
            &installation_scope,
            &request,
            expected_actor.as_ref(),
        )
        .await?;

        resolved_binding_from_resolution(resolution)
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
        let installation_scope = self
            .installations
            .resolve(&request.adapter_id, &request.installation_id)?;
        let conversation_request =
            conversation_request(&request, installation_scope.tenant_id.clone())?;
        installation_scope
            .ensure_shared_route_admitted(&request)
            .await?;
        let resolution = self
            .conversations
            .lookup_binding(conversation_request)
            .await
            .map_err(map_conversation_error)?;
        let expected_actor = resolve_actor_user(&installation_scope, &request).await?;
        ensure_resolved_actor_matches_expected_user(expected_actor.as_ref(), &resolution)?;

        resolved_binding_from_resolution(resolution)
    }

    async fn reset_binding(
        &self,
        request: ResetBindingRequest,
    ) -> Result<ResetBindingOutcome, ProductOperationFailure> {
        let ResetBindingRequest {
            resolve_request,
            expected_thread_id,
        } = request;
        let installation_scope = self.installations.resolve(
            &resolve_request.adapter_id,
            &resolve_request.installation_id,
        )?;
        let conversation_request =
            conversation_request(&resolve_request, installation_scope.tenant_id.clone())?;
        // Gate the pairing write below on shared-conversation admission before
        // any mutation.
        installation_scope
            .ensure_shared_route_admitted(&resolve_request)
            .await?;
        let existing = self
            .conversations
            .lookup_binding(conversation_request.clone())
            .await
            .map_err(map_conversation_error)?;

        let expected_actor = resolve_actor_user(&installation_scope, &resolve_request).await?;
        if let Some(resolved_actor) = expected_actor.as_ref() {
            self.apply_resolved_actor_binding(
                &installation_scope,
                &resolve_request,
                resolved_actor,
            )
            .await?;
        }
        self.ensure_reset_resolution_authorized(
            &installation_scope,
            &resolve_request,
            expected_actor.as_ref(),
            &existing,
        )
        .await?;

        let outcome = self
            .conversations
            .reset_conversation_binding(ironclaw_conversations::ResetConversationRequest {
                resolve_request: conversation_request,
                expected_thread_id,
            })
            .await
            .map_err(map_conversation_error)?;
        self.ensure_reset_resolution_authorized(
            &installation_scope,
            &resolve_request,
            expected_actor.as_ref(),
            &outcome.resolution,
        )
        .await?;
        let binding = resolved_binding_from_resolution(outcome.resolution)?;
        Ok(ResetBindingOutcome {
            previous_thread_id: outcome.previous_thread_id,
            binding,
        })
    }
}

fn resolved_binding_from_resolution(
    resolution: ironclaw_conversations::ConversationBindingResolution,
) -> Result<ResolvedBinding, ProductOperationFailure> {
    Ok(ResolvedBinding {
        tenant_id: resolution.tenant_id,
        actor_user_id: resolution.actor.user_id,
        thread_id: resolution.turn_scope.thread_id,
        agent_id: resolution.turn_scope.agent_id,
        project_id: resolution.turn_scope.project_id,
    })
}

fn shared_conversation_not_connected_error() -> ProductOperationFailure {
    ProductOperationFailure::BindingRequired {
        reason: "shared conversation is not connected for this channel".into(),
    }
}

fn conversation_request(
    request: &ResolveBindingRequest,
    tenant_id: TenantId,
) -> Result<ironclaw_conversations::ResolveConversationRequest, ProductOperationFailure> {
    Ok(ironclaw_conversations::ResolveConversationRequest {
        tenant_id,
        adapter_kind: conversation_adapter_kind(&request.adapter_id)?,
        adapter_installation_id: conversation_installation_id(&request.installation_id)?,
        external_actor_ref: request.external_actor_ref.clone(),
        external_conversation_ref: request.external_conversation_ref.clone(),
        external_event_id: conversation_event_id(&request.external_event_id)?,
        route_kind: conversation_route_kind(request.route_kind),
        requested_agent_id: None,
        requested_project_id: None,
    })
}

fn conversation_adapter_kind(
    adapter_id: &ProductAdapterId,
) -> Result<ironclaw_conversations::AdapterKind, ProductOperationFailure> {
    ironclaw_conversations::AdapterKind::new(adapter_id.as_str()).map_err(map_conversation_error)
}

fn conversation_installation_id(
    installation_id: &AdapterInstallationId,
) -> Result<ironclaw_conversations::AdapterInstallationId, ProductOperationFailure> {
    ironclaw_conversations::AdapterInstallationId::new(installation_id.as_str())
        .map_err(map_conversation_error)
}

fn conversation_event_id(
    event_id: &ironclaw_extension_contracts::external::ExternalEventId,
) -> Result<ironclaw_conversations::ExternalEventId, ProductOperationFailure> {
    ironclaw_conversations::ExternalEventId::new(event_id.as_str()).map_err(map_conversation_error)
}

fn conversation_route_kind(
    route_kind: ProductConversationRouteKind,
) -> ironclaw_conversations::ConversationRouteKind {
    match route_kind {
        ProductConversationRouteKind::Direct => {
            ironclaw_conversations::ConversationRouteKind::Direct
        }
        ProductConversationRouteKind::Shared => {
            ironclaw_conversations::ConversationRouteKind::Shared
        }
    }
}

fn map_conversation_error(
    error: ironclaw_conversations::InboundTurnError,
) -> ProductOperationFailure {
    match error {
        ironclaw_conversations::InboundTurnError::InvalidExternalRef { reason, .. }
        | ironclaw_conversations::InboundTurnError::InvalidCanonicalRef { reason } => {
            ProductOperationFailure::InvalidBindingRequest { reason }
        }
        ironclaw_conversations::InboundTurnError::BindingRequired { .. } => {
            ProductOperationFailure::BindingRequired {
                reason: "external actor is not paired with a canonical user".into(),
            }
        }
        ironclaw_conversations::InboundTurnError::AccessDenied { .. }
        | ironclaw_conversations::InboundTurnError::BindingConflict { .. }
        | ironclaw_conversations::InboundTurnError::ThreadNotFound { .. } => {
            ProductOperationFailure::BindingAccessDenied
        }
        ironclaw_conversations::InboundTurnError::StatePoisoned
        | ironclaw_conversations::InboundTurnError::DurableState { .. } => {
            ProductOperationFailure::Transient {
                reason: "conversation binding store unavailable".into(),
            }
        }
        // Unreachable on this surface, and kept only for exhaustiveness. Every
        // `InboundTurnError` this function sees comes from
        // `ProductBindingResolver` — resolve/lookup/link/validate and the
        // id constructors — which never submits a turn; the submission
        // orchestration is `ironclaw_conversations::InboundTurnService`, which
        // this crate does not use (it has its own `DefaultInboundTurnService`
        // calling the coordinator directly, and that is where every live
        // `ProductOperationFailure::TurnSubmissionFailed` is minted, with a real
        // `TurnError`).
        //
        // Since WS5's port inversion, conversations carries the *port's*
        // `TurnSubmissionError` here rather than a `TurnError`. A `TurnError`
        // is deliberately NOT synthesized back from it: fabricating a kernel
        // error to satisfy a variant no caller can reach would be a shim.
        // The port error's own rendering is carried through instead, so the
        // typed cause is preserved in the message rather than dropped.
        ironclaw_conversations::InboundTurnError::TurnSubmissionFailed { error } => {
            ProductOperationFailure::TurnSubmissionRejected {
                reason: error.to_string(),
            }
        }
    }
}
