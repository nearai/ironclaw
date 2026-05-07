//! Conversation binding and session-thread contracts for IronClaw Reborn.
//!
//! This crate is the adapter-safe boundary between product/channel adapters and
//! `ironclaw_turns::TurnCoordinator`. It resolves external actor/conversation
//! identifiers into canonical tenant/thread/message/binding references without
//! asking the turn coordinator to parse raw channel payloads or store message
//! content.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, RunProfileRequest, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnScope,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! bounded_string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InboundTurnError> {
                let value = value.into();
                validate_external_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

bounded_string_id!(AdapterKind, "adapter_kind");
bounded_string_id!(AdapterInstallationId, "adapter_installation_id");
bounded_string_id!(ExternalEventId, "external_event_id");
bounded_string_id!(InboundMessageContentRef, "inbound_message_content_ref");

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalActorRef {
    kind: String,
    id: String,
}

impl ExternalActorRef {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Result<Self, InboundTurnError> {
        let kind = kind.into();
        let id = id.into();
        validate_external_id("external_actor_kind", &kind)?;
        validate_external_id("external_actor_id", &id)?;
        Ok(Self { kind, id })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalConversationRef {
    space_id: Option<String>,
    conversation_id: String,
    thread_id: Option<String>,
    message_id: Option<String>,
}

impl ExternalConversationRef {
    pub fn new(
        space_id: Option<&str>,
        conversation_id: impl Into<String>,
        thread_id: Option<&str>,
        message_id: Option<&str>,
    ) -> Result<Self, InboundTurnError> {
        let space_id = space_id.map(str::to_string);
        let conversation_id = conversation_id.into();
        let thread_id = thread_id.map(str::to_string);
        let message_id = message_id.map(str::to_string);
        if let Some(value) = &space_id {
            validate_external_id("external_space_id", value)?;
        }
        validate_external_id("external_conversation_id", &conversation_id)?;
        if let Some(value) = &thread_id {
            validate_external_id("external_thread_id", value)?;
        }
        if let Some(value) = &message_id {
            validate_external_id("external_message_id", value)?;
        }
        Ok(Self {
            space_id,
            conversation_id,
            thread_id,
            message_id,
        })
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn conversation_fingerprint(&self) -> String {
        length_prefixed_fingerprint(&[
            self.space_id.as_deref().unwrap_or(""),
            &self.conversation_id,
            self.thread_id.as_deref().unwrap_or(""),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveConversationRequest {
    pub tenant_id: TenantId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_event_id: ExternalEventId,
    pub requested_agent_id: Option<AgentId>,
    pub requested_project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationBindingResolution {
    pub tenant_id: TenantId,
    pub actor: TurnActor,
    pub turn_scope: TurnScope,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub access: ThreadAccessDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkConversationRequest {
    pub tenant_id: TenantId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub target_thread_id: ThreadId,
    pub target_agent_id: Option<AgentId>,
    pub target_project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedConversationBinding {
    pub thread_id: ThreadId,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTargetBinding {
    pub tenant_id: TenantId,
    pub actor_user_id: UserId,
    pub thread_id: ThreadId,
    pub external_conversation_ref: ExternalConversationRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadAccessDecision {
    Allowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageIdempotencyStatus {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptInboundMessageRequest {
    pub tenant_id: TenantId,
    pub thread_id: ThreadId,
    pub actor: TurnActor,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub external_event_id: ExternalEventId,
    pub content_ref: InboundMessageContentRef,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedInboundMessage {
    pub tenant_id: TenantId,
    pub thread_id: ThreadId,
    pub message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub idempotency: MessageIdempotencyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessageRecord {
    pub accepted: AcceptedInboundMessage,
    pub actor: TurnActor,
    pub external_event_id: ExternalEventId,
    pub content_ref: InboundMessageContentRef,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundTurnRequest {
    pub tenant_id: TenantId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_event_id: ExternalEventId,
    pub content_ref: InboundMessageContentRef,
    pub requested_agent_id: Option<AgentId>,
    pub requested_project_id: Option<ProjectId>,
    pub received_at: DateTime<Utc>,
    pub requested_run_profile: Option<RunProfileRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundTurnResponse {
    pub resolution: ConversationBindingResolution,
    pub accepted_message: AcceptedInboundMessage,
    pub turn_submission: Option<SubmitTurnResponse>,
}

#[derive(Debug, thiserror::Error)]
pub enum InboundTurnError {
    #[error("{kind} is invalid: {reason}")]
    InvalidExternalRef { kind: &'static str, reason: String },
    #[error(
        "external actor {external_actor_id} on adapter {adapter_kind} requires pairing/binding"
    )]
    BindingRequired {
        adapter_kind: String,
        external_actor_id: String,
    },
    #[error("actor {actor_id} is not allowed to access thread {thread_id}")]
    AccessDenied { actor_id: String, thread_id: String },
    #[error("thread {thread_id} was not found")]
    ThreadNotFound { thread_id: String },
    #[error("internal conversation state lock is poisoned")]
    StatePoisoned,
    #[error("failed to construct canonical reference: {reason}")]
    InvalidCanonicalRef { reason: String },
    #[error("turn submission failed: {reason}")]
    TurnSubmissionFailed { reason: String },
}

#[async_trait]
pub trait ConversationBindingService: Send + Sync {
    async fn resolve_or_create_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError>;

    async fn link_conversation_to_thread(
        &self,
        request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError>;

    async fn validate_reply_target(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
        reply_target_binding_ref: &ReplyTargetBindingRef,
    ) -> Result<ReplyTargetBinding, InboundTurnError>;
}

pub trait ConversationBindingServiceExt: ConversationBindingService {}
impl<T> ConversationBindingServiceExt for T where T: ConversationBindingService {}

#[async_trait]
pub trait SessionThreadService: Send + Sync {
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, InboundTurnError>;
}

#[derive(Clone)]
pub struct InboundTurnService<B, S, C> {
    binding_service: B,
    session_thread_service: S,
    turn_coordinator: Arc<C>,
}

impl<B, S, C> InboundTurnService<B, S, C>
where
    B: ConversationBindingService,
    S: SessionThreadService,
    C: TurnCoordinator,
{
    pub fn new(binding_service: B, session_thread_service: S, turn_coordinator: Arc<C>) -> Self {
        Self {
            binding_service,
            session_thread_service,
            turn_coordinator,
        }
    }

    pub async fn handle_inbound_turn(
        &self,
        request: InboundTurnRequest,
    ) -> Result<InboundTurnResponse, InboundTurnError> {
        let resolution = self
            .binding_service
            .resolve_or_create_binding(ResolveConversationRequest {
                tenant_id: request.tenant_id.clone(),
                adapter_kind: request.adapter_kind,
                adapter_installation_id: request.adapter_installation_id,
                external_actor_ref: request.external_actor_ref,
                external_conversation_ref: request.external_conversation_ref,
                external_event_id: request.external_event_id.clone(),
                requested_agent_id: request.requested_agent_id,
                requested_project_id: request.requested_project_id,
            })
            .await?;
        let accepted_message = self
            .session_thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                tenant_id: resolution.tenant_id.clone(),
                thread_id: resolution.turn_scope.thread_id.clone(),
                actor: resolution.actor.clone(),
                source_binding_ref: resolution.source_binding_ref.clone(),
                reply_target_binding_ref: resolution.reply_target_binding_ref.clone(),
                external_event_id: request.external_event_id,
                content_ref: request.content_ref,
                received_at: request.received_at,
            })
            .await?;

        if accepted_message.idempotency == MessageIdempotencyStatus::Duplicate {
            return Ok(InboundTurnResponse {
                resolution,
                accepted_message,
                turn_submission: None,
            });
        }

        let idempotency_key = IdempotencyKey::new(format!(
            "accepted-message:{}",
            accepted_message.message_ref.as_str()
        ))
        .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        let turn_submission = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: resolution.turn_scope.clone(),
                actor: resolution.actor.clone(),
                accepted_message_ref: accepted_message.message_ref.clone(),
                source_binding_ref: accepted_message.source_binding_ref.clone(),
                reply_target_binding_ref: accepted_message.reply_target_binding_ref.clone(),
                requested_run_profile: request.requested_run_profile,
                idempotency_key,
                received_at: request.received_at,
            })
            .await
            .map_err(|error| InboundTurnError::TurnSubmissionFailed {
                reason: error.to_string(),
            })?;

        Ok(InboundTurnResponse {
            resolution,
            accepted_message,
            turn_submission: Some(turn_submission),
        })
    }
}

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

        if let Some(binding) = state.bindings.get(&binding_key).cloned() {
            state.ensure_participant(&request.tenant_id, &actor_user_id, &binding.thread_id)?;
            return Ok(binding.resolution(actor_user_id, request.tenant_id));
        }

        let thread_id = ThreadId::new(Uuid::new_v4().to_string()).map_err(|error| {
            InboundTurnError::InvalidCanonicalRef {
                reason: error.to_string(),
            }
        })?;
        let thread = ThreadRecord {
            agent_id: request.requested_agent_id.clone(),
            project_id: request.requested_project_id.clone(),
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
            thread_id,
            request.requested_agent_id,
            request.requested_project_id,
        )?;
        let resolution = binding.resolution(actor_user_id, request.tenant_id.clone());
        state.reply_targets.insert(
            binding.reply_target_binding_ref.as_str().to_string(),
            binding.clone(),
        );
        state.bindings.insert(binding_key, binding);
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
            external_conversation_ref: request.external_conversation_ref.clone(),
        };
        let binding = BindingRecord::new(
            request.tenant_id,
            request.adapter_kind,
            request.adapter_installation_id,
            request.external_conversation_ref,
            request.target_thread_id,
            target_thread.agent_id,
            target_thread.project_id,
        )?;
        let linked = LinkedConversationBinding {
            thread_id: binding.thread_id.clone(),
            source_binding_ref: binding.source_binding_ref.clone(),
            reply_target_binding_ref: binding.reply_target_binding_ref.clone(),
        };
        state.reply_targets.insert(
            binding.reply_target_binding_ref.as_str().to_string(),
            binding.clone(),
        );
        state.bindings.insert(binding_key, binding);
        Ok(linked)
    }

    async fn validate_reply_target(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
        reply_target_binding_ref: &ReplyTargetBindingRef,
    ) -> Result<ReplyTargetBinding, InboundTurnError> {
        let state = self.lock_state()?;
        let Some(binding) = state
            .reply_targets
            .get(reply_target_binding_ref.as_str())
            .cloned()
        else {
            return Err(InboundTurnError::ThreadNotFound {
                thread_id: reply_target_binding_ref.as_str().to_string(),
            });
        };
        if binding.tenant_id != *tenant_id {
            return Err(InboundTurnError::AccessDenied {
                actor_id: actor_user_id.to_string(),
                thread_id: binding.thread_id.to_string(),
            });
        }
        state.ensure_participant(&binding.tenant_id, actor_user_id, &binding.thread_id)?;
        Ok(ReplyTargetBinding {
            tenant_id: binding.tenant_id,
            actor_user_id: actor_user_id.clone(),
            thread_id: binding.thread_id,
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
        state.ensure_participant(
            &request.tenant_id,
            &request.actor.user_id,
            &request.thread_id,
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
        let accepted = AcceptedInboundMessage {
            tenant_id: request.tenant_id,
            thread_id: request.thread_id,
            message_ref,
            source_binding_ref: request.source_binding_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
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
    reply_targets: HashMap<String, BindingRecord>,
    threads: HashMap<ThreadKey, ThreadRecord>,
    message_idempotency: HashMap<MessageIdempotencyKey, AcceptedInboundMessage>,
    messages: Vec<ThreadMessageRecord>,
}

impl InMemoryState {
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
    external_conversation_ref: ExternalConversationRef,
}

impl BindingKey {
    fn from_request(request: &ResolveConversationRequest) -> Self {
        Self {
            tenant_id: request.tenant_id.clone(),
            adapter_kind: request.adapter_kind.clone(),
            adapter_installation_id: request.adapter_installation_id.clone(),
            external_conversation_ref: request.external_conversation_ref.clone(),
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
struct BindingRecord {
    tenant_id: TenantId,
    external_conversation_ref: ExternalConversationRef,
    thread_id: ThreadId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

impl BindingRecord {
    fn new(
        tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        _adapter_installation_id: AdapterInstallationId,
        external_conversation_ref: ExternalConversationRef,
        thread_id: ThreadId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
    ) -> Result<Self, InboundTurnError> {
        let source_binding_ref = SourceBindingRef::new(format!("source:{}", Uuid::new_v4()))
            .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        let reply_target_binding_ref =
            ReplyTargetBindingRef::new(format!("reply:{}", Uuid::new_v4()))
                .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        Ok(Self {
            tenant_id,
            external_conversation_ref,
            thread_id,
            agent_id,
            project_id,
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
    external_event_id: ExternalEventId,
}

fn length_prefixed_fingerprint(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts {
        out.push_str(&part.len().to_string());
        out.push(':');
        out.push_str(part);
        out.push('|');
    }
    out
}

fn validate_external_id(kind: &'static str, value: &str) -> Result<(), InboundTurnError> {
    if value.is_empty() {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must not be empty".to_string(),
        });
    }
    if value.len() > 512 {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must be at most 512 bytes".to_string(),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must not contain NUL/control characters".to_string(),
        });
    }
    Ok(())
}
