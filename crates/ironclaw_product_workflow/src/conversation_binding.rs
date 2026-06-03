//! Adapter from product workflow binding requests to `ironclaw_conversations`.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef, ProductAdapterId};

use crate::{
    ConversationBindingService, ProductConversationRouteKind, ProductWorkflowError,
    ResolveBindingRequest, ResolvedBinding,
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

/// Host-owned actor binding policy for one adapter installation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProductActorBindingPolicy {
    /// Use the canonical conversations service's trusted installation path,
    /// creating the first external actor/conversation binding when needed.
    #[default]
    ExistingConversationPairings,
    /// Allow only these host-preconfigured external actors and write their
    /// pairings into the canonical conversations service before resolving the
    /// external conversation binding.
    PreconfiguredOnly(HashMap<ExternalActorRef, UserId>),
}

/// Trusted host configuration for one adapter installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductInstallationScope {
    pub tenant_id: TenantId,
    pub default_agent_id: Option<AgentId>,
    pub default_project_id: Option<ProjectId>,
    pub actor_binding_policy: ProductActorBindingPolicy,
}

impl ProductInstallationScope {
    pub fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            default_agent_id: None,
            default_project_id: None,
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
            actor_binding_policy: ProductActorBindingPolicy::default(),
        }
    }

    pub fn with_actor_binding_policy(mut self, policy: ProductActorBindingPolicy) -> Self {
        self.actor_binding_policy = policy;
        self
    }

    pub fn with_preconfigured_actor_bindings(
        self,
        bindings: impl IntoIterator<Item = (ExternalActorRef, UserId)>,
    ) -> Self {
        self.with_actor_binding_policy(ProductActorBindingPolicy::PreconfiguredOnly(
            bindings.into_iter().collect(),
        ))
    }

    pub fn with_preconfigured_actor_binding(
        self,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
    ) -> Self {
        self.with_preconfigured_actor_bindings([(external_actor_ref, user_id)])
    }
}

/// Static tenant map for product adapter installations.
#[derive(Debug, Clone, Default)]
pub struct StaticProductInstallationResolver {
    scopes: HashMap<ProductInstallationKey, ProductInstallationScope>,
}

impl StaticProductInstallationResolver {
    pub fn new(
        scopes: impl IntoIterator<Item = (ProductInstallationKey, ProductInstallationScope)>,
    ) -> Self {
        Self {
            scopes: scopes.into_iter().collect(),
        }
    }

    pub fn insert(&mut self, key: ProductInstallationKey, scope: ProductInstallationScope) {
        self.scopes.insert(key, scope);
    }

    fn resolve(
        &self,
        adapter_id: &ProductAdapterId,
        installation_id: &AdapterInstallationId,
    ) -> Result<ProductInstallationScope, ProductWorkflowError> {
        self.scopes
            .get(&ProductInstallationKey::new(
                adapter_id.clone(),
                installation_id.clone(),
            ))
            .cloned()
            .ok_or(ProductWorkflowError::UnknownInstallation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PreconfiguredActorCacheKey {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    external_actor_ref: ExternalActorRef,
    user_id: UserId,
}

/// Product workflow binding service backed by the canonical conversations
/// service. Tenant selection comes only from trusted installation config.
#[derive(Clone)]
pub struct ProductConversationBindingService {
    conversations: Arc<dyn ironclaw_conversations::ConversationBindingService>,
    actor_pairings: Option<Arc<dyn ironclaw_conversations::ConversationActorPairingService>>,
    installations: StaticProductInstallationResolver,
    preconfigured_pairing_cache: Arc<Mutex<HashSet<PreconfiguredActorCacheKey>>>,
}

impl ProductConversationBindingService {
    pub fn new(
        conversations: Arc<dyn ironclaw_conversations::ConversationBindingService>,
        installations: StaticProductInstallationResolver,
    ) -> Self {
        Self {
            conversations,
            actor_pairings: None,
            installations,
            preconfigured_pairing_cache: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn with_actor_pairings(
        mut self,
        actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    ) -> Self {
        self.actor_pairings = Some(actor_pairings);
        self
    }

    async fn apply_preconfigured_actor_binding(
        &self,
        installation_scope: &ProductInstallationScope,
        request: &ResolveBindingRequest,
    ) -> Result<(), ProductWorkflowError> {
        let Some(user_id) = ensure_preconfigured_actor_allowed(installation_scope, request)? else {
            return Ok(());
        };
        let cache_key = preconfigured_actor_cache_key(request, user_id.clone());
        if self
            .preconfigured_pairing_cache
            .lock()
            .map_err(|_| ProductWorkflowError::BindingResolutionFailed {
                reason: "preconfigured actor binding cache lock poisoned".into(),
            })?
            .contains(&cache_key)
        {
            return Ok(());
        }
        let Some(actor_pairings) = &self.actor_pairings else {
            return Err(ProductWorkflowError::BindingResolutionFailed {
                reason:
                    "preconfigured actor binding policy requires a conversation pairing service"
                        .into(),
            });
        };
        actor_pairings
            .pair_external_actor(
                installation_scope.tenant_id.clone(),
                conversation_adapter_kind(&request.adapter_id)?,
                conversation_installation_id(&request.installation_id)?,
                conversation_actor_ref(&request.external_actor_ref)?,
                user_id.clone(),
            )
            .await
            .map_err(map_conversation_error)?;
        self.preconfigured_pairing_cache
            .lock()
            .map_err(|_| ProductWorkflowError::BindingResolutionFailed {
                reason: "preconfigured actor binding cache lock poisoned".into(),
            })?
            .insert(cache_key);
        Ok(())
    }
}

fn preconfigured_actor_cache_key(
    request: &ResolveBindingRequest,
    user_id: UserId,
) -> PreconfiguredActorCacheKey {
    PreconfiguredActorCacheKey {
        adapter_id: request.adapter_id.clone(),
        installation_id: request.installation_id.clone(),
        external_actor_ref: request.external_actor_ref.clone(),
        user_id,
    }
}

fn ensure_preconfigured_actor_allowed<'a>(
    installation_scope: &'a ProductInstallationScope,
    request: &ResolveBindingRequest,
) -> Result<Option<&'a UserId>, ProductWorkflowError> {
    let ProductActorBindingPolicy::PreconfiguredOnly(bindings) =
        &installation_scope.actor_binding_policy
    else {
        return Ok(None);
    };
    bindings
        .get(&request.external_actor_ref)
        .map(Some)
        .ok_or_else(|| ProductWorkflowError::BindingRequired {
            reason: "external actor is not preconfigured for this adapter installation".into(),
        })
}

fn ensure_resolved_actor_matches_preconfigured(
    expected_user_id: Option<&UserId>,
    resolution: &ironclaw_conversations::ConversationBindingResolution,
) -> Result<(), ProductWorkflowError> {
    if let Some(expected_user_id) = expected_user_id
        && &resolution.actor.user_id != expected_user_id
    {
        return Err(ProductWorkflowError::BindingAccessDenied);
    }
    Ok(())
}

#[async_trait]
impl ConversationBindingService for ProductConversationBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let installation_scope = self
            .installations
            .resolve(&request.adapter_id, &request.installation_id)?;
        let expected_user_id =
            ensure_preconfigured_actor_allowed(&installation_scope, &request)?.cloned();
        if expected_user_id.is_some() {
            self.apply_preconfigured_actor_binding(&installation_scope, &request)
                .await?;
        }
        let resolution = self
            .conversations
            .resolve_or_create_binding_with_trusted_scope(
                conversation_request(&request, installation_scope.tenant_id.clone())?,
                installation_scope.default_agent_id.clone(),
                installation_scope.default_project_id.clone(),
            )
            .await
            .map_err(map_conversation_error)?;
        ensure_resolved_actor_matches_preconfigured(expected_user_id.as_ref(), &resolution)?;

        Ok(resolved_binding_from_resolution(resolution))
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let installation_scope = self
            .installations
            .resolve(&request.adapter_id, &request.installation_id)?;
        let expected_user_id =
            ensure_preconfigured_actor_allowed(&installation_scope, &request)?.cloned();
        let resolution = self
            .conversations
            .lookup_binding(conversation_request(
                &request,
                installation_scope.tenant_id,
            )?)
            .await
            .map_err(map_conversation_error)?;
        ensure_resolved_actor_matches_preconfigured(expected_user_id.as_ref(), &resolution)?;

        Ok(resolved_binding_from_resolution(resolution))
    }
}

fn resolved_binding_from_resolution(
    resolution: ironclaw_conversations::ConversationBindingResolution,
) -> ResolvedBinding {
    ResolvedBinding {
        tenant_id: resolution.tenant_id,
        user_id: resolution.actor.user_id,
        thread_id: resolution.turn_scope.thread_id,
        agent_id: resolution.turn_scope.agent_id,
        project_id: resolution.turn_scope.project_id,
    }
}

fn conversation_request(
    request: &ResolveBindingRequest,
    tenant_id: TenantId,
) -> Result<ironclaw_conversations::ResolveConversationRequest, ProductWorkflowError> {
    Ok(ironclaw_conversations::ResolveConversationRequest {
        tenant_id,
        adapter_kind: conversation_adapter_kind(&request.adapter_id)?,
        adapter_installation_id: conversation_installation_id(&request.installation_id)?,
        external_actor_ref: conversation_actor_ref(&request.external_actor_ref)?,
        external_conversation_ref: conversation_conversation_ref(
            &request.external_conversation_ref,
        )?,
        external_event_id: conversation_event_id(&request.external_event_id)?,
        route_kind: conversation_route_kind(request.route_kind),
        requested_agent_id: None,
        requested_project_id: None,
    })
}

fn conversation_adapter_kind(
    adapter_id: &ProductAdapterId,
) -> Result<ironclaw_conversations::AdapterKind, ProductWorkflowError> {
    ironclaw_conversations::AdapterKind::new(adapter_id.as_str()).map_err(map_conversation_error)
}

fn conversation_installation_id(
    installation_id: &AdapterInstallationId,
) -> Result<ironclaw_conversations::AdapterInstallationId, ProductWorkflowError> {
    ironclaw_conversations::AdapterInstallationId::new(installation_id.as_str())
        .map_err(map_conversation_error)
}

fn conversation_event_id(
    event_id: &ironclaw_product_adapters::ExternalEventId,
) -> Result<ironclaw_conversations::ExternalEventId, ProductWorkflowError> {
    ironclaw_conversations::ExternalEventId::new(event_id.as_str()).map_err(map_conversation_error)
}

fn conversation_actor_ref(
    actor_ref: &ironclaw_product_adapters::ExternalActorRef,
) -> Result<ironclaw_conversations::ExternalActorRef, ProductWorkflowError> {
    ironclaw_conversations::ExternalActorRef::new(actor_ref.kind(), actor_ref.id())
        .map_err(map_conversation_error)
}

fn conversation_conversation_ref(
    conversation_ref: &ironclaw_product_adapters::ExternalConversationRef,
) -> Result<ironclaw_conversations::ExternalConversationRef, ProductWorkflowError> {
    ironclaw_conversations::ExternalConversationRef::new(
        conversation_ref.space_id(),
        conversation_ref.conversation_id(),
        conversation_ref.topic_id(),
        conversation_ref.reply_target_message_id(),
    )
    .map_err(map_conversation_error)
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

fn map_conversation_error(error: ironclaw_conversations::InboundTurnError) -> ProductWorkflowError {
    match error {
        ironclaw_conversations::InboundTurnError::InvalidExternalRef { reason, .. }
        | ironclaw_conversations::InboundTurnError::InvalidCanonicalRef { reason } => {
            ProductWorkflowError::InvalidBindingRequest { reason }
        }
        ironclaw_conversations::InboundTurnError::BindingRequired { .. } => {
            ProductWorkflowError::BindingRequired {
                reason: "external actor is not paired with a canonical user".into(),
            }
        }
        ironclaw_conversations::InboundTurnError::AccessDenied { .. }
        | ironclaw_conversations::InboundTurnError::BindingConflict { .. }
        | ironclaw_conversations::InboundTurnError::ThreadNotFound { .. } => {
            ProductWorkflowError::BindingAccessDenied
        }
        ironclaw_conversations::InboundTurnError::StatePoisoned
        | ironclaw_conversations::InboundTurnError::DurableState { .. } => {
            ProductWorkflowError::Transient {
                reason: "conversation binding store unavailable".into(),
            }
        }
        ironclaw_conversations::InboundTurnError::TurnSubmissionFailed { error } => {
            ProductWorkflowError::TurnSubmissionFailed { error }
        }
    }
}
