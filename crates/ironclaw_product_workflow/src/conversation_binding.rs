//! Adapter from product workflow binding requests to `ironclaw_conversations`.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_conversations::TrustedOwnerScope;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterId,
};

use crate::{
    ConversationBindingService, ProductConversationRouteKind, ProductWorkflowError,
    ResolveBindingRequest, ResolvedBinding,
};

const RESOLVED_ACTOR_PAIRING_CACHE_LIMIT: usize = 50_000;

/// Tenant-scoped installation identity used before external actor/conversation
/// refs enter the conversation binding layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductInstallationKey {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
}

/// Stable conversation route key used by hosts to assign shared-route subjects.
///
/// The key intentionally ignores topic/thread ids. For Slack this maps to
/// `(team_id, channel_id)`, so each Slack thread in a configured channel runs
/// under the same shared subject while retaining its own conversation context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductConversationRouteKey {
    space_id: Option<String>,
    conversation_id: String,
}

impl ProductConversationRouteKey {
    pub fn new(
        space_id: Option<String>,
        conversation_id: String,
    ) -> Result<Self, ProductWorkflowError> {
        ExternalConversationRef::new(space_id.as_deref(), conversation_id.as_str(), None, None)
            .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("invalid conversation route key: {error}"),
            })?;
        Ok(Self {
            space_id,
            conversation_id,
        })
    }

    pub fn space_id(&self) -> Option<&str> {
        self.space_id.as_deref()
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    fn from_external_conversation_ref(conversation_ref: &ExternalConversationRef) -> Self {
        Self {
            space_id: conversation_ref.space_id().map(str::to_string),
            conversation_id: conversation_ref.conversation_id().to_string(),
        }
    }
}

impl ProductInstallationKey {
    pub fn new(adapter_id: ProductAdapterId, installation_id: AdapterInstallationId) -> Self {
        Self {
            adapter_id,
            installation_id,
        }
    }
}

/// Request passed to host-owned actor-to-user resolvers before the workflow
/// writes a conversation pairing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductActorUserResolutionRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
}

impl ProductActorUserResolutionRequest {
    pub fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            external_actor_ref,
        }
    }
}

#[async_trait]
pub trait ProductActorUserResolver: Send + Sync {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError>;
}

/// Request passed to host-owned shared-route subject resolvers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductConversationSubjectRouteResolutionRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub route_key: ProductConversationRouteKey,
}

impl ProductConversationSubjectRouteResolutionRequest {
    fn from_binding_request(request: &ResolveBindingRequest) -> Self {
        Self {
            adapter_id: request.adapter_id.clone(),
            installation_id: request.installation_id.clone(),
            route_key: ProductConversationRouteKey::from_external_conversation_ref(
                &request.external_conversation_ref,
            ),
        }
    }
}

#[async_trait]
pub trait ProductConversationSubjectRouteResolver: Send + Sync + std::fmt::Debug {
    async fn resolve_product_conversation_subject_route(
        &self,
        request: ProductConversationSubjectRouteResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError>;
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
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        Ok(self.bindings.get(&request.external_actor_ref).cloned())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnroutedSharedConversationSubjectPolicy {
    UseDefaultSubject,
    RequireConfiguredRoute,
}

/// Trusted host configuration for one adapter installation.
#[derive(Debug, Clone)]
pub struct ProductInstallationScope {
    pub tenant_id: TenantId,
    pub default_agent_id: Option<AgentId>,
    pub default_project_id: Option<ProjectId>,
    pub default_subject_user_id: Option<UserId>,
    pub unrouted_shared_conversation_subject_policy: UnroutedSharedConversationSubjectPolicy,
    pub conversation_subject_routes: HashMap<ProductConversationRouteKey, UserId>,
    pub conversation_subject_route_resolver:
        Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
    pub actor_binding_policy: ProductActorBindingPolicy,
}

impl ProductInstallationScope {
    pub fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            default_agent_id: None,
            default_project_id: None,
            default_subject_user_id: None,
            unrouted_shared_conversation_subject_policy:
                UnroutedSharedConversationSubjectPolicy::UseDefaultSubject,
            conversation_subject_routes: HashMap::new(),
            conversation_subject_route_resolver: None,
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
            default_subject_user_id: None,
            unrouted_shared_conversation_subject_policy:
                UnroutedSharedConversationSubjectPolicy::UseDefaultSubject,
            conversation_subject_routes: HashMap::new(),
            conversation_subject_route_resolver: None,
            actor_binding_policy: ProductActorBindingPolicy::default(),
        }
    }

    pub fn with_default_subject_user_id(mut self, subject_user_id: UserId) -> Self {
        self.default_subject_user_id = Some(subject_user_id);
        self
    }

    pub fn without_default_subject_for_unrouted_shared_conversations(mut self) -> Self {
        self.unrouted_shared_conversation_subject_policy =
            UnroutedSharedConversationSubjectPolicy::RequireConfiguredRoute;
        self
    }

    pub fn with_conversation_subject_route(
        mut self,
        route_key: ProductConversationRouteKey,
        subject_user_id: UserId,
    ) -> Self {
        self.conversation_subject_routes
            .insert(route_key, subject_user_id);
        self
    }

    pub fn with_conversation_subject_route_resolver(
        mut self,
        resolver: Arc<dyn ProductConversationSubjectRouteResolver>,
    ) -> Self {
        self.conversation_subject_route_resolver = Some(resolver);
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

    async fn shared_subject_user_id_for(
        &self,
        request: &ResolveBindingRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if let Some(resolver) = &self.conversation_subject_route_resolver
            && let Some(subject_user_id) = resolver
                .resolve_product_conversation_subject_route(
                    ProductConversationSubjectRouteResolutionRequest::from_binding_request(request),
                )
                .await?
        {
            return Ok(Some(subject_user_id));
        }
        let route_key = ProductConversationRouteKey::from_external_conversation_ref(
            &request.external_conversation_ref,
        );
        if route_key.space_id.is_none() && !self.conversation_subject_routes.is_empty() {
            tracing::warn!(
                "conversation ref has no space_id; channel route lookup will not match configured routes"
            );
        }
        if let Some(subject_user_id) = self.conversation_subject_routes.get(&route_key) {
            return Ok(Some(subject_user_id.clone()));
        }
        match self.unrouted_shared_conversation_subject_policy {
            UnroutedSharedConversationSubjectPolicy::UseDefaultSubject => {
                Ok(self.default_subject_user_id.clone())
            }
            UnroutedSharedConversationSubjectPolicy::RequireConfiguredRoute => Ok(None),
        }
    }

    async fn configured_subject_user_id_for_route(
        &self,
        request: &ResolveBindingRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        match request.route_kind {
            ProductConversationRouteKind::Direct => Ok(None),
            ProductConversationRouteKind::Shared => self.shared_subject_user_id_for(request).await,
        }
    }

    fn requires_current_subject_route_for_existing_shared_binding(&self) -> bool {
        self.conversation_subject_route_resolver.is_some()
            && self.unrouted_shared_conversation_subject_policy
                == UnroutedSharedConversationSubjectPolicy::RequireConfiguredRoute
    }

    async fn current_subject_for_existing_shared_binding(
        &self,
        request: &ResolveBindingRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if request.route_kind != ProductConversationRouteKind::Shared
            || !self.requires_current_subject_route_for_existing_shared_binding()
        {
            return Ok(None);
        }
        let configured_subject_user_id = self.configured_subject_user_id_for_route(request).await?;
        ensure_shared_route_has_configured_subject(
            request.route_kind,
            configured_subject_user_id.as_ref(),
        )?;
        Ok(configured_subject_user_id)
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
    ) -> Result<Arc<ProductInstallationScope>, ProductWorkflowError> {
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
struct ResolvedActorCacheKey {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    external_actor_ref: ExternalActorRef,
    user_id: UserId,
}

#[derive(Debug, Default)]
struct ResolvedActorPairingCache {
    set: HashSet<ResolvedActorCacheKey>,
    order: VecDeque<ResolvedActorCacheKey>,
}

impl ResolvedActorPairingCache {
    fn contains(&self, key: &ResolvedActorCacheKey) -> bool {
        self.set.contains(key)
    }

    fn insert(&mut self, key: ResolvedActorCacheKey) {
        if !self.set.insert(key.clone()) {
            return;
        }
        self.order.push_back(key);
        while self.set.len() > RESOLVED_ACTOR_PAIRING_CACHE_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
    }
}

/// Product workflow binding service backed by the canonical conversations
/// service. Tenant selection comes only from trusted installation config.
#[derive(Clone)]
pub struct ProductConversationBindingService {
    conversations: Arc<dyn ironclaw_conversations::ConversationBindingService>,
    installations: StaticProductInstallationResolver,
    resolved_actor_pairing_cache: Arc<Mutex<ResolvedActorPairingCache>>,
}

impl ProductConversationBindingService {
    pub fn new(
        conversations: Arc<dyn ironclaw_conversations::ConversationBindingService>,
        installations: StaticProductInstallationResolver,
    ) -> Self {
        Self {
            conversations,
            installations,
            resolved_actor_pairing_cache: Arc::new(
                Mutex::new(ResolvedActorPairingCache::default()),
            ),
        }
    }

    async fn apply_resolved_actor_binding(
        &self,
        installation_scope: &ProductInstallationScope,
        request: &ResolveBindingRequest,
        user_id: &UserId,
    ) -> Result<(), ProductWorkflowError> {
        let cache_key = resolved_actor_cache_key(request, user_id.clone());
        if self
            .resolved_actor_pairing_cache
            .lock()
            .map_err(|_| ProductWorkflowError::BindingResolutionFailed {
                reason: "resolved actor binding cache lock poisoned".into(),
            })?
            .contains(&cache_key)
        {
            return Ok(());
        };
        let ProductActorBindingPolicy::ResolveActor { actor_pairings, .. } =
            &installation_scope.actor_binding_policy
        else {
            return Ok(());
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
        self.resolved_actor_pairing_cache
            .lock()
            .map_err(|_| ProductWorkflowError::BindingResolutionFailed {
                reason: "resolved actor binding cache lock poisoned".into(),
            })?
            .insert(cache_key);
        Ok(())
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

fn resolved_actor_cache_key(
    request: &ResolveBindingRequest,
    user_id: UserId,
) -> ResolvedActorCacheKey {
    ResolvedActorCacheKey {
        adapter_id: request.adapter_id.clone(),
        installation_id: request.installation_id.clone(),
        external_actor_ref: request.external_actor_ref.clone(),
        user_id,
    }
}

async fn resolve_actor_user(
    installation_scope: &ProductInstallationScope,
    request: &ResolveBindingRequest,
) -> Result<Option<UserId>, ProductWorkflowError> {
    match &installation_scope.actor_binding_policy {
        ProductActorBindingPolicy::ExistingConversationPairings => Ok(None),
        ProductActorBindingPolicy::ResolveActor { resolver, .. } => resolver
            .resolve_product_actor_user(actor_user_resolution_request(request))
            .await?
            .map(Some)
            .ok_or_else(|| ProductWorkflowError::BindingRequired {
                reason: "external actor is not bound for this adapter installation".into(),
            }),
    }
}

fn ensure_resolved_actor_matches_expected_user(
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
        let conversation_request =
            conversation_request(&request, installation_scope.tenant_id.clone())?;
        if request.route_kind == ProductConversationRouteKind::Shared
            && installation_scope
                .conversation_subject_route_resolver
                .is_some()
        {
            match self
                .conversations
                .lookup_binding(conversation_request.clone())
                .await
            {
                Ok(resolution) if resolution.turn_scope.explicit_owner_user_id().is_some() => {
                    let current_subject_user_id = installation_scope
                        .current_subject_for_existing_shared_binding(&request)
                        .await?;
                    ensure_existing_shared_binding_matches_current_subject(
                        current_subject_user_id.as_ref(),
                        &resolution,
                    )?;
                    let owner_user_id = resolution.turn_scope.explicit_owner_user_id().cloned();
                    let trusted_owner = owner_user_id
                        .map(TrustedOwnerScope::User)
                        .unwrap_or(TrustedOwnerScope::Unspecified);
                    let expected_user_id =
                        resolve_actor_user(&installation_scope, &request).await?;
                    if let Some(user_id) = expected_user_id.as_ref() {
                        self.apply_resolved_actor_binding(&installation_scope, &request, user_id)
                            .await?;
                    }
                    let resolution = self
                        .conversations
                        .resolve_or_create_binding_with_trusted_scope(
                            conversation_request,
                            installation_scope.default_agent_id.clone(),
                            installation_scope.default_project_id.clone(),
                            trusted_owner,
                        )
                        .await
                        .map_err(map_conversation_error)?;
                    ensure_resolved_actor_matches_expected_user(
                        expected_user_id.as_ref(),
                        &resolution,
                    )?;

                    return resolved_binding_from_resolution(resolution, request.route_kind);
                }
                Ok(_) | Err(ironclaw_conversations::InboundTurnError::BindingRequired { .. }) => {}
                Err(error) => return Err(map_conversation_error(error)),
            }
        }
        let configured_subject_user_id = installation_scope
            .configured_subject_user_id_for_route(&request)
            .await?;
        ensure_shared_route_has_configured_subject(
            request.route_kind,
            configured_subject_user_id.as_ref(),
        )?;
        let expected_user_id = resolve_actor_user(&installation_scope, &request).await?;
        if let Some(user_id) = expected_user_id.as_ref() {
            self.apply_resolved_actor_binding(&installation_scope, &request, user_id)
                .await?;
        }
        let trusted_owner_for_subject = configured_subject_user_id
            .clone()
            .map(TrustedOwnerScope::User)
            .unwrap_or(TrustedOwnerScope::Unspecified);
        let resolution = self
            .conversations
            .resolve_or_create_binding_with_trusted_scope(
                conversation_request,
                installation_scope.default_agent_id.clone(),
                installation_scope.default_project_id.clone(),
                trusted_owner_for_subject,
            )
            .await
            .map_err(map_conversation_error)?;
        ensure_resolved_actor_matches_expected_user(expected_user_id.as_ref(), &resolution)?;

        resolved_binding_from_resolution(resolution, request.route_kind)
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let installation_scope = self
            .installations
            .resolve(&request.adapter_id, &request.installation_id)?;
        let conversation_request =
            conversation_request(&request, installation_scope.tenant_id.clone())?;
        let resolution = self
            .conversations
            .lookup_binding(conversation_request)
            .await
            .map_err(map_conversation_error)?;
        if request.route_kind == ProductConversationRouteKind::Shared {
            let current_subject_user_id = installation_scope
                .current_subject_for_existing_shared_binding(&request)
                .await?;
            ensure_existing_shared_binding_matches_current_subject(
                current_subject_user_id.as_ref(),
                &resolution,
            )?;
            let expected_user_id = resolve_actor_user(&installation_scope, &request).await?;
            ensure_resolved_actor_matches_expected_user(expected_user_id.as_ref(), &resolution)?;
        }

        resolved_binding_from_resolution(resolution, request.route_kind)
    }
}

fn ensure_existing_shared_binding_matches_current_subject(
    current_subject_user_id: Option<&UserId>,
    resolution: &ironclaw_conversations::ConversationBindingResolution,
) -> Result<(), ProductWorkflowError> {
    let Some(current_subject_user_id) = current_subject_user_id else {
        return Ok(());
    };
    if resolution.turn_scope.explicit_owner_user_id() != Some(current_subject_user_id) {
        return Err(ProductWorkflowError::BindingAccessDenied);
    }
    Ok(())
}

fn resolved_binding_from_resolution(
    resolution: ironclaw_conversations::ConversationBindingResolution,
    route_kind: ProductConversationRouteKind,
) -> Result<ResolvedBinding, ProductWorkflowError> {
    let actor_user_id = resolution.actor.user_id;
    let subject_user_id = match route_kind {
        ProductConversationRouteKind::Direct => Some(actor_user_id.clone()),
        ProductConversationRouteKind::Shared => Some(
            resolution
                .turn_scope
                .explicit_owner_user_id()
                .cloned()
                .ok_or_else(shared_route_missing_persisted_subject_error)?,
        ),
    };
    Ok(ResolvedBinding {
        tenant_id: resolution.tenant_id,
        actor_user_id,
        subject_user_id,
        thread_id: resolution.turn_scope.thread_id,
        agent_id: resolution.turn_scope.agent_id,
        project_id: resolution.turn_scope.project_id,
    })
}

fn ensure_shared_route_has_configured_subject(
    route_kind: ProductConversationRouteKind,
    configured_subject_user_id: Option<&UserId>,
) -> Result<(), ProductWorkflowError> {
    if route_kind == ProductConversationRouteKind::Shared && configured_subject_user_id.is_none() {
        return Err(shared_route_requires_subject_error());
    }
    Ok(())
}

fn shared_route_requires_subject_error() -> ProductWorkflowError {
    ProductWorkflowError::BindingRequired {
        reason: "shared product route requires a configured subject user".into(),
    }
}

fn shared_route_missing_persisted_subject_error() -> ProductWorkflowError {
    ProductWorkflowError::BindingAccessDenied
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
