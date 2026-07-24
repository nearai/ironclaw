// arch-exempt: large_file, targeted durable delivery state remains with the existing conversation store pending its planned split, plan #6175
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use tokio::sync::Mutex as AsyncMutex;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, SourceBindingRef,
    SubmitTurnResponse, TurnActor, TurnScope,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageLookup,
    AcceptedInboundMessageReplay, AdapterInstallationId, AdapterKind, ConditionalUnpairOutcome,
    ConversationActorPairingService, ConversationBindingResolution, ConversationBindingService,
    ConversationRouteKind, ExpectedExternalActorOwner, ExternalActorBindingEpoch, ExternalActorRef,
    ExternalConversationIdentity, ExternalConversationRef, InboundTurnError,
    LinkConversationRequest, LinkedConversationBinding, MessageIdempotencyStatus,
    ReplyTargetBinding, ResolveConversationRequest, ResolveStoredReplyTargetRequest,
    SessionThreadService, StoredReplyTargetAccess, StoredReplyTargetBinding, ThreadAccessDecision,
    ThreadMessageRecord, ValidateReplyTargetRequest,
};

#[derive(Clone)]
pub struct InMemoryConversationServices {
    state: Arc<Mutex<InMemoryState>>,
    state_repository: Option<Arc<dyn crate::state_store::ConversationStateRepository>>,
    mutation_lock: Arc<AsyncMutex<()>>,
}

impl Default for InMemoryConversationServices {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(InMemoryState::default())),
            state_repository: None,
            mutation_lock: Arc::new(AsyncMutex::new(())),
        }
    }
}

impl InMemoryConversationServices {
    pub(crate) async fn with_state_repository(
        state_repository: Arc<dyn crate::state_store::ConversationStateRepository>,
    ) -> Result<Self, InboundTurnError> {
        let persisted = state_repository.load_state().await?;
        let mut state = persisted.state;
        state.persistence_revision = persisted.revision;
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            state_repository: Some(state_repository),
            mutation_lock: Arc::new(AsyncMutex::new(())),
        })
    }

    async fn refresh_state_from_repository(&self) -> Result<(), InboundTurnError> {
        let Some(state_repository) = &self.state_repository else {
            return Ok(());
        };
        let persisted = state_repository.load_state().await?;
        let mut state = persisted.state;
        state.persistence_revision = persisted.revision;
        *self.lock_state()? = state;
        Ok(())
    }

    async fn persist_state(
        &self,
        old_state: InMemoryState,
        new_state: InMemoryState,
    ) -> Result<(), InboundTurnError> {
        let mut new_state = new_state;
        let Some(state_repository) = &self.state_repository else {
            return Ok(());
        };
        match state_repository
            .save_state(new_state.persistence_revision, &new_state)
            .await
        {
            Ok(revision) => {
                new_state.persistence_revision = revision;
                *self.lock_state()? = new_state;
                Ok(())
            }
            Err(error) => {
                *self.lock_state()? = old_state;
                Err(error)
            }
        }
    }
    pub async fn pair_external_actor(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
    ) {
        let _ = self
            .try_pair_external_actor(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
                user_id,
            )
            .await;
    }

    pub async fn try_pair_external_actor(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
    ) -> Result<(), InboundTurnError> {
        self.try_pair_external_actor_inner(
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            user_id,
            None,
        )
        .await
    }

    pub async fn pair_external_actor_with_epoch(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
        binding_epoch: ExternalActorBindingEpoch,
    ) -> Result<(), InboundTurnError> {
        self.try_pair_external_actor_inner(
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            user_id,
            Some(binding_epoch),
        )
        .await
    }

    async fn try_pair_external_actor_inner(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
        binding_epoch: Option<ExternalActorBindingEpoch>,
    ) -> Result<(), InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let snapshot = {
            let mut state = self.lock_state()?;
            let actor_key = ActorKey::new(
                &tenant_id,
                &adapter_kind,
                &adapter_installation_id,
                &external_actor_ref,
            );
            if state.pairings.get(&actor_key) == Some(&user_id)
                && state.pairing_epochs.get(&actor_key) == binding_epoch.as_ref()
            {
                return Ok(());
            }
            state.pairings.insert(actor_key.clone(), user_id);
            match binding_epoch {
                Some(binding_epoch) => {
                    state.pairing_epochs.insert(actor_key, binding_epoch);
                }
                None => {
                    state.pairing_epochs.remove(&actor_key);
                }
            }
            state.clone()
        };
        self.persist_state(old_state, snapshot).await
    }

    pub async fn accepted_messages(&self) -> Vec<ThreadMessageRecord> {
        match self.state.lock() {
            Ok(state) => state.messages.clone(),
            Err(_) => Vec::new(),
        }
    }

    pub async fn unpair_external_actor(
        &self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
    ) {
        let _ = self
            .try_unpair_external_actor(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
            )
            .await;
    }

    pub async fn try_unpair_external_actor(
        &self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
    ) -> Result<(), InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let snapshot = {
            let mut state = self.lock_state()?;
            let actor_key = ActorKey::new(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
            );
            state.pairings.remove(&actor_key);
            state.pairing_epochs.remove(&actor_key);
            state.revoke_direct_bindings_for_actor(&actor_key);
            state.clone()
        };
        self.persist_state(old_state, snapshot).await
    }

    pub async fn unpair_external_actor_if_owned_by(
        &self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
        expected: &ExpectedExternalActorOwner,
    ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let actor_key = ActorKey::new(
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
        );
        let snapshot = {
            let mut state = self.lock_state()?;
            let Some(current_user_id) = state.pairings.get(&actor_key) else {
                return Ok(ConditionalUnpairOutcome::AlreadyAbsent);
            };
            if current_user_id != &expected.user_id
                || state.pairing_epochs.get(&actor_key) != expected.binding_epoch.as_ref()
            {
                return Ok(ConditionalUnpairOutcome::OwnerChanged);
            }
            state.pairings.remove(&actor_key);
            state.pairing_epochs.remove(&actor_key);
            state.revoke_direct_bindings_for_actor(&actor_key);
            state.clone()
        };
        self.persist_state(old_state, snapshot).await?;
        Ok(ConditionalUnpairOutcome::Unpaired)
    }

    /// Remove every external actor pairing owned by `user_id` for one
    /// adapter, optionally narrowed to a specific adapter installation.
    ///
    /// Channel lifecycle removal does not always retain the provider's actor
    /// id, but it still has host-trusted tenant, adapter, installation, and
    /// user scope. Keeping this bulk operation in the conversation store lets
    /// removal revoke the same direct conversation routes as actor-specific
    /// unpairing without parsing provider identity strings in composition.
    pub async fn unpair_external_actors_owned_by(
        &self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: Option<&AdapterInstallationId>,
        user_id: &UserId,
    ) -> Result<usize, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let (snapshot, removed_count) = {
            let mut state = self.lock_state()?;
            let actor_keys = state
                .pairings
                .iter()
                .filter(|(actor_key, paired_user_id)| {
                    actor_key.tenant_id == *tenant_id
                        && actor_key.adapter_kind == *adapter_kind
                        && adapter_installation_id
                            .is_none_or(|expected| actor_key.adapter_installation_id == *expected)
                        && *paired_user_id == user_id
                })
                .map(|(actor_key, _)| actor_key.clone())
                .collect::<Vec<_>>();
            for actor_key in &actor_keys {
                state.pairings.remove(actor_key);
                state.pairing_epochs.remove(actor_key);
                state.revoke_direct_bindings_for_actor(actor_key);
            }
            (state.clone(), actor_keys.len())
        };
        if removed_count == 0 {
            return Ok(0);
        }
        self.persist_state(old_state, snapshot).await?;
        Ok(removed_count)
    }

    pub async fn add_thread_participant(
        &self,
        tenant_id: &TenantId,
        thread_id: &ThreadId,
        user_id: UserId,
    ) -> Result<(), InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let snapshot = {
            let mut state = self.lock_state()?;
            let Some(thread) = state.threads.get_mut(&ThreadKey::new(tenant_id, thread_id)) else {
                return Err(InboundTurnError::ThreadNotFound {
                    thread_id: thread_id.to_string(),
                });
            };
            thread.participants.insert(user_id);
            state.clone()
        };
        self.persist_state(old_state, snapshot).await?;
        Ok(())
    }
}

#[async_trait]
impl ConversationActorPairingService for InMemoryConversationServices {
    async fn pair_external_actor(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
    ) -> Result<(), InboundTurnError> {
        self.try_pair_external_actor(
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            user_id,
        )
        .await
    }

    async fn pair_external_actor_with_epoch(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
        binding_epoch: ExternalActorBindingEpoch,
    ) -> Result<(), InboundTurnError> {
        InMemoryConversationServices::pair_external_actor_with_epoch(
            self,
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            user_id,
            binding_epoch,
        )
        .await
    }

    async fn unpair_external_actor(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
    ) -> Result<(), InboundTurnError> {
        self.try_unpair_external_actor(
            &tenant_id,
            &adapter_kind,
            &adapter_installation_id,
            &external_actor_ref,
        )
        .await
    }

    async fn unpair_external_actor_if_owned_by(
        &self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
        expected: &ExpectedExternalActorOwner,
    ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
        InMemoryConversationServices::unpair_external_actor_if_owned_by(
            self,
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            expected,
        )
        .await
    }
}

#[async_trait]
impl ConversationBindingService for InMemoryConversationServices {
    async fn resolve_or_create_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        self.resolve_or_create_binding_inner(request, None, None, None)
            .await
    }

    async fn resolve_or_create_binding_with_trusted_scope(
        &self,
        request: ResolveConversationRequest,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
        trusted_owner_user_id: Option<UserId>,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        self.resolve_or_create_binding_inner(
            request,
            trusted_agent_id,
            trusted_project_id,
            trusted_owner_user_id,
        )
        .await
    }

    async fn lookup_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let state = self.lock_state()?;
        let actor_user_id = state.resolve_actor(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_actor_ref,
        )?;
        let binding_key = BindingKey::from_request(&request);
        let external_conversation_identity = request.external_conversation_ref.identity();
        state.ensure_external_event_route(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_event_id,
            &external_conversation_identity,
            &actor_user_id,
        )?;
        let route_actor_key = ActorKey::new(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_actor_ref,
        );
        let binding_epoch = state.pairing_epochs.get(&route_actor_key).cloned();
        let binding = state.bindings.get(&binding_key).cloned().ok_or_else(|| {
            InboundTurnError::BindingRequired {
                adapter_kind: request.adapter_kind.as_str().to_string(),
                external_actor_id: request.external_actor_ref.id().to_string(),
            }
        })?;
        state.ensure_participant(&request.tenant_id, &actor_user_id, &binding.thread_id)?;
        if !binding
            .route_access
            .allows(&route_actor_key, request.route_kind)
        {
            return Err(InboundTurnError::AccessDenied {
                actor_id: actor_user_id.to_string(),
                thread_id: binding.thread_id.to_string(),
            });
        }
        Ok(binding.resolution(actor_user_id, binding_epoch, request.tenant_id))
    }

    async fn link_conversation_to_thread(
        &self,
        request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let (linked, snapshot) = {
            let mut state = self.lock_state()?;
            let actor_user_id = state.resolve_actor(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_actor_ref,
            )?;
            let target_thread = state.thread_for_participant(
                &request.tenant_id,
                &actor_user_id,
                &request.target_thread_id,
            )?;
            let binding_key = BindingKey {
                tenant_id: request.tenant_id.clone(),
                adapter_kind: request.adapter_kind.clone(),
                adapter_installation_id: request.adapter_installation_id.clone(),
                external_conversation_identity: request.external_conversation_ref.identity(),
            };
            if state.bindings.contains_key(&binding_key) {
                let existing = state
                    .bindings
                    .get(&binding_key)
                    .cloned()
                    .ok_or(InboundTurnError::StatePoisoned)?;
                if existing.thread_id == request.target_thread_id {
                    let route_actor_key = ActorKey::new(
                        &request.tenant_id,
                        &request.adapter_kind,
                        &request.adapter_installation_id,
                        &request.external_actor_ref,
                    );
                    if !existing
                        .route_access
                        .allows(&route_actor_key, request.route_kind)
                    {
                        return Err(InboundTurnError::AccessDenied {
                            actor_id: actor_user_id.to_string(),
                            thread_id: existing.thread_id.to_string(),
                        });
                    }
                    if request.route_kind == ConversationRouteKind::Shared {
                        state.widen_binding_route_access(&binding_key)?;
                    }
                    let existing = state
                        .bindings
                        .get(&binding_key)
                        .cloned()
                        .ok_or(InboundTurnError::StatePoisoned)?;
                    let linked = LinkedConversationBinding {
                        thread_id: existing.thread_id,
                        source_binding_ref: existing.source_binding_ref,
                        reply_target_binding_ref: existing.reply_target_binding_ref,
                    };
                    (linked, state.clone())
                } else {
                    return Err(InboundTurnError::BindingConflict {
                        thread_id: existing.thread_id.to_string(),
                    });
                }
            } else {
                let route_actor_key = ActorKey::new(
                    &request.tenant_id,
                    &request.adapter_kind,
                    &request.adapter_installation_id,
                    &request.external_actor_ref,
                );
                let binding = BindingRecord::new(
                    request.tenant_id,
                    request.adapter_kind,
                    request.adapter_installation_id,
                    request.external_conversation_ref,
                    ReplyRouteAccess::new(route_actor_key, request.route_kind),
                    BindingTarget::new(
                        request.target_thread_id,
                        target_thread.agent_id,
                        target_thread.project_id,
                        None,
                    ),
                )?;
                let linked = LinkedConversationBinding {
                    thread_id: binding.thread_id.clone(),
                    source_binding_ref: binding.source_binding_ref.clone(),
                    reply_target_binding_ref: binding.reply_target_binding_ref.clone(),
                };
                state.store_binding(binding_key, binding);
                (linked, state.clone())
            }
        };
        self.persist_state(old_state, snapshot).await?;
        Ok(linked)
    }

    async fn validate_reply_target(
        &self,
        request: ValidateReplyTargetRequest,
    ) -> Result<ReplyTargetBinding, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let state = self.lock_state()?;
        let Some(binding) = state
            .reply_targets
            .get(request.reply_target_binding_ref.as_str())
            .cloned()
        else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: request.reply_target_binding_ref.as_str().to_string(),
            });
        };
        let route_actor_key = ActorKey::new(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_actor_ref,
        );
        if binding.tenant_id != request.tenant_id
            || binding.thread_id != request.current_thread_id
            || binding.adapter_kind != request.adapter_kind
            || binding.adapter_installation_id != request.adapter_installation_id
            || !binding
                .route_access
                .allows(&route_actor_key, ConversationRouteKind::Shared)
        {
            return Err(InboundTurnError::AccessDenied {
                actor_id: request.actor_user_id.to_string(),
                thread_id: binding.thread_id.to_string(),
            });
        }
        let paired_user_id = state.resolve_actor(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_actor_ref,
        )?;
        if paired_user_id != request.actor_user_id {
            return Err(InboundTurnError::AccessDenied {
                actor_id: request.actor_user_id.to_string(),
                thread_id: binding.thread_id.to_string(),
            });
        }
        state.ensure_participant(&binding.tenant_id, &paired_user_id, &binding.thread_id)?;
        Ok(ReplyTargetBinding {
            tenant_id: binding.tenant_id,
            actor_user_id: request.actor_user_id,
            thread_id: binding.thread_id,
            adapter_kind: binding.adapter_kind,
            adapter_installation_id: binding.adapter_installation_id,
            external_conversation_ref: binding.external_conversation_ref,
        })
    }

    async fn resolve_stored_reply_target(
        &self,
        request: ResolveStoredReplyTargetRequest,
    ) -> Result<StoredReplyTargetBinding, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let state = self.lock_state()?;
        let Some(binding) = state
            .reply_targets
            .get(request.reply_target_binding_ref.as_str())
            .cloned()
        else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: request.reply_target_binding_ref.as_str().to_string(),
            });
        };
        if binding.tenant_id != request.tenant_id || binding.thread_id != request.current_thread_id
        {
            return Err(InboundTurnError::AccessDenied {
                actor_id: request.actor_user_id.to_string(),
                thread_id: binding.thread_id.to_string(),
            });
        }
        state.ensure_participant(
            &binding.tenant_id,
            &request.actor_user_id,
            &binding.thread_id,
        )?;

        let exact_origin_actor_is_current = state
            .pairings
            .get(&binding.route_access.owner_actor_key)
            .is_some_and(|user_id| user_id == &request.actor_user_id);
        let route_kind = if binding.route_access.shared {
            ConversationRouteKind::Shared
        } else {
            ConversationRouteKind::Direct
        };
        let access_allowed = match request.access {
            StoredReplyTargetAccess::OrdinaryReply => {
                binding.route_access.shared || exact_origin_actor_is_current
            }
            StoredReplyTargetAccess::ExactOriginActor => exact_origin_actor_is_current,
        };
        if !access_allowed {
            return Err(InboundTurnError::AccessDenied {
                actor_id: request.actor_user_id.to_string(),
                thread_id: binding.thread_id.to_string(),
            });
        }

        Ok(StoredReplyTargetBinding {
            tenant_id: binding.tenant_id,
            actor_user_id: request.actor_user_id,
            thread_id: binding.thread_id,
            adapter_kind: binding.adapter_kind,
            adapter_installation_id: binding.adapter_installation_id,
            external_conversation_ref: binding.external_conversation_ref,
            route_kind,
        })
    }
}

impl InMemoryConversationServices {
    async fn resolve_or_create_binding_inner(
        &self,
        request: ResolveConversationRequest,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
        trusted_owner_user_id: Option<UserId>,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let (resolution, snapshot) = {
            let mut state = self.lock_state()?;
            let actor_user_id = state.resolve_actor(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_actor_ref,
            )?;
            let binding_key = BindingKey::from_request(&request);
            let external_conversation_identity = request.external_conversation_ref.identity();
            state.ensure_external_event_route(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_event_id,
                &external_conversation_identity,
                &actor_user_id,
            )?;
            let route_actor_key = ActorKey::new(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_actor_ref,
            );
            let binding_epoch = state.pairing_epochs.get(&route_actor_key).cloned();

            if state.bindings.contains_key(&binding_key) {
                let binding = state
                    .bindings
                    .get(&binding_key)
                    .cloned()
                    .ok_or(InboundTurnError::StatePoisoned)?;
                state.ensure_participant(&request.tenant_id, &actor_user_id, &binding.thread_id)?;
                if !binding
                    .route_access
                    .allows(&route_actor_key, request.route_kind)
                {
                    return Err(InboundTurnError::AccessDenied {
                        actor_id: actor_user_id.to_string(),
                        thread_id: binding.thread_id.to_string(),
                    });
                }
                state.ensure_trusted_scope_not_reinterpreted(
                    &binding,
                    trusted_agent_id.as_ref(),
                    trusted_project_id.as_ref(),
                )?;
                if request.route_kind == ConversationRouteKind::Shared {
                    state.widen_binding_route_access(&binding_key)?;
                    if binding.owner_user_id.is_none()
                        && let Some(owner_user_id) = trusted_owner_user_id.clone()
                    {
                        state.set_binding_owner(&binding_key, owner_user_id)?;
                    }
                }
                state.record_external_event_route(
                    &request.tenant_id,
                    &request.adapter_kind,
                    &request.adapter_installation_id,
                    &request.external_event_id,
                    &external_conversation_identity,
                    &actor_user_id,
                )?;
                let binding = state
                    .bindings
                    .get(&binding_key)
                    .cloned()
                    .ok_or(InboundTurnError::StatePoisoned)?;
                let resolution =
                    binding.resolution(actor_user_id, binding_epoch, request.tenant_id);
                (resolution, state.clone())
            } else {
                let thread_id = ThreadId::new(Uuid::new_v4().to_string()).map_err(|error| {
                    InboundTurnError::InvalidCanonicalRef {
                        reason: error.to_string(),
                    }
                })?;
                let thread = ThreadRecord {
                    agent_id: trusted_agent_id.clone(),
                    project_id: trusted_project_id.clone(),
                    participants: HashSet::from([actor_user_id.clone()]),
                };
                state
                    .threads
                    .insert(ThreadKey::new(&request.tenant_id, &thread_id), thread);
                let binding = BindingRecord::new(
                    request.tenant_id.clone(),
                    request.adapter_kind.clone(),
                    request.adapter_installation_id.clone(),
                    request.external_conversation_ref,
                    ReplyRouteAccess::new(route_actor_key, request.route_kind),
                    BindingTarget::new(
                        thread_id,
                        trusted_agent_id,
                        trusted_project_id,
                        trusted_owner_user_id,
                    ),
                )?;
                let resolution = binding.resolution(
                    actor_user_id.clone(),
                    binding_epoch,
                    request.tenant_id.clone(),
                );
                state.store_binding(binding_key, binding);
                state.record_external_event_route(
                    &request.tenant_id,
                    &request.adapter_kind,
                    &request.adapter_installation_id,
                    &request.external_event_id,
                    &external_conversation_identity,
                    &actor_user_id,
                )?;
                (resolution, state.clone())
            }
        };
        self.persist_state(old_state, snapshot).await?;
        Ok(resolution)
    }
}

#[async_trait]
impl SessionThreadService for InMemoryConversationServices {
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let (accepted, snapshot) = {
            let mut state = self.lock_state()?;
            let paired_user_id = state.resolve_actor(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_actor_ref,
            )?;
            if paired_user_id != request.actor.user_id {
                return Err(InboundTurnError::AccessDenied {
                    actor_id: request.actor.user_id.to_string(),
                    thread_id: request.thread_id.to_string(),
                });
            }
            state.ensure_participant(&request.tenant_id, &paired_user_id, &request.thread_id)?;
            let route_actor_key = ActorKey::new(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_actor_ref,
            );
            let binding_epoch = state.pairing_epochs.get(&route_actor_key).cloned();
            state.ensure_binding_refs_match(BindingRefValidation {
                tenant_id: &request.tenant_id,
                thread_id: &request.thread_id,
                source_binding_ref: request.source_binding_ref.as_str(),
                reply_target_binding_ref: request.reply_target_binding_ref.as_str(),
                actor_user_id: &request.actor.user_id,
                route_actor_key: &route_actor_key,
                route_kind: request.route_kind,
            })?;
            let source_binding = state
                .source_bindings
                .get(request.source_binding_ref.as_str())
                .cloned()
                .ok_or_else(|| InboundTurnError::ThreadNotFound {
                    thread_id: request.source_binding_ref.as_str().to_string(),
                })?;
            let external_conversation_identity = request.external_conversation_ref.identity();
            if source_binding.external_conversation_identity != external_conversation_identity {
                return Err(InboundTurnError::AccessDenied {
                    actor_id: request.actor.user_id.to_string(),
                    thread_id: request.thread_id.to_string(),
                });
            }
            state.ensure_external_event_route(
                &request.tenant_id,
                &source_binding.adapter_kind,
                &source_binding.adapter_installation_id,
                &request.external_event_id,
                &external_conversation_identity,
                &request.actor.user_id,
            )?;
            let replay_key = AcceptedMessageReplayKey::new(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_actor_ref,
                &request.external_event_id,
            );
            let idempotency_key = MessageIdempotencyKey {
                tenant_id: request.tenant_id.clone(),
                source_binding_ref: request.source_binding_ref.as_str().to_string(),
                external_event_id: request.external_event_id.clone(),
            };
            if let Some(existing) = state.message_idempotency.get(&idempotency_key) {
                let mut duplicate = existing.clone();
                duplicate.idempotency = MessageIdempotencyStatus::Duplicate;
                (duplicate, state.clone())
            } else {
                let message_ref = AcceptedMessageRef::new(format!("message:{}", Uuid::new_v4()))
                    .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
                let reply_target_record = state
                    .reply_targets
                    .get(request.reply_target_binding_ref.as_str())
                    .cloned()
                    .ok_or_else(|| InboundTurnError::ThreadNotFound {
                        thread_id: request.reply_target_binding_ref.as_str().to_string(),
                    })?;
                state.record_external_event_route(
                    &request.tenant_id,
                    &source_binding.adapter_kind,
                    &source_binding.adapter_installation_id,
                    &request.external_event_id,
                    &external_conversation_identity,
                    &request.actor.user_id,
                )?;
                let message_reply_target_binding_ref =
                    ReplyTargetBindingRef::new(format!("reply:{}", Uuid::new_v4()))
                        .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
                state.reply_targets.insert(
                    message_reply_target_binding_ref.as_str().to_string(),
                    reply_target_record.with_reply_target(
                        message_reply_target_binding_ref.clone(),
                        request.external_conversation_ref.clone(),
                    ),
                );
                let accepted = AcceptedInboundMessage {
                    tenant_id: request.tenant_id,
                    thread_id: request.thread_id,
                    actor: request.actor.clone(),
                    message_ref,
                    source_binding_ref: request.source_binding_ref,
                    reply_target_binding_ref: message_reply_target_binding_ref,
                    received_at: request.received_at,
                    requested_run_profile: request.requested_run_profile,
                    idempotency: MessageIdempotencyStatus::Inserted,
                };
                state
                    .message_idempotency
                    .insert(idempotency_key, accepted.clone());
                state.message_replays.insert(
                    replay_key,
                    StoredAcceptedMessageReplay {
                        external_conversation_identity,
                        replay: AcceptedInboundMessageReplay {
                            resolution: source_binding.resolution(
                                accepted.actor.user_id.clone(),
                                binding_epoch,
                                accepted.tenant_id.clone(),
                            ),
                            accepted_message: accepted.clone(),
                        },
                    },
                );
                state.messages.push(ThreadMessageRecord {
                    accepted: accepted.clone(),
                    actor: request.actor,
                    external_event_id: request.external_event_id,
                    content_ref: request.content_ref,
                    received_at: request.received_at,
                });
                (accepted, state.clone())
            }
        };
        self.persist_state(old_state, snapshot).await?;
        Ok(accepted)
    }

    async fn replay_accepted_inbound_message(
        &self,
        lookup: AcceptedInboundMessageLookup,
    ) -> Result<Option<AcceptedInboundMessageReplay>, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let state = self.lock_state()?;
        let key = AcceptedMessageReplayKey::new(
            &lookup.tenant_id,
            &lookup.adapter_kind,
            &lookup.adapter_installation_id,
            &lookup.external_actor_ref,
            &lookup.external_event_id,
        );
        let Some(stored) = state.message_replays.get(&key) else {
            return Ok(None);
        };
        if stored.external_conversation_identity != lookup.external_conversation_ref.identity() {
            return Err(InboundTurnError::AccessDenied {
                actor_id: lookup.external_actor_ref.id().to_string(),
                thread_id: "external_event_route_mismatch".to_string(),
            });
        }
        let mut replay = stored.replay.clone();
        replay.accepted_message.idempotency = MessageIdempotencyStatus::Duplicate;
        Ok(Some(replay))
    }

    async fn inbound_message_turn_submission(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<Option<SubmitTurnResponse>, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let state = self.lock_state()?;
        Ok(state.submitted_message_responses.get(message_ref).cloned())
    }

    async fn inbound_message_turn_submission_key(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<IdempotencyKey, InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let maybe_key_and_snapshot = {
            let mut state = self.lock_state()?;
            if let Some(key) = state.submission_keys.get(message_ref).cloned() {
                return Ok(key);
            }
            let key = IdempotencyKey::new(message_ref.as_str().to_string())
                .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
            state
                .submission_keys
                .insert(message_ref.clone(), key.clone());
            (key, state.clone())
        };
        self.persist_state(old_state, maybe_key_and_snapshot.1)
            .await?;
        Ok(maybe_key_and_snapshot.0)
    }

    async fn rotate_inbound_message_turn_submission_key(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<(), InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let snapshot = {
            let mut state = self.lock_state()?;
            state
                .submission_keys
                .insert(message_ref.clone(), state_generated_submission_key()?);
            state.clone()
        };
        self.persist_state(old_state, snapshot).await?;
        Ok(())
    }

    async fn mark_inbound_message_turn_submitted(
        &self,
        message_ref: &AcceptedMessageRef,
        response: SubmitTurnResponse,
    ) -> Result<(), InboundTurnError> {
        let _mutation = self.mutation_lock.lock().await;
        self.refresh_state_from_repository().await?;
        let old_state = self.lock_state()?.clone();
        let snapshot = {
            let mut state = self.lock_state()?;
            state
                .submitted_message_responses
                .insert(message_ref.clone(), response);
            state.clone()
        };
        self.persist_state(old_state, snapshot).await?;
        Ok(())
    }
}

fn state_generated_submission_key() -> Result<IdempotencyKey, InboundTurnError> {
    IdempotencyKey::new(format!("submit:{}", Uuid::new_v4()))
        .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })
}

impl InMemoryConversationServices {
    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, InMemoryState>, InboundTurnError> {
        self.state
            .lock()
            .map_err(|_| InboundTurnError::StatePoisoned)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct InMemoryState {
    #[serde(default, skip)]
    pub(crate) persistence_revision: i64,
    pub(crate) pairings: HashMap<ActorKey, UserId>,
    #[serde(default)]
    pub(crate) pairing_epochs: HashMap<ActorKey, ExternalActorBindingEpoch>,
    pub(crate) bindings: HashMap<BindingKey, BindingRecord>,
    pub(crate) source_bindings: HashMap<String, BindingRecord>,
    pub(crate) reply_targets: HashMap<String, ReplyTargetRecord>,
    pub(crate) threads: HashMap<ThreadKey, ThreadRecord>,
    pub(crate) external_event_routes: HashMap<ExternalEventRouteKey, ExternalConversationIdentity>,
    pub(crate) message_idempotency: HashMap<MessageIdempotencyKey, AcceptedInboundMessage>,
    pub(crate) message_replays: HashMap<AcceptedMessageReplayKey, StoredAcceptedMessageReplay>,
    pub(crate) submission_keys: HashMap<AcceptedMessageRef, IdempotencyKey>,
    pub(crate) submitted_message_responses: HashMap<AcceptedMessageRef, SubmitTurnResponse>,
    pub(crate) messages: Vec<ThreadMessageRecord>,
}

impl InMemoryState {
    fn store_binding(&mut self, binding_key: BindingKey, binding: BindingRecord) {
        self.source_bindings.insert(
            binding.source_binding_ref.as_str().to_string(),
            binding.clone(),
        );
        self.reply_targets.insert(
            binding.reply_target_binding_ref.as_str().to_string(),
            ReplyTargetRecord::from_binding(&binding, binding.external_conversation_ref.clone()),
        );
        self.bindings.insert(binding_key, binding);
    }

    fn revoke_direct_bindings_for_actor(&mut self, actor_key: &ActorKey) {
        let removed_binding_keys: Vec<_> = self
            .bindings
            .iter()
            .filter(|(_, binding)| binding.route_access.is_direct_owner(actor_key))
            .map(|(binding_key, _)| binding_key.clone())
            .collect();
        let mut removed_conversations = std::collections::HashSet::new();
        for binding_key in removed_binding_keys {
            if let Some(binding) = self.bindings.remove(&binding_key) {
                removed_conversations.insert(binding.external_conversation_identity.clone());
                self.source_bindings
                    .remove(binding.source_binding_ref.as_str());
                self.reply_targets
                    .remove(binding.reply_target_binding_ref.as_str());
            }
        }
        self.source_bindings
            .retain(|_, binding| !binding.route_access.is_direct_owner(actor_key));
        self.reply_targets
            .retain(|_, binding| !binding.route_access.is_direct_owner(actor_key));
        self.external_event_routes.retain(|route_key, identity| {
            route_key.tenant_id != actor_key.tenant_id
                || route_key.adapter_kind != actor_key.adapter_kind
                || route_key.adapter_installation_id != actor_key.adapter_installation_id
                || !removed_conversations.contains(identity)
        });
    }

    fn widen_binding_route_access(
        &mut self,
        binding_key: &BindingKey,
    ) -> Result<(), InboundTurnError> {
        let binding = self
            .bindings
            .get_mut(binding_key)
            .ok_or(InboundTurnError::StatePoisoned)?;
        binding.route_access.allow_shared();
        if let Some(source_binding) = self
            .source_bindings
            .get_mut(binding.source_binding_ref.as_str())
        {
            source_binding.route_access.allow_shared();
        }
        if let Some(reply_target) = self
            .reply_targets
            .get_mut(binding.reply_target_binding_ref.as_str())
        {
            reply_target.route_access.allow_shared();
        }
        Ok(())
    }

    fn set_binding_owner(
        &mut self,
        binding_key: &BindingKey,
        owner_user_id: UserId,
    ) -> Result<(), InboundTurnError> {
        let binding = self
            .bindings
            .get_mut(binding_key)
            .ok_or(InboundTurnError::StatePoisoned)?;
        binding.owner_user_id = Some(owner_user_id.clone());
        if let Some(source_binding) = self
            .source_bindings
            .get_mut(binding.source_binding_ref.as_str())
        {
            source_binding.owner_user_id = Some(owner_user_id.clone());
        }
        Ok(())
    }

    fn ensure_trusted_scope_not_reinterpreted(
        &self,
        binding: &BindingRecord,
        trusted_agent_id: Option<&AgentId>,
        trusted_project_id: Option<&ProjectId>,
    ) -> Result<(), InboundTurnError> {
        if (binding.agent_id.is_none() && trusted_agent_id.is_some())
            || (binding.project_id.is_none() && trusted_project_id.is_some())
        {
            return Err(InboundTurnError::BindingConflict {
                thread_id: binding.thread_id.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_binding_refs_match(
        &self,
        validation: BindingRefValidation<'_>,
    ) -> Result<(), InboundTurnError> {
        let Some(source_binding) = self.source_bindings.get(validation.source_binding_ref) else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: validation.source_binding_ref.to_string(),
            });
        };
        let Some(reply_binding) = self.reply_targets.get(validation.reply_target_binding_ref)
        else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: validation.reply_target_binding_ref.to_string(),
            });
        };
        if source_binding.tenant_id != *validation.tenant_id
            || reply_binding.tenant_id != *validation.tenant_id
            || source_binding.thread_id != *validation.thread_id
            || reply_binding.thread_id != *validation.thread_id
            || source_binding.adapter_kind != validation.route_actor_key.adapter_kind
            || reply_binding.adapter_kind != validation.route_actor_key.adapter_kind
            || source_binding.adapter_installation_id
                != validation.route_actor_key.adapter_installation_id
            || reply_binding.adapter_installation_id
                != validation.route_actor_key.adapter_installation_id
            || source_binding.source_binding_ref.as_str() != validation.source_binding_ref
            || source_binding.reply_target_binding_ref.as_str()
                != validation.reply_target_binding_ref
            || reply_binding.reply_target_binding_ref.as_str()
                != validation.reply_target_binding_ref
            || source_binding.source_binding_ref != reply_binding.source_binding_ref
            || !reply_binding
                .route_access
                .allows(validation.route_actor_key, validation.route_kind)
        {
            return Err(InboundTurnError::AccessDenied {
                actor_id: validation.actor_user_id.to_string(),
                thread_id: validation.thread_id.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_external_event_route(
        &self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_event_id: &crate::ExternalEventId,
        external_conversation_identity: &ExternalConversationIdentity,
        actor_user_id: &UserId,
    ) -> Result<(), InboundTurnError> {
        let key = ExternalEventRouteKey::new(
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_event_id,
        );
        if let Some(existing) = self.external_event_routes.get(&key)
            && existing != external_conversation_identity
        {
            return Err(InboundTurnError::AccessDenied {
                actor_id: actor_user_id.to_string(),
                thread_id: "external_event_route_mismatch".to_string(),
            });
        }
        Ok(())
    }

    fn record_external_event_route(
        &mut self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_event_id: &crate::ExternalEventId,
        external_conversation_identity: &ExternalConversationIdentity,
        actor_user_id: &UserId,
    ) -> Result<(), InboundTurnError> {
        self.ensure_external_event_route(
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_event_id,
            external_conversation_identity,
            actor_user_id,
        )?;
        self.external_event_routes.insert(
            ExternalEventRouteKey::new(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_event_id,
            ),
            external_conversation_identity.clone(),
        );
        Ok(())
    }

    fn resolve_actor(
        &self,
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
    ) -> Result<UserId, InboundTurnError> {
        self.pairings
            .get(&ActorKey::new(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
            ))
            .cloned()
            .ok_or_else(|| InboundTurnError::BindingRequired {
                adapter_kind: adapter_kind.as_str().to_string(),
                external_actor_id: external_actor_ref.id().to_string(),
            })
    }

    fn ensure_participant(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
        thread_id: &ThreadId,
    ) -> Result<(), InboundTurnError> {
        self.thread_for_participant(tenant_id, actor_user_id, thread_id)
            .map(|_| ())
    }

    fn thread_for_participant(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
        thread_id: &ThreadId,
    ) -> Result<ThreadRecord, InboundTurnError> {
        let Some(thread) = self.threads.get(&ThreadKey::new(tenant_id, thread_id)) else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: thread_id.to_string(),
            });
        };
        if !thread.participants.contains(actor_user_id) {
            return Err(InboundTurnError::AccessDenied {
                actor_id: actor_user_id.to_string(),
                thread_id: thread_id.to_string(),
            });
        }
        Ok(thread.clone())
    }
}

struct BindingRefValidation<'a> {
    tenant_id: &'a TenantId,
    thread_id: &'a ThreadId,
    source_binding_ref: &'a str,
    reply_target_binding_ref: &'a str,
    actor_user_id: &'a UserId,
    route_actor_key: &'a ActorKey,
    route_kind: ConversationRouteKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ActorKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) external_actor_ref: ExternalActorRef,
}

impl ActorKey {
    pub(crate) fn new(
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
    ) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: adapter_installation_id.clone(),
            external_actor_ref: external_actor_ref.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct BindingKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) external_conversation_identity: ExternalConversationIdentity,
}

impl BindingKey {
    pub(crate) fn from_request(request: &ResolveConversationRequest) -> Self {
        Self {
            tenant_id: request.tenant_id.clone(),
            adapter_kind: request.adapter_kind.clone(),
            adapter_installation_id: request.adapter_installation_id.clone(),
            external_conversation_identity: request.external_conversation_ref.identity(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ExternalEventRouteKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) external_event_id: crate::ExternalEventId,
}

impl ExternalEventRouteKey {
    pub(crate) fn new(
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_event_id: &crate::ExternalEventId,
    ) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: adapter_installation_id.clone(),
            external_event_id: external_event_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ThreadKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) thread_id: ThreadId,
}

impl ThreadKey {
    pub(crate) fn new(tenant_id: &TenantId, thread_id: &ThreadId) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            thread_id: thread_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ThreadRecord {
    pub(crate) agent_id: Option<AgentId>,
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) participants: HashSet<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReplyRouteAccess {
    pub(crate) owner_actor_key: ActorKey,
    pub(crate) shared: bool,
}

impl ReplyRouteAccess {
    pub(crate) fn new(owner_actor_key: ActorKey, route_kind: ConversationRouteKind) -> Self {
        Self {
            owner_actor_key,
            shared: route_kind == ConversationRouteKind::Shared,
        }
    }

    fn allow_shared(&mut self) {
        self.shared = true;
    }

    fn allows(&self, actor_key: &ActorKey, route_kind: ConversationRouteKind) -> bool {
        self.owner_actor_key == *actor_key
            || (self.shared && route_kind == ConversationRouteKind::Shared)
    }

    fn is_direct_owner(&self, actor_key: &ActorKey) -> bool {
        self.owner_actor_key == *actor_key && !self.shared
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReplyTargetRecord {
    pub(crate) tenant_id: TenantId,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) external_conversation_ref: ExternalConversationRef,
    pub(crate) thread_id: ThreadId,
    pub(crate) source_binding_ref: SourceBindingRef,
    pub(crate) reply_target_binding_ref: ReplyTargetBindingRef,
    pub(crate) route_access: ReplyRouteAccess,
}

impl ReplyTargetRecord {
    fn from_binding(
        binding: &BindingRecord,
        external_conversation_ref: ExternalConversationRef,
    ) -> Self {
        Self {
            tenant_id: binding.tenant_id.clone(),
            adapter_kind: binding.adapter_kind.clone(),
            adapter_installation_id: binding.adapter_installation_id.clone(),
            external_conversation_ref,
            thread_id: binding.thread_id.clone(),
            source_binding_ref: binding.source_binding_ref.clone(),
            reply_target_binding_ref: binding.reply_target_binding_ref.clone(),
            route_access: binding.route_access.clone(),
        }
    }

    fn with_reply_target(
        &self,
        reply_target_binding_ref: ReplyTargetBindingRef,
        external_conversation_ref: ExternalConversationRef,
    ) -> Self {
        Self {
            tenant_id: self.tenant_id.clone(),
            adapter_kind: self.adapter_kind.clone(),
            adapter_installation_id: self.adapter_installation_id.clone(),
            external_conversation_ref,
            thread_id: self.thread_id.clone(),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref,
            route_access: self.route_access.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BindingTarget {
    pub(crate) thread_id: ThreadId,
    pub(crate) agent_id: Option<AgentId>,
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) owner_user_id: Option<UserId>,
}

impl BindingTarget {
    pub(crate) fn new(
        thread_id: ThreadId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        owner_user_id: Option<UserId>,
    ) -> Self {
        Self {
            thread_id,
            agent_id,
            project_id,
            owner_user_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BindingRecord {
    pub(crate) tenant_id: TenantId,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) external_conversation_ref: ExternalConversationRef,
    pub(crate) external_conversation_identity: ExternalConversationIdentity,
    pub(crate) thread_id: ThreadId,
    pub(crate) agent_id: Option<AgentId>,
    pub(crate) project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner_user_id: Option<UserId>,
    pub(crate) route_access: ReplyRouteAccess,
    pub(crate) source_binding_ref: SourceBindingRef,
    pub(crate) reply_target_binding_ref: ReplyTargetBindingRef,
}

impl BindingRecord {
    pub(crate) fn new(
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_conversation_ref: ExternalConversationRef,
        route_access: ReplyRouteAccess,
        target: BindingTarget,
    ) -> Result<Self, InboundTurnError> {
        let source_binding_ref = SourceBindingRef::new(format!("source:{}", Uuid::new_v4()))
            .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        let reply_target_binding_ref =
            ReplyTargetBindingRef::new(format!("reply:{}", Uuid::new_v4()))
                .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        let external_conversation_identity = external_conversation_ref.identity();
        Ok(Self {
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_conversation_ref: external_conversation_ref.without_message_id(),
            external_conversation_identity,
            thread_id: target.thread_id,
            agent_id: target.agent_id,
            project_id: target.project_id,
            owner_user_id: target.owner_user_id,
            route_access,
            source_binding_ref,
            reply_target_binding_ref,
        })
    }

    fn resolution(
        &self,
        actor_user_id: UserId,
        binding_epoch: Option<ExternalActorBindingEpoch>,
        tenant_id: TenantId,
    ) -> ConversationBindingResolution {
        let turn_scope = match self.owner_user_id.clone() {
            Some(owner_user_id) => TurnScope::new_with_owner(
                tenant_id.clone(),
                self.agent_id.clone(),
                self.project_id.clone(),
                self.thread_id.clone(),
                Some(owner_user_id),
            ),
            None => TurnScope::new(
                tenant_id.clone(),
                self.agent_id.clone(),
                self.project_id.clone(),
                self.thread_id.clone(),
            ),
        };
        ConversationBindingResolution {
            tenant_id,
            actor: TurnActor::new(actor_user_id),
            binding_epoch,
            turn_scope,
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            access: ThreadAccessDecision::Allowed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredAcceptedMessageReplay {
    pub(crate) external_conversation_identity: ExternalConversationIdentity,
    pub(crate) replay: AcceptedInboundMessageReplay,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct AcceptedMessageReplayKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) external_actor_ref: ExternalActorRef,
    pub(crate) external_event_id: crate::ExternalEventId,
}

impl AcceptedMessageReplayKey {
    pub(crate) fn new(
        tenant_id: &TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
        external_event_id: &crate::ExternalEventId,
    ) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: adapter_installation_id.clone(),
            external_actor_ref: external_actor_ref.clone(),
            external_event_id: external_event_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct MessageIdempotencyKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) source_binding_ref: String,
    pub(crate) external_event_id: crate::ExternalEventId,
}
