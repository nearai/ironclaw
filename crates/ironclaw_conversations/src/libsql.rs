use std::sync::Arc;

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::{
    InMemoryConversationServices, InboundTurnError, ThreadMessageRecord,
    memory::{
        ActorKey, BindingKey, BindingRecord, InMemoryState, MessageIdempotencyKey,
        ReplyTargetRecord, ThreadKey, ThreadRecord,
    },
    state_store::{ConversationStateRepository, PersistedConversationState},
};

const STATE_KEY: &str = "default";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_conversation_state_meta (
    state_key TEXT PRIMARY KEY,
    version INTEGER NOT NULL
);
INSERT OR IGNORE INTO reborn_conversation_state_meta (state_key, version) VALUES ('default', 0);

CREATE TABLE IF NOT EXISTS reborn_conversation_actor_pairings (
    tenant_id TEXT NOT NULL,
    adapter_kind TEXT NOT NULL,
    adapter_installation_id TEXT NOT NULL,
    external_actor_kind TEXT NOT NULL,
    external_actor_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    key_payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, adapter_kind, adapter_installation_id, external_actor_kind, external_actor_id)
);

CREATE TABLE IF NOT EXISTS reborn_conversation_threads (
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT,
    project_id TEXT,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, thread_id)
);

CREATE TABLE IF NOT EXISTS reborn_conversation_thread_participants (
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (tenant_id, thread_id, user_id)
);

CREATE TABLE IF NOT EXISTS reborn_conversation_bindings (
    tenant_id TEXT NOT NULL,
    adapter_kind TEXT NOT NULL,
    adapter_installation_id TEXT NOT NULL,
    conversation_key TEXT NOT NULL,
    conversation_fingerprint TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source_binding_ref TEXT NOT NULL UNIQUE,
    reply_target_binding_ref TEXT NOT NULL,
    owner_external_actor_kind TEXT NOT NULL,
    owner_external_actor_id TEXT NOT NULL,
    shared INTEGER NOT NULL,
    key_payload TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, adapter_kind, adapter_installation_id, conversation_key)
);
CREATE INDEX IF NOT EXISTS idx_reborn_conversation_bindings_thread
    ON reborn_conversation_bindings(tenant_id, thread_id);

CREATE TABLE IF NOT EXISTS reborn_conversation_reply_targets (
    reply_target_binding_ref TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source_binding_ref TEXT NOT NULL,
    adapter_kind TEXT NOT NULL,
    adapter_installation_id TEXT NOT NULL,
    conversation_key TEXT NOT NULL,
    conversation_fingerprint TEXT NOT NULL,
    owner_external_actor_kind TEXT NOT NULL,
    owner_external_actor_id TEXT NOT NULL,
    shared INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_conversation_reply_targets_thread
    ON reborn_conversation_reply_targets(tenant_id, thread_id);

CREATE TABLE IF NOT EXISTS reborn_conversation_external_event_routes (
    tenant_id TEXT NOT NULL,
    adapter_kind TEXT NOT NULL,
    adapter_installation_id TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    conversation_key TEXT NOT NULL,
    conversation_fingerprint TEXT NOT NULL,
    key_payload TEXT NOT NULL,
    identity_payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, adapter_kind, adapter_installation_id, external_event_id)
);

CREATE TABLE IF NOT EXISTS reborn_conversation_accepted_messages (
    message_ref TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source_binding_ref TEXT NOT NULL,
    reply_target_binding_ref TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    actor_user_id TEXT NOT NULL,
    content_ref TEXT NOT NULL,
    received_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    UNIQUE (tenant_id, source_binding_ref, external_event_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_conversation_accepted_messages_thread
    ON reborn_conversation_accepted_messages(tenant_id, thread_id);

CREATE TABLE IF NOT EXISTS reborn_conversation_message_replays (
    tenant_id TEXT NOT NULL,
    adapter_kind TEXT NOT NULL,
    adapter_installation_id TEXT NOT NULL,
    external_actor_kind TEXT NOT NULL,
    external_actor_id TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    conversation_key TEXT NOT NULL,
    conversation_fingerprint TEXT NOT NULL,
    message_ref TEXT NOT NULL,
    key_payload TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, adapter_kind, adapter_installation_id, external_actor_kind, external_actor_id, external_event_id)
);

CREATE TABLE IF NOT EXISTS reborn_conversation_submission_keys (
    message_ref TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS reborn_conversation_submit_responses (
    message_ref TEXT PRIMARY KEY,
    payload TEXT NOT NULL
);
"#;

const TABLES: &[&str] = &[
    "reborn_conversation_submit_responses",
    "reborn_conversation_submission_keys",
    "reborn_conversation_message_replays",
    "reborn_conversation_accepted_messages",
    "reborn_conversation_external_event_routes",
    "reborn_conversation_reply_targets",
    "reborn_conversation_bindings",
    "reborn_conversation_thread_participants",
    "reborn_conversation_threads",
    "reborn_conversation_actor_pairings",
];

pub struct RebornLibSqlConversationStateStore {
    db: Arc<::libsql::Database>,
}

impl RebornLibSqlConversationStateStore {
    pub fn new(db: Arc<::libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), InboundTurnError> {
        let conn = self.connect().await?;
        conn.execute_batch(SCHEMA).await.map_err(db_error)?;
        Ok(())
    }

    async fn connect(&self) -> Result<::libsql::Connection, InboundTurnError> {
        let conn = self.db.connect().map_err(db_error)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(db_error)?;
        Ok(conn)
    }

    async fn begin_immediate(&self) -> Result<::libsql::Connection, InboundTurnError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(db_error)?;
        Ok(conn)
    }
}

#[async_trait]
impl ConversationStateRepository for RebornLibSqlConversationStateStore {
    async fn load_state(&self) -> Result<PersistedConversationState, InboundTurnError> {
        self.run_migrations().await?;
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await.map_err(db_error)?;
        let result = load_state_from_conn(&conn).await;
        finish_transaction(&conn, result).await
    }

    async fn save_state(
        &self,
        expected_revision: i64,
        state: &InMemoryState,
    ) -> Result<i64, InboundTurnError> {
        self.run_migrations().await?;
        let conn = self.begin_immediate().await?;
        let result = save_state_to_conn(&conn, expected_revision, state).await;
        finish_transaction(&conn, result).await
    }
}

#[derive(Clone)]
pub struct RebornLibSqlConversationServices {
    inner: InMemoryConversationServices,
}

impl RebornLibSqlConversationServices {
    pub async fn new(db: Arc<::libsql::Database>) -> Result<Self, InboundTurnError> {
        let store = Arc::new(RebornLibSqlConversationStateStore::new(db));
        store.run_migrations().await?;
        Ok(Self {
            inner: InMemoryConversationServices::with_state_repository(store).await?,
        })
    }

    pub fn inner(&self) -> &InMemoryConversationServices {
        &self.inner
    }

    pub async fn pair_external_actor(
        &self,
        tenant_id: ironclaw_host_api::TenantId,
        adapter_kind: crate::AdapterKind,
        adapter_installation_id: crate::AdapterInstallationId,
        external_actor_ref: crate::ExternalActorRef,
        user_id: ironclaw_host_api::UserId,
    ) -> Result<(), InboundTurnError> {
        self.inner
            .try_pair_external_actor(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
                user_id,
            )
            .await
    }

    pub async fn unpair_external_actor(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        adapter_kind: &crate::AdapterKind,
        adapter_installation_id: &crate::AdapterInstallationId,
        external_actor_ref: &crate::ExternalActorRef,
    ) -> Result<(), InboundTurnError> {
        self.inner
            .try_unpair_external_actor(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
            )
            .await
    }
}

#[async_trait]
impl crate::ConversationBindingService for RebornLibSqlConversationServices {
    async fn resolve_or_create_binding(
        &self,
        request: crate::ResolveConversationRequest,
    ) -> Result<crate::ConversationBindingResolution, InboundTurnError> {
        self.inner.resolve_or_create_binding(request).await
    }

    async fn link_conversation_to_thread(
        &self,
        request: crate::LinkConversationRequest,
    ) -> Result<crate::LinkedConversationBinding, InboundTurnError> {
        self.inner.link_conversation_to_thread(request).await
    }

    async fn validate_reply_target(
        &self,
        request: crate::ValidateReplyTargetRequest,
    ) -> Result<crate::ReplyTargetBinding, InboundTurnError> {
        self.inner.validate_reply_target(request).await
    }
}

#[async_trait]
impl crate::SessionThreadService for RebornLibSqlConversationServices {
    async fn accept_inbound_message(
        &self,
        request: crate::AcceptInboundMessageRequest,
    ) -> Result<crate::AcceptedInboundMessage, InboundTurnError> {
        self.inner.accept_inbound_message(request).await
    }

    async fn replay_accepted_inbound_message(
        &self,
        lookup: crate::AcceptedInboundMessageLookup,
    ) -> Result<Option<crate::AcceptedInboundMessageReplay>, InboundTurnError> {
        self.inner.replay_accepted_inbound_message(lookup).await
    }

    async fn inbound_message_turn_submission(
        &self,
        message_ref: &ironclaw_turns::AcceptedMessageRef,
    ) -> Result<Option<ironclaw_turns::SubmitTurnResponse>, InboundTurnError> {
        self.inner
            .inbound_message_turn_submission(message_ref)
            .await
    }

    async fn inbound_message_turn_submission_key(
        &self,
        message_ref: &ironclaw_turns::AcceptedMessageRef,
    ) -> Result<ironclaw_turns::IdempotencyKey, InboundTurnError> {
        self.inner
            .inbound_message_turn_submission_key(message_ref)
            .await
    }

    async fn rotate_inbound_message_turn_submission_key(
        &self,
        message_ref: &ironclaw_turns::AcceptedMessageRef,
    ) -> Result<(), InboundTurnError> {
        self.inner
            .rotate_inbound_message_turn_submission_key(message_ref)
            .await
    }

    async fn mark_inbound_message_turn_submitted(
        &self,
        message_ref: &ironclaw_turns::AcceptedMessageRef,
        response: ironclaw_turns::SubmitTurnResponse,
    ) -> Result<(), InboundTurnError> {
        self.inner
            .mark_inbound_message_turn_submitted(message_ref, response)
            .await
    }
}

async fn load_state_from_conn(
    conn: &::libsql::Connection,
) -> Result<PersistedConversationState, InboundTurnError> {
    let revision = load_revision(conn).await?;
    let mut state = InMemoryState::default();

    let mut rows = conn
        .query(
            "SELECT key_payload, user_id FROM reborn_conversation_actor_pairings",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let key: ActorKey = from_json(&row.get::<String>(0).map_err(db_error)?)?;
        let user_id = ironclaw_host_api::UserId::new(row.get::<String>(1).map_err(db_error)?)
            .map_err(|error| InboundTurnError::DurableState {
                reason: error.to_string(),
            })?;
        state.pairings.insert(key, user_id);
    }

    let mut rows = conn
        .query(
            "SELECT key_payload, payload FROM reborn_conversation_bindings",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let key: BindingKey = from_json(&row.get::<String>(0).map_err(db_error)?)?;
        let binding: BindingRecord = from_json(&row.get::<String>(1).map_err(db_error)?)?;
        state.source_bindings.insert(
            binding.source_binding_ref.as_str().to_string(),
            binding.clone(),
        );
        state.bindings.insert(key, binding);
    }

    let mut rows = conn
        .query("SELECT payload FROM reborn_conversation_reply_targets", ())
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let reply_target: ReplyTargetRecord = from_json(&row.get::<String>(0).map_err(db_error)?)?;
        state.reply_targets.insert(
            reply_target.reply_target_binding_ref.as_str().to_string(),
            reply_target,
        );
    }

    let mut rows = conn
        .query("SELECT payload FROM reborn_conversation_threads", ())
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let thread_key_and_record: (ThreadKey, ThreadRecord) =
            from_json(&row.get::<String>(0).map_err(db_error)?)?;
        state
            .threads
            .insert(thread_key_and_record.0, thread_key_and_record.1);
    }

    let mut rows = conn
        .query(
            "SELECT tenant_id, thread_id, user_id FROM reborn_conversation_thread_participants",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let tenant_id = ironclaw_host_api::TenantId::new(row.get::<String>(0).map_err(db_error)?)
            .map_err(|error| InboundTurnError::DurableState {
            reason: error.to_string(),
        })?;
        let thread_id = ironclaw_host_api::ThreadId::new(row.get::<String>(1).map_err(db_error)?)
            .map_err(|error| InboundTurnError::DurableState {
            reason: error.to_string(),
        })?;
        let user_id = ironclaw_host_api::UserId::new(row.get::<String>(2).map_err(db_error)?)
            .map_err(|error| InboundTurnError::DurableState {
                reason: error.to_string(),
            })?;
        if let Some(thread) = state
            .threads
            .get_mut(&ThreadKey::new(&tenant_id, &thread_id))
        {
            thread.participants.insert(user_id);
        }
    }

    let mut rows = conn
        .query(
            "SELECT key_payload, identity_payload FROM reborn_conversation_external_event_routes",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        state.external_event_routes.insert(
            from_json(&row.get::<String>(0).map_err(db_error)?)?,
            from_json(&row.get::<String>(1).map_err(db_error)?)?,
        );
    }

    let mut rows = conn
        .query(
            "SELECT payload FROM reborn_conversation_accepted_messages",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let message: ThreadMessageRecord = from_json(&row.get::<String>(0).map_err(db_error)?)?;
        let idempotency_key = MessageIdempotencyKey {
            tenant_id: message.accepted.tenant_id.clone(),
            source_binding_ref: message.accepted.source_binding_ref.as_str().to_string(),
            external_event_id: message.external_event_id.clone(),
        };
        state
            .message_idempotency
            .insert(idempotency_key, message.accepted.clone());
        state.messages.push(message);
    }

    let mut rows = conn
        .query(
            "SELECT key_payload, payload FROM reborn_conversation_message_replays",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        state.message_replays.insert(
            from_json(&row.get::<String>(0).map_err(db_error)?)?,
            from_json(&row.get::<String>(1).map_err(db_error)?)?,
        );
    }

    let mut rows = conn
        .query(
            "SELECT message_ref, idempotency_key FROM reborn_conversation_submission_keys",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let message_ref =
            ironclaw_turns::AcceptedMessageRef::new(row.get::<String>(0).map_err(db_error)?)
                .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        let key = ironclaw_turns::IdempotencyKey::new(row.get::<String>(1).map_err(db_error)?)
            .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        state.submission_keys.insert(message_ref, key);
    }

    let mut rows = conn
        .query(
            "SELECT message_ref, payload FROM reborn_conversation_submit_responses",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let message_ref =
            ironclaw_turns::AcceptedMessageRef::new(row.get::<String>(0).map_err(db_error)?)
                .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        state.submitted_message_responses.insert(
            message_ref,
            from_json(&row.get::<String>(1).map_err(db_error)?)?,
        );
    }

    Ok(PersistedConversationState { state, revision })
}

async fn save_state_to_conn(
    conn: &::libsql::Connection,
    expected_revision: i64,
    state: &InMemoryState,
) -> Result<i64, InboundTurnError> {
    let new_revision = expected_revision + 1;
    let updated = conn
        .execute(
            "UPDATE reborn_conversation_state_meta SET version = ?2 WHERE state_key = ?1 AND version = ?3",
            ::libsql::params![STATE_KEY, new_revision, expected_revision],
        )
        .await
        .map_err(db_error)?;
    if updated == 0 {
        return Err(InboundTurnError::DurableState {
            reason: "stale conversation state revision".to_string(),
        });
    }
    for table in TABLES {
        conn.execute(&format!("DELETE FROM {table}"), ())
            .await
            .map_err(db_error)?;
    }

    for (key, user_id) in &state.pairings {
        conn.execute(
            "INSERT INTO reborn_conversation_actor_pairings \
             (tenant_id, adapter_kind, adapter_installation_id, external_actor_kind, external_actor_id, user_id, key_payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            ::libsql::params![
                key.tenant_id.as_str(),
                key.adapter_kind.as_str(),
                key.adapter_installation_id.as_str(),
                key.external_actor_ref.kind(),
                key.external_actor_ref.id(),
                user_id.as_str(),
                to_json(key)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }

    for (key, thread) in &state.threads {
        conn.execute(
            "INSERT INTO reborn_conversation_threads (tenant_id, thread_id, agent_id, project_id, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            ::libsql::params![
                key.tenant_id.as_str(),
                key.thread_id.as_str(),
                thread.agent_id.as_ref().map(|id| id.as_str()),
                thread.project_id.as_ref().map(|id| id.as_str()),
                to_json(&(key, thread))?,
            ],
        )
        .await
        .map_err(db_error)?;
        for participant in &thread.participants {
            conn.execute(
                "INSERT INTO reborn_conversation_thread_participants (tenant_id, thread_id, user_id) VALUES (?1, ?2, ?3)",
                ::libsql::params![key.tenant_id.as_str(), key.thread_id.as_str(), participant.as_str()],
            )
            .await
            .map_err(db_error)?;
        }
    }

    for (key, binding) in &state.bindings {
        let conversation_fingerprint = key
            .external_conversation_identity
            .conversation_fingerprint();
        conn.execute(
            "INSERT INTO reborn_conversation_bindings \
             (tenant_id, adapter_kind, adapter_installation_id, conversation_key, conversation_fingerprint, thread_id, source_binding_ref, reply_target_binding_ref, owner_external_actor_kind, owner_external_actor_id, shared, key_payload, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            ::libsql::params![
                key.tenant_id.as_str(),
                key.adapter_kind.as_str(),
                key.adapter_installation_id.as_str(),
                conversation_digest(&conversation_fingerprint),
                conversation_fingerprint,
                binding.thread_id.as_str(),
                binding.source_binding_ref.as_str(),
                binding.reply_target_binding_ref.as_str(),
                binding.route_access.owner_actor_key.external_actor_ref.kind(),
                binding.route_access.owner_actor_key.external_actor_ref.id(),
                if binding.route_access.shared { 1_i64 } else { 0_i64 },
                to_json(key)?,
                to_json(binding)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }

    for reply_target in state.reply_targets.values() {
        let conversation_fingerprint = reply_target
            .external_conversation_ref
            .conversation_fingerprint();
        conn.execute(
            "INSERT INTO reborn_conversation_reply_targets \
             (reply_target_binding_ref, tenant_id, thread_id, source_binding_ref, adapter_kind, adapter_installation_id, conversation_key, conversation_fingerprint, owner_external_actor_kind, owner_external_actor_id, shared, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            ::libsql::params![
                reply_target.reply_target_binding_ref.as_str(),
                reply_target.tenant_id.as_str(),
                reply_target.thread_id.as_str(),
                reply_target.source_binding_ref.as_str(),
                reply_target.adapter_kind.as_str(),
                reply_target.adapter_installation_id.as_str(),
                conversation_digest(&conversation_fingerprint),
                conversation_fingerprint,
                reply_target.route_access.owner_actor_key.external_actor_ref.kind(),
                reply_target.route_access.owner_actor_key.external_actor_ref.id(),
                if reply_target.route_access.shared { 1_i64 } else { 0_i64 },
                to_json(reply_target)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }

    for (key, identity) in &state.external_event_routes {
        let conversation_fingerprint = identity.conversation_fingerprint();
        conn.execute(
            "INSERT INTO reborn_conversation_external_event_routes \
             (tenant_id, adapter_kind, adapter_installation_id, external_event_id, conversation_key, conversation_fingerprint, key_payload, identity_payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            ::libsql::params![
                key.tenant_id.as_str(),
                key.adapter_kind.as_str(),
                key.adapter_installation_id.as_str(),
                key.external_event_id.as_str(),
                conversation_digest(&conversation_fingerprint),
                conversation_fingerprint,
                to_json(key)?,
                to_json(identity)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }

    for message in &state.messages {
        conn.execute(
            "INSERT INTO reborn_conversation_accepted_messages \
             (message_ref, tenant_id, thread_id, source_binding_ref, reply_target_binding_ref, external_event_id, actor_user_id, content_ref, received_at, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            ::libsql::params![
                message.accepted.message_ref.as_str(),
                message.accepted.tenant_id.as_str(),
                message.accepted.thread_id.as_str(),
                message.accepted.source_binding_ref.as_str(),
                message.accepted.reply_target_binding_ref.as_str(),
                message.external_event_id.as_str(),
                message.actor.user_id.as_str(),
                message.content_ref.as_str(),
                message.received_at.to_rfc3339(),
                to_json(message)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }

    for (key, replay) in &state.message_replays {
        let conversation_fingerprint = replay
            .external_conversation_identity
            .conversation_fingerprint();
        conn.execute(
            "INSERT INTO reborn_conversation_message_replays \
             (tenant_id, adapter_kind, adapter_installation_id, external_actor_kind, external_actor_id, external_event_id, conversation_key, conversation_fingerprint, message_ref, key_payload, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            ::libsql::params![
                key.tenant_id.as_str(),
                key.adapter_kind.as_str(),
                key.adapter_installation_id.as_str(),
                key.external_actor_ref.kind(),
                key.external_actor_ref.id(),
                key.external_event_id.as_str(),
                conversation_digest(&conversation_fingerprint),
                conversation_fingerprint,
                replay.replay.accepted_message.message_ref.as_str(),
                to_json(key)?,
                to_json(replay)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }

    for (message_ref, key) in &state.submission_keys {
        conn.execute(
            "INSERT INTO reborn_conversation_submission_keys (message_ref, idempotency_key) VALUES (?1, ?2)",
            ::libsql::params![message_ref.as_str(), key.as_str()],
        )
        .await
        .map_err(db_error)?;
    }

    for (message_ref, response) in &state.submitted_message_responses {
        conn.execute(
            "INSERT INTO reborn_conversation_submit_responses (message_ref, payload) VALUES (?1, ?2)",
            ::libsql::params![message_ref.as_str(), to_json(response)?],
        )
        .await
        .map_err(db_error)?;
    }

    Ok(new_revision)
}

async fn load_revision(conn: &::libsql::Connection) -> Result<i64, InboundTurnError> {
    let mut rows = conn
        .query(
            "SELECT version FROM reborn_conversation_state_meta WHERE state_key = ?1",
            ::libsql::params![STATE_KEY],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Err(InboundTurnError::DurableState {
            reason: "missing conversation state metadata row".to_string(),
        });
    };
    row.get(0).map_err(db_error)
}

async fn finish_transaction<T>(
    conn: &::libsql::Connection,
    result: Result<T, InboundTurnError>,
) -> Result<T, InboundTurnError> {
    match result {
        Ok(value) => {
            conn.execute("COMMIT", ()).await.map_err(db_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(error)
        }
    }
}

fn conversation_digest(fingerprint: &str) -> String {
    let digest = Sha256::digest(fingerprint.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn to_json<T: Serialize>(value: &T) -> Result<String, InboundTurnError> {
    serde_json::to_string(value).map_err(|error| InboundTurnError::DurableState {
        reason: error.to_string(),
    })
}

fn from_json<T: DeserializeOwned>(value: &str) -> Result<T, InboundTurnError> {
    serde_json::from_str(value).map_err(|error| InboundTurnError::DurableState {
        reason: error.to_string(),
    })
}

fn db_error(error: impl std::error::Error) -> InboundTurnError {
    InboundTurnError::DurableState {
        reason: error.to_string(),
    }
}
