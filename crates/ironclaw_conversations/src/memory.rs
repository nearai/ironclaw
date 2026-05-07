use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, SourceBindingRef,
    SubmitTurnResponse, TurnActor, TurnScope,
};
use uuid::Uuid;

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AdapterInstallationId, AdapterKind,
    ConversationBindingResolution, ConversationBindingService, ConversationRouteKind,
    ExternalActorRef, ExternalConversationIdentity, ExternalConversationRef, InboundTurnError,
    LinkConversationRequest, LinkedConversationBinding, MessageIdempotencyStatus,
    ReplyTargetBinding, ResolveConversationRequest, SessionThreadService, ThreadAccessDecision,
    ThreadMessageRecord, ValidateReplyTargetRequest,
};

#[derive(Clone, Default)]
pub struct InMemoryConversationServices {
    state: Arc<Mutex<InMemoryState>>,
}

impl InMemoryConversationServices {
    pub async fn pair_external_actor(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
    ) {
        if let Ok(mut state) = self.state.lock() {
            state.pairings.insert(
                ActorKey::new(
                    &tenant_id,
                    &adapter_kind,
                    &adapter_installation_id,
                    &external_actor_ref,
                ),
                user_id,
            );
        }
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
        if let Ok(mut state) = self.state.lock() {
            state.pairings.remove(&ActorKey::new(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
            ));
        }
    }

    pub async fn add_thread_participant(
        &self,
        tenant_id: &TenantId,
        thread_id: &ThreadId,
        user_id: UserId,
    ) -> Result<(), InboundTurnError> {
        let mut state = self.lock_state()?;
        let Some(thread) = state.threads.get_mut(&ThreadKey::new(tenant_id, thread_id)) else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: thread_id.to_string(),
            });
        };
        thread.participants.insert(user_id);
        Ok(())
    }
}

#[async_trait]
impl ConversationBindingService for InMemoryConversationServices {
    async fn resolve_or_create_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        let mut state = self.lock_state()?;
        let actor_user_id = state.resolve_actor(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_actor_ref,
        )?;
        let binding_key = BindingKey::from_request(&request);
        state.record_external_event_route(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_event_id,
            &request.external_conversation_ref.identity(),
            &actor_user_id,
        )?;
        let route_actor_key = ActorKey::new(
            &request.tenant_id,
            &request.adapter_kind,
            &request.adapter_installation_id,
            &request.external_actor_ref,
        );

        if state.bindings.contains_key(&binding_key) {
            let binding = state
                .bindings
                .get(&binding_key)
                .cloned()
                .ok_or(InboundTurnError::StatePoisoned)?;
            state.ensure_participant(&request.tenant_id, &actor_user_id, &binding.thread_id)?;
            if !binding.route_access.allows(&route_actor_key) {
                return Err(InboundTurnError::AccessDenied {
                    actor_id: actor_user_id.to_string(),
                    thread_id: binding.thread_id.to_string(),
                });
            }
            if request.route_kind == ConversationRouteKind::Shared {
                state.widen_binding_route_access(&binding_key)?;
            }
            let binding = state
                .bindings
                .get(&binding_key)
                .cloned()
                .ok_or(InboundTurnError::StatePoisoned)?;
            return Ok(binding.resolution(actor_user_id, request.tenant_id));
        }

        let thread_id = ThreadId::new(Uuid::new_v4().to_string()).map_err(|error| {
            InboundTurnError::InvalidCanonicalRef {
                reason: error.to_string(),
            }
        })?;
        let thread = ThreadRecord {
            agent_id: None,
            project_id: None,
            participants: HashSet::from([actor_user_id.clone()]),
        };
        state
            .threads
            .insert(ThreadKey::new(&request.tenant_id, &thread_id), thread);
        let binding = BindingRecord::new(
            request.tenant_id.clone(),
            request.adapter_kind,
            request.adapter_installation_id,
            request.external_conversation_ref,
            ReplyRouteAccess::new(route_actor_key, request.route_kind),
            BindingTarget::new(thread_id, None, None),
        )?;
        let resolution = binding.resolution(actor_user_id, request.tenant_id.clone());
        state.store_binding(binding_key, binding);
        Ok(resolution)
    }

    async fn link_conversation_to_thread(
        &self,
        request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError> {
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
                if !existing.route_access.allows(&route_actor_key) {
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
                return Ok(LinkedConversationBinding {
                    thread_id: existing.thread_id,
                    source_binding_ref: existing.source_binding_ref,
                    reply_target_binding_ref: existing.reply_target_binding_ref,
                });
            }
            return Err(InboundTurnError::BindingConflict {
                thread_id: existing.thread_id.to_string(),
            });
        }
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
            ),
        )?;
        let linked = LinkedConversationBinding {
            thread_id: binding.thread_id.clone(),
            source_binding_ref: binding.source_binding_ref.clone(),
            reply_target_binding_ref: binding.reply_target_binding_ref.clone(),
        };
        state.store_binding(binding_key, binding);
        Ok(linked)
    }

    async fn validate_reply_target(
        &self,
        request: ValidateReplyTargetRequest,
    ) -> Result<ReplyTargetBinding, InboundTurnError> {
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
            || !binding.route_access.allows(&route_actor_key)
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
}

#[async_trait]
impl SessionThreadService for InMemoryConversationServices {
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, InboundTurnError> {
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
        state.ensure_binding_refs_match(
            &request.tenant_id,
            &request.thread_id,
            request.source_binding_ref.as_str(),
            request.reply_target_binding_ref.as_str(),
            &request.actor.user_id,
            &ActorKey::new(
                &request.tenant_id,
                &request.adapter_kind,
                &request.adapter_installation_id,
                &request.external_actor_ref,
            ),
        )?;
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
        state.record_external_event_route(
            &request.tenant_id,
            &source_binding.adapter_kind,
            &source_binding.adapter_installation_id,
            &request.external_event_id,
            &external_conversation_identity,
            &request.actor.user_id,
        )?;
        let idempotency_key = MessageIdempotencyKey {
            tenant_id: request.tenant_id.clone(),
            source_binding_ref: request.source_binding_ref.as_str().to_string(),
            external_event_id: request.external_event_id.clone(),
        };
        if let Some(existing) = state.message_idempotency.get(&idempotency_key) {
            let mut duplicate = existing.clone();
            duplicate.idempotency = MessageIdempotencyStatus::Duplicate;
            return Ok(duplicate);
        }

        let message_ref = AcceptedMessageRef::new(format!("message:{}", Uuid::new_v4()))
            .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        let reply_target_record = state
            .reply_targets
            .get(request.reply_target_binding_ref.as_str())
            .cloned()
            .ok_or_else(|| InboundTurnError::ThreadNotFound {
                thread_id: request.reply_target_binding_ref.as_str().to_string(),
            })?;
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
        state.messages.push(ThreadMessageRecord {
            accepted: accepted.clone(),
            actor: request.actor,
            external_event_id: request.external_event_id,
            content_ref: request.content_ref,
            received_at: request.received_at,
        });
        Ok(accepted)
    }

    async fn inbound_message_turn_submission(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<Option<SubmitTurnResponse>, InboundTurnError> {
        let state = self.lock_state()?;
        Ok(state.submitted_message_responses.get(message_ref).cloned())
    }

    async fn inbound_message_turn_submission_key(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<IdempotencyKey, InboundTurnError> {
        let mut state = self.lock_state()?;
        if let Some(key) = state.submission_keys.get(message_ref).cloned() {
            return Ok(key);
        }
        let key = IdempotencyKey::new(message_ref.as_str().to_string())
            .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        state
            .submission_keys
            .insert(message_ref.clone(), key.clone());
        Ok(key)
    }

    async fn rotate_inbound_message_turn_submission_key(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<(), InboundTurnError> {
        let mut state = self.lock_state()?;
        state
            .submission_keys
            .insert(message_ref.clone(), state_generated_submission_key()?);
        Ok(())
    }

    async fn mark_inbound_message_turn_submitted(
        &self,
        message_ref: &AcceptedMessageRef,
        response: SubmitTurnResponse,
    ) -> Result<(), InboundTurnError> {
        let mut state = self.lock_state()?;
        state
            .submitted_message_responses
            .insert(message_ref.clone(), response);
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

#[derive(Default)]
struct InMemoryState {
    pairings: HashMap<ActorKey, UserId>,
    bindings: HashMap<BindingKey, BindingRecord>,
    source_bindings: HashMap<String, BindingRecord>,
    reply_targets: HashMap<String, ReplyTargetRecord>,
    threads: HashMap<ThreadKey, ThreadRecord>,
    external_event_routes: HashMap<ExternalEventRouteKey, ExternalConversationIdentity>,
    message_idempotency: HashMap<MessageIdempotencyKey, AcceptedInboundMessage>,
    submission_keys: HashMap<AcceptedMessageRef, IdempotencyKey>,
    submitted_message_responses: HashMap<AcceptedMessageRef, SubmitTurnResponse>,
    messages: Vec<ThreadMessageRecord>,
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

    fn ensure_binding_refs_match(
        &self,
        tenant_id: &TenantId,
        thread_id: &ThreadId,
        source_binding_ref: &str,
        reply_target_binding_ref: &str,
        actor_user_id: &UserId,
        route_actor_key: &ActorKey,
    ) -> Result<(), InboundTurnError> {
        let Some(source_binding) = self.source_bindings.get(source_binding_ref) else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: source_binding_ref.to_string(),
            });
        };
        let Some(reply_binding) = self.reply_targets.get(reply_target_binding_ref) else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: reply_target_binding_ref.to_string(),
            });
        };
        if source_binding.tenant_id != *tenant_id
            || reply_binding.tenant_id != *tenant_id
            || source_binding.thread_id != *thread_id
            || reply_binding.thread_id != *thread_id
            || source_binding.adapter_kind != route_actor_key.adapter_kind
            || reply_binding.adapter_kind != route_actor_key.adapter_kind
            || source_binding.adapter_installation_id != route_actor_key.adapter_installation_id
            || reply_binding.adapter_installation_id != route_actor_key.adapter_installation_id
            || source_binding.source_binding_ref.as_str() != source_binding_ref
            || reply_binding.reply_target_binding_ref.as_str() != reply_target_binding_ref
            || source_binding.source_binding_ref != reply_binding.source_binding_ref
            || !reply_binding.route_access.allows(route_actor_key)
        {
            return Err(InboundTurnError::AccessDenied {
                actor_id: actor_user_id.to_string(),
                thread_id: thread_id.to_string(),
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActorKey {
    tenant_id: TenantId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_actor_ref: ExternalActorRef,
}

impl ActorKey {
    fn new(
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BindingKey {
    tenant_id: TenantId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_conversation_identity: ExternalConversationIdentity,
}

impl BindingKey {
    fn from_request(request: &ResolveConversationRequest) -> Self {
        Self {
            tenant_id: request.tenant_id.clone(),
            adapter_kind: request.adapter_kind.clone(),
            adapter_installation_id: request.adapter_installation_id.clone(),
            external_conversation_identity: request.external_conversation_ref.identity(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ExternalEventRouteKey {
    tenant_id: TenantId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_event_id: crate::ExternalEventId,
}

impl ExternalEventRouteKey {
    fn new(
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ThreadKey {
    tenant_id: TenantId,
    thread_id: ThreadId,
}

impl ThreadKey {
    fn new(tenant_id: &TenantId, thread_id: &ThreadId) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            thread_id: thread_id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct ThreadRecord {
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    participants: HashSet<UserId>,
}

#[derive(Debug, Clone)]
struct ReplyRouteAccess {
    owner_actor_key: ActorKey,
    shared: bool,
}

impl ReplyRouteAccess {
    fn new(owner_actor_key: ActorKey, route_kind: ConversationRouteKind) -> Self {
        Self {
            owner_actor_key,
            shared: route_kind == ConversationRouteKind::Shared,
        }
    }

    fn allow_shared(&mut self) {
        self.shared = true;
    }

    fn allows(&self, actor_key: &ActorKey) -> bool {
        self.shared || self.owner_actor_key == *actor_key
    }
}

#[derive(Debug, Clone)]
struct ReplyTargetRecord {
    tenant_id: TenantId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_conversation_ref: ExternalConversationRef,
    thread_id: ThreadId,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    route_access: ReplyRouteAccess,
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

#[derive(Debug, Clone)]
struct BindingTarget {
    thread_id: ThreadId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
}

impl BindingTarget {
    fn new(thread_id: ThreadId, agent_id: Option<AgentId>, project_id: Option<ProjectId>) -> Self {
        Self {
            thread_id,
            agent_id,
            project_id,
        }
    }
}

#[derive(Debug, Clone)]
struct BindingRecord {
    tenant_id: TenantId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_conversation_ref: ExternalConversationRef,
    external_conversation_identity: ExternalConversationIdentity,
    thread_id: ThreadId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    route_access: ReplyRouteAccess,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

impl BindingRecord {
    fn new(
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
            route_access,
            source_binding_ref,
            reply_target_binding_ref,
        })
    }

    fn resolution(
        &self,
        actor_user_id: UserId,
        tenant_id: TenantId,
    ) -> ConversationBindingResolution {
        ConversationBindingResolution {
            tenant_id: tenant_id.clone(),
            actor: TurnActor::new(actor_user_id),
            turn_scope: TurnScope::new(
                tenant_id,
                self.agent_id.clone(),
                self.project_id.clone(),
                self.thread_id.clone(),
            ),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            access: ThreadAccessDecision::Allowed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MessageIdempotencyKey {
    tenant_id: TenantId,
    source_binding_ref: String,
    external_event_id: crate::ExternalEventId,
}
