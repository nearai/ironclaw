//! Filesystem-backed conversation state store.
//!
//! Persists the [`InMemoryState`] singleton under the `/conversations`
//! mount alias on a [`ScopedFilesystem`] over any [`RootFilesystem`]. The
//! path returned by [`state_path`] is alias-relative — at every op the
//! [`ScopedFilesystem`] resolves the alias against its caller-supplied
//! [`MountView`](ironclaw_host_api::MountView) and enforces per-grant ACL
//! before backend dispatch. The composition layer wires the alias to a
//! tenant/user-scoped [`VirtualPath`](ironclaw_host_api::VirtualPath), so
//! tenant isolation is structural — two services constructed over
//! different `MountView`s against the same `RootFilesystem` cannot see
//! each other's state. The store does not encode `tenant_id` / `user_id`
//! in the path itself.
//!
//! The on-disk layout under the `/conversations` mount alias is a single
//! JSON blob:
//!
//! ```text
//! /conversations/state.json
//! ```
//!
//! State revisions are tracked two ways. `revision` on the persisted
//! record carries the existing [`ConversationStateRepository`] contract
//! (`save_state(expected_revision, state)`); the underlying filesystem
//! also returns a `RecordVersion` which we use as the
//! [`CasExpectation`] for write CAS. Backends that don't implement
//! versioned `put` reject `CasExpectation::Version` with `Unsupported`;
//! for those we fall back to `CasExpectation::Any` and rely on the
//! in-process serialization the caller
//! (`InMemoryConversationServices::mutation_lock`) already provides —
//! same shape as the secrets / processes / run-state stores.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, IndexKey, IndexValue,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::UserId;
use ironclaw_host_api::{HostApiError, ResourceScope, ScopedPath};
use ironclaw_turns::{AcceptedMessageRef, IdempotencyKey, SubmitTurnResponse};
use serde::{Deserialize, Serialize};

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageLookup,
    AcceptedInboundMessageReplay, AdapterInstallationId, AdapterKind, ConditionalUnpairOutcome,
    ConversationActorPairingService, ConversationBindingResolution, ConversationBindingService,
    ExpectedExternalActorOwner, ExternalActorBindingEpoch, ExternalActorRef,
    ExternalConversationIdentity, InMemoryConversationServices, InboundTurnError,
    LinkConversationRequest, LinkedConversationBinding, ReplyTargetBinding,
    ResolveConversationRequest, SessionThreadService, ThreadMessageRecord,
    ValidateReplyTargetRequest,
    memory::{
        AcceptedMessageReplayKey, ActorKey, BindingKey, BindingRecord, ExternalEventRouteKey,
        InMemoryState, MessageIdempotencyKey, ReplyTargetRecord, StoredAcceptedMessageReplay,
        ThreadKey, ThreadRecord,
    },
    state_store::{ConversationStateRepository, PersistedConversationState},
};

const STATE_PREFIX: &str = "/conversations";

/// Maximum number of compare-and-swap retries before
/// [`FilesystemConversationStateStore::save_state`] returns a
/// `DurableState` error. Five attempts mirrors the budgets used by the
/// secrets, processes, and run-state stores — enough to absorb common
/// contention while failing loudly on pathological loops.
const FILESYSTEM_CAS_RETRIES: usize = 5;

/// Filesystem-backed conversation state store under the `/conversations`
/// mount alias.
///
/// Construct with an [`Arc<ScopedFilesystem<F>>`] over any
/// [`RootFilesystem`]. Tenant/user isolation lives in the caller's
/// [`MountView`](ironclaw_host_api::MountView), not in this store.
pub struct FilesystemConversationStateStore<F: ?Sized>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemConversationStateStore<F>
where
    F: RootFilesystem + ?Sized,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }
}

/// On-disk envelope persisted at `/conversations/state.json`.
///
/// Carries the trait-level revision counter alongside the in-memory state
/// so the existing [`ConversationStateRepository`] CAS contract
/// (`save_state(expected_revision, ...)`) survives the migration. The
/// filesystem layer additionally provides a `RecordVersion` which we use
/// as the inner [`CasExpectation`] — `revision` is the consumer-facing
/// monotone counter and `RecordVersion` is the per-op concurrency token.
///
/// JSON requires HashMap keys to be strings, but `InMemoryState` keys
/// several maps by struct values (`ActorKey`, `BindingKey`, `ThreadKey`,
/// `ExternalEventRouteKey`, `MessageIdempotencyKey`,
/// `AcceptedMessageReplayKey`, `AcceptedMessageRef`). Wire those as
/// `Vec<(K, V)>` and rebuild the HashMaps on load. The legacy
/// libSQL/Postgres adapters did the same shape via individual rows;
/// this envelope keeps the equivalent contract within a single JSON
/// document.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredConversationState {
    revision: i64,
    pairings: Vec<(ActorKey, UserId)>,
    #[serde(default)]
    pairing_epochs: Vec<(ActorKey, ExternalActorBindingEpoch)>,
    bindings: Vec<(BindingKey, BindingRecord)>,
    source_bindings: HashMap<String, BindingRecord>,
    reply_targets: HashMap<String, ReplyTargetRecord>,
    threads: Vec<(ThreadKey, ThreadRecord)>,
    external_event_routes: Vec<(ExternalEventRouteKey, ExternalConversationIdentity)>,
    message_idempotency: Vec<(MessageIdempotencyKey, AcceptedInboundMessage)>,
    message_replays: Vec<(AcceptedMessageReplayKey, StoredAcceptedMessageReplay)>,
    submission_keys: Vec<(AcceptedMessageRef, IdempotencyKey)>,
    submitted_message_responses: Vec<(AcceptedMessageRef, SubmitTurnResponse)>,
    messages: Vec<ThreadMessageRecord>,
}

impl StoredConversationState {
    fn from_state(revision: i64, state: &InMemoryState) -> Self {
        Self {
            revision,
            pairings: state
                .pairings
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            pairing_epochs: state
                .pairing_epochs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            bindings: state
                .bindings
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            source_bindings: state.source_bindings.clone(),
            reply_targets: state.reply_targets.clone(),
            threads: state
                .threads
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            external_event_routes: state
                .external_event_routes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            message_idempotency: state
                .message_idempotency
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            message_replays: state
                .message_replays
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            submission_keys: state
                .submission_keys
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            submitted_message_responses: state
                .submitted_message_responses
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            messages: state.messages.clone(),
        }
    }

    fn into_state(self) -> InMemoryState {
        InMemoryState {
            persistence_revision: 0,
            pairings: self.pairings.into_iter().collect(),
            pairing_epochs: self.pairing_epochs.into_iter().collect(),
            bindings: self.bindings.into_iter().collect(),
            source_bindings: self.source_bindings,
            reply_targets: self.reply_targets,
            threads: self.threads.into_iter().collect(),
            external_event_routes: self.external_event_routes.into_iter().collect(),
            message_idempotency: self.message_idempotency.into_iter().collect(),
            message_replays: self.message_replays.into_iter().collect(),
            submission_keys: self.submission_keys.into_iter().collect(),
            submitted_message_responses: self.submitted_message_responses.into_iter().collect(),
            messages: self.messages,
        }
    }
}

#[async_trait]
impl<F> ConversationStateRepository for FilesystemConversationStateStore<F>
where
    F: RootFilesystem + ?Sized,
{
    async fn load_state(&self) -> Result<PersistedConversationState, InboundTurnError> {
        let path = state_path()?;
        // Conversation state is a single process-wide singleton record;
        // route through the system scope.
        let scope = ResourceScope::system();
        let Some(versioned) = self
            .filesystem
            .get(&scope, &path)
            .await
            .map_err(filesystem_error)?
        else {
            return Ok(PersistedConversationState {
                state: InMemoryState::default(),
                revision: 0,
            });
        };
        let stored: StoredConversationState = deserialize(&versioned.entry.body)?;
        let revision = stored.revision;
        Ok(PersistedConversationState {
            state: stored.into_state(),
            revision,
        })
    }

    async fn save_state(
        &self,
        expected_revision: i64,
        state: &InMemoryState,
    ) -> Result<i64, InboundTurnError> {
        let path = state_path()?;
        let new_revision =
            expected_revision
                .checked_add(1)
                .ok_or_else(|| InboundTurnError::DurableState {
                    reason: "conversation state revision overflow".to_string(),
                })?;
        let stored = StoredConversationState::from_state(new_revision, state);
        let body = serialize(&stored)?;
        let scope = ResourceScope::system();

        for _ in 0..FILESYSTEM_CAS_RETRIES {
            let current = self
                .filesystem
                .get(&scope, &path)
                .await
                .map_err(filesystem_error)?;
            let cas = match &current {
                None if expected_revision == 0 => CasExpectation::Absent,
                None => {
                    return Err(InboundTurnError::DurableState {
                        reason: "stale conversation state revision".to_string(),
                    });
                }
                Some(versioned) => {
                    let existing: StoredConversationState = deserialize(&versioned.entry.body)?;
                    if existing.revision != expected_revision {
                        return Err(InboundTurnError::DurableState {
                            reason: "stale conversation state revision".to_string(),
                        });
                    }
                    CasExpectation::Version(versioned.version)
                }
            };
            let entry = state_entry(body.clone(), state);
            match put_with_byte_fallback(&self.filesystem, &scope, &path, entry, cas).await {
                Ok(()) => return Ok(new_revision),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error(error)),
            }
        }
        Err(InboundTurnError::DurableState {
            reason: format!(
                "filesystem CAS retries exhausted for path {}",
                path.as_str()
            ),
        })
    }
}

fn state_path() -> Result<ScopedPath, InboundTurnError> {
    ScopedPath::new(format!("{STATE_PREFIX}/state.json")).map_err(invalid_path)
}

/// Build the [`Entry`] persisted at `state_path()`.
///
/// Defense-in-depth: every write carries a `tenant_ids` indexed
/// projection alongside the path-prefix scope so an admin-tier query
/// can filter, and a path-rewriting bug surfaces as a query-time
/// mismatch rather than silent cross-tenant leakage — same shape as the
/// processes / secrets / outbound / authorization migrations.
///
/// Unlike those stores the conversation state is a multi-tenant blob in
/// shape today (the [`InMemoryState`] keys actor/binding/thread maps by
/// `tenant_id`), so the projection lists every tenant present in the
/// snapshot rather than a single scope-supplied value. Under a
/// per-invocation `MountView` the blob is sliced to one tenant per
/// resolved path, so in practice the projection holds zero or one
/// tenant id; the multi-tenant case is the single-tenant default mount
/// view used for tests and the long-lived composition.
fn state_entry(body: Vec<u8>, state: &InMemoryState) -> Entry {
    let tenant_ids = collect_tenant_ids(state);
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    if !tenant_ids.is_empty() {
        entry = entry.with_indexed(
            index_key_tenant_ids(),
            IndexValue::Text(tenant_ids.join(",")),
        );
    }
    entry
}

/// Collect a sorted, deduplicated list of `tenant_id`s present in the
/// state snapshot. Sources every map keyed by tenant (`pairings`,
/// `bindings`, `threads`, `external_event_routes`, `message_idempotency`,
/// `message_replays`) so a state with empty `pairings` but populated
/// `bindings` still surfaces in the projection.
fn collect_tenant_ids(state: &InMemoryState) -> Vec<String> {
    let mut tenants: BTreeSet<String> = BTreeSet::new();
    for actor_key in state.pairings.keys() {
        tenants.insert(actor_key.tenant_id.as_str().to_string());
    }
    for binding_key in state.bindings.keys() {
        tenants.insert(binding_key.tenant_id.as_str().to_string());
    }
    for thread_key in state.threads.keys() {
        tenants.insert(thread_key.tenant_id.as_str().to_string());
    }
    for route_key in state.external_event_routes.keys() {
        tenants.insert(route_key.tenant_id.as_str().to_string());
    }
    for idempotency_key in state.message_idempotency.keys() {
        tenants.insert(idempotency_key.tenant_id.as_str().to_string());
    }
    for replay_key in state.message_replays.keys() {
        tenants.insert(replay_key.tenant_id.as_str().to_string());
    }
    tenants.into_iter().collect()
}

fn index_key_tenant_ids() -> IndexKey {
    IndexKey::new("tenant_ids")
        .unwrap_or_else(|_| unreachable!("tenant_ids is a simple ascii identifier"))
}

/// `put` with a fallback to opaque (byte-only) writes on `Unsupported`.
///
/// Mirrors the secrets / processes / run-state stores: record-shaped
/// entries and non-`Any` CAS are stripped/downgraded when the backend
/// reports `Unsupported` so byte-only mounts (DiskFilesystem) keep
/// working. The single-instance `mutation_lock` on
/// [`InMemoryConversationServices`] is the caller-side ordering guarantee
/// that CAS would otherwise provide on byte-only backends.
async fn put_with_byte_fallback<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<(), FilesystemError>
where
    F: RootFilesystem + ?Sized,
{
    let fallback = entry.clone();
    match filesystem.put(scope, path, entry, cas).await {
        Ok(_) => Ok(()),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => filesystem
            .put(scope, path, fallback, CasExpectation::Any)
            .await
            .map(|_| ()),
        Err(error) => Err(error),
    }
}

fn serialize<T>(value: &T) -> Result<Vec<u8>, InboundTurnError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(|error| InboundTurnError::DurableState {
        reason: error.to_string(),
    })
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, InboundTurnError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| InboundTurnError::DurableState {
        reason: error.to_string(),
    })
}

fn invalid_path(error: HostApiError) -> InboundTurnError {
    InboundTurnError::DurableState {
        reason: error.to_string(),
    }
}

fn filesystem_error(error: FilesystemError) -> InboundTurnError {
    InboundTurnError::DurableState {
        reason: error.to_string(),
    }
}

/// Filesystem-backed equivalent of the legacy
/// `RebornLibSqlConversationServices` / `RebornPostgresConversationServices`
/// wrappers. Wires an [`InMemoryConversationServices`] over a
/// [`FilesystemConversationStateStore`] so the in-process state is
/// rehydrated from the filesystem at construction and every mutation
/// reaches durable storage before returning.
///
/// Backend selection (libSQL / Postgres / in-memory / local-disk) is now
/// a property of the underlying [`RootFilesystem`], not of this service —
/// callers construct `Arc<ScopedFilesystem<F>>` once and reuse it across
/// every consumer store under the same `MountView`.
#[derive(Clone)]
pub struct RebornFilesystemConversationServices {
    inner: InMemoryConversationServices,
}

impl RebornFilesystemConversationServices {
    pub async fn new<F>(filesystem: Arc<ScopedFilesystem<F>>) -> Result<Self, InboundTurnError>
    where
        F: RootFilesystem + ?Sized + 'static,
    {
        let store = Arc::new(FilesystemConversationStateStore::new(filesystem));
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
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
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
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
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

    pub async fn pair_external_actor_with_epoch(
        &self,
        tenant_id: ironclaw_host_api::TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: ironclaw_host_api::UserId,
        binding_epoch: ExternalActorBindingEpoch,
    ) -> Result<(), InboundTurnError> {
        self.inner
            .pair_external_actor_with_epoch(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
                user_id,
                binding_epoch,
            )
            .await
    }

    pub async fn unpair_external_actor_if_owned_by(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
        expected: &ExpectedExternalActorOwner,
    ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
        self.inner
            .unpair_external_actor_if_owned_by(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
                expected,
            )
            .await
    }

    /// Remove all pairings and direct conversation routes owned by one user
    /// for an adapter, optionally narrowed to one installation.
    pub async fn unpair_external_actors_owned_by(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: Option<&AdapterInstallationId>,
        user_id: &UserId,
    ) -> Result<usize, InboundTurnError> {
        self.inner
            .unpair_external_actors_owned_by(
                tenant_id,
                adapter_kind,
                adapter_installation_id,
                user_id,
            )
            .await
    }
}

#[async_trait]
impl ConversationActorPairingService for RebornFilesystemConversationServices {
    async fn pair_external_actor(
        &self,
        tenant_id: ironclaw_host_api::TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
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

    async fn pair_external_actor_with_epoch(
        &self,
        tenant_id: ironclaw_host_api::TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
        binding_epoch: ExternalActorBindingEpoch,
    ) -> Result<(), InboundTurnError> {
        self.inner
            .pair_external_actor_with_epoch(
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
        tenant_id: ironclaw_host_api::TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
    ) -> Result<(), InboundTurnError> {
        self.inner
            .try_unpair_external_actor(
                &tenant_id,
                &adapter_kind,
                &adapter_installation_id,
                &external_actor_ref,
            )
            .await
    }

    async fn unpair_external_actor_if_owned_by(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        adapter_kind: &AdapterKind,
        adapter_installation_id: &AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
        expected: &ExpectedExternalActorOwner,
    ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
        self.inner
            .unpair_external_actor_if_owned_by(
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
impl ConversationBindingService for RebornFilesystemConversationServices {
    async fn resolve_or_create_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        self.inner.resolve_or_create_binding(request).await
    }

    async fn resolve_or_create_binding_with_trusted_scope(
        &self,
        request: ResolveConversationRequest,
        trusted_agent_id: Option<ironclaw_host_api::AgentId>,
        trusted_project_id: Option<ironclaw_host_api::ProjectId>,
        trusted_owner_user_id: Option<ironclaw_host_api::UserId>,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        self.inner
            .resolve_or_create_binding_with_trusted_scope(
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
        self.inner.lookup_binding(request).await
    }

    async fn link_conversation_to_thread(
        &self,
        request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError> {
        self.inner.link_conversation_to_thread(request).await
    }

    async fn validate_reply_target(
        &self,
        request: ValidateReplyTargetRequest,
    ) -> Result<ReplyTargetBinding, InboundTurnError> {
        self.inner.validate_reply_target(request).await
    }

    async fn resolve_stored_reply_target(
        &self,
        request: crate::ResolveStoredReplyTargetRequest,
    ) -> Result<crate::StoredReplyTargetBinding, InboundTurnError> {
        self.inner.resolve_stored_reply_target(request).await
    }
}

#[async_trait]
impl SessionThreadService for RebornFilesystemConversationServices {
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, InboundTurnError> {
        self.inner.accept_inbound_message(request).await
    }

    async fn replay_accepted_inbound_message(
        &self,
        lookup: AcceptedInboundMessageLookup,
    ) -> Result<Option<AcceptedInboundMessageReplay>, InboundTurnError> {
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
