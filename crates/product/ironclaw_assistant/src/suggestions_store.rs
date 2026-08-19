//! Durable, authenticated caller-context-scoped suggestion state.
//!
//! Suggestions follow the authenticated user across product surfaces. There is
//! exactly one document per tenant/user pair; agent and project are caller
//! context for the canonical run, not additional projection keys. Mutations
//! are single-document CAS updates; running turns and creating threads remain
//! outside this module and are independently idempotent.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{CasApply, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ids::{AgentId, ProjectId, ThreadId};
use ironclaw_host_api::{resource::ResourceScope, turn::TurnRunId};
use ironclaw_product_contracts::inbound_requests::SUGGESTIONS_CLIENT_ACTION_ID_MAX_BYTES;
use serde::{Deserialize, Serialize};

#[path = "suggestions_store_filesystem.rs"]
mod filesystem;
#[cfg(test)]
use filesystem::document_entry;
use filesystem::{decode_document, document_path, filesystem_error};

const SUGGESTION_DOCUMENT_SCHEMA_VERSION: u32 = 1;
const SUGGESTION_DOCUMENT_RECORD_KIND: &str = "suggestion_document";
const SUGGESTION_DOCUMENT_ROOT: &str = "/suggestions/contexts";
const MAX_FAILURE_REASON_CHARS: usize = 160;
const MAX_ID_CHARS: usize = 128;
const SAFE_FAILURE_REASON: &str = "suggestion generation failed";

/// Stable identity used inside the suggestion domain. Persisted ids remain
/// plain JSON strings, while Rust code cannot accidentally compare a
/// suggestion id with a generation id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SuggestionId(String);

impl SuggestionId {
    pub fn new(value: impl Into<String>) -> Result<Self, SuggestionsStoreError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    fn validate(value: &str) -> Result<(), SuggestionsStoreError> {
        validate_id("suggestion", value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for SuggestionId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for SuggestionId {
    type Error = SuggestionsStoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for SuggestionId {
    type Error = SuggestionsStoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SuggestionId> for String {
    fn from(value: SuggestionId) -> Self {
        value.0
    }
}

impl std::fmt::Display for SuggestionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Stable identity for one persisted generation attempt.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct GenerationId(String);

impl GenerationId {
    pub fn new(value: impl Into<String>) -> Result<Self, SuggestionsStoreError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    fn validate(value: &str) -> Result<(), SuggestionsStoreError> {
        validate_id("generation", value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for GenerationId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for GenerationId {
    type Error = SuggestionsStoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for GenerationId {
    type Error = SuggestionsStoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<GenerationId> for String {
    fn from(value: GenerationId) -> Self {
        value.0
    }
}

impl std::fmt::Display for GenerationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

fn validate_id(kind: &'static str, value: &str) -> Result<(), SuggestionsStoreError> {
    if value.is_empty()
        || value.chars().count() > MAX_ID_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(SuggestionsStoreError::InvalidId {
            kind,
            reason: "id must be non-empty, bounded and control-free".to_string(),
        });
    }
    Ok(())
}

/// Strongly typed caller idempotency key for a suggestion generation.
///
/// The request DTO remains a plain string at the product boundary; once it
/// enters durable suggestion state, it cannot be confused with a generation
/// or card id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct GenerationActionId(String);

impl GenerationActionId {
    pub fn new(value: impl Into<String>) -> Result<Self, SuggestionsStoreError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > SUGGESTIONS_CLIENT_ACTION_ID_MAX_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(SuggestionsStoreError::InvalidId {
                kind: "generation action",
                reason: "id must be non-empty, bounded and control-free".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GenerationActionId {
    type Error = SuggestionsStoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for GenerationActionId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for GenerationActionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One durable, caller-scoped suggestion snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionDocument {
    pub schema_version: u32,
    pub generation: GenerationState,
    /// Terminal generations are retained as durable idempotency evidence.
    /// Their cards remain in `suggestions`, but only the current ready
    /// generation is projected to product surfaces.
    #[serde(default)]
    pub generation_history: Vec<GenerationHistoryEntry>,
    pub suggestions: Vec<SuggestionRecord>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SuggestionDocument {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            schema_version: SUGGESTION_DOCUMENT_SCHEMA_VERSION,
            generation: GenerationState::Never,
            generation_history: Vec::new(),
            suggestions: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Durable generation progress and replay facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationState {
    Never,
    /// Intent claimed, but the canonical unbound turn has not yet been
    /// durably accepted. The request lease only fences this short CAS window;
    /// it is never used to decide terminal settlement or take over a run.
    Generating {
        generation_id: GenerationId,
        public_id: String,
        accept_key: String,
        /// The caller-owned idempotency key for the generation request.  It is
        /// retained so a crash before run binding can replay the exact same
        /// prepared context and turn submission.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_action_id: Option<GenerationActionId>,
        prompt_schema_version: u32,
        run_id: Option<TurnRunId>,
        lease_owner: String,
        lease_expires_at: DateTime<Utc>,
    },
    /// The canonical unbound run has been accepted.  This state has no
    /// request-owned lease: the process scheduler and its durable journal own
    /// execution, while a commit observer owns materialization.
    Pending {
        generation_id: GenerationId,
        public_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_action_id: Option<GenerationActionId>,
        run_id: TurnRunId,
    },
    Ready {
        generation_id: GenerationId,
        completed_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_action_id: Option<GenerationActionId>,
    },
    Failed {
        generation_id: GenerationId,
        failed_at: DateTime<Utc>,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_action_id: Option<GenerationActionId>,
    },
}

/// A terminal generation retained for replay after a later generation has
/// replaced the visible projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationHistoryEntry {
    pub generation_id: GenerationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<GenerationActionId>,
    pub state: GenerationHistoryState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationHistoryState {
    Ready {
        completed_at: DateTime<Utc>,
    },
    Failed {
        failed_at: DateTime<Utc>,
        reason: String,
    },
}

impl GenerationHistoryEntry {
    fn as_generation_state(&self) -> GenerationState {
        match &self.state {
            GenerationHistoryState::Ready { completed_at } => GenerationState::Ready {
                generation_id: self.generation_id.clone(),
                completed_at: *completed_at,
                client_action_id: self.client_action_id.clone(),
            },
            GenerationHistoryState::Failed { failed_at, reason } => GenerationState::Failed {
                generation_id: self.generation_id.clone(),
                failed_at: *failed_at,
                reason: reason.clone(),
                client_action_id: self.client_action_id.clone(),
            },
        }
    }
}

fn generation_history_entry(state: &GenerationState) -> Option<GenerationHistoryEntry> {
    match state {
        GenerationState::Ready {
            generation_id,
            completed_at,
            client_action_id,
        } => Some(GenerationHistoryEntry {
            generation_id: generation_id.clone(),
            client_action_id: client_action_id.clone(),
            state: GenerationHistoryState::Ready {
                completed_at: *completed_at,
            },
        }),
        GenerationState::Failed {
            generation_id,
            failed_at,
            reason,
            client_action_id,
        } => Some(GenerationHistoryEntry {
            generation_id: generation_id.clone(),
            client_action_id: client_action_id.clone(),
            state: GenerationHistoryState::Failed {
                failed_at: *failed_at,
                reason: reason.clone(),
            },
        }),
        _ => None,
    }
}

/// The durable identity needed to settle one terminal unbound run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenerationTerminalCorrelation {
    pub(crate) generation_id: GenerationId,
    pub(crate) client_action_id: Option<GenerationActionId>,
}

/// Correlate a terminal process commit with its durable generation intent.
///
/// The pre-bind `Generating` window has no run id yet. Its public id is still
/// a durable unique identity, so a matching terminal commit may settle it
/// before the request has persisted the run binding.
pub(crate) fn terminal_generation_correlation(
    state: &GenerationState,
    public_id: &str,
    run_id: TurnRunId,
) -> Option<GenerationTerminalCorrelation> {
    match state {
        GenerationState::Pending {
            public_id: current_public_id,
            generation_id,
            client_action_id,
            run_id: current_run_id,
            ..
        } if current_public_id == public_id && *current_run_id == run_id => {
            Some(GenerationTerminalCorrelation {
                generation_id: generation_id.clone(),
                client_action_id: client_action_id.clone(),
            })
        }
        GenerationState::Generating {
            public_id: current_public_id,
            generation_id,
            client_action_id,
            run_id: current_run_id,
            ..
        } if current_public_id == public_id
            && (current_run_id.is_none() || *current_run_id == Some(run_id)) =>
        {
            Some(GenerationTerminalCorrelation {
                generation_id: generation_id.clone(),
                client_action_id: client_action_id.clone(),
            })
        }
        _ => None,
    }
}

/// Result of atomically attempting to own generation execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationLeaseClaim {
    Acquired {
        state: GenerationState,
    },
    CurrentReplay {
        state: GenerationState,
    },
    /// The request key matched a terminal generation that is no longer the
    /// current visible projection.
    HistoricalReplay {
        state: GenerationState,
    },
}

impl GenerationLeaseClaim {
    pub fn state(&self) -> &GenerationState {
        match self {
            Self::Acquired { state }
            | Self::CurrentReplay { state }
            | Self::HistoricalReplay { state } => state,
        }
    }

    pub fn is_acquired(&self) -> bool {
        matches!(self, Self::Acquired { .. })
    }

    pub fn is_historical_replay(&self) -> bool {
        matches!(self, Self::HistoricalReplay { .. })
    }
}

#[derive(Debug, Clone)]
pub struct BeginGenerationRequest {
    pub generation_id: GenerationId,
    pub public_id: String,
    pub accept_key: String,
    pub client_action_id: Option<String>,
    pub prompt_schema_version: u32,
    pub lease_owner: String,
    pub lease_expires_at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

/// A retained generated card.  Dismissal and start binding are never removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionRecord {
    pub id: SuggestionId,
    pub title: String,
    pub description: String,
    pub suggested_prompt: String,
    pub icon: String,
    pub sources: Vec<String>,
    pub generation_id: GenerationId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub dismissed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_reservation: Option<SuggestionStartReservation>,
    pub binding: Option<SuggestionBinding>,
}

/// Stable product idempotency keys reserved before any thread/turn side effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionStartReservation {
    pub thread_action_id: String,
    pub turn_action_id: String,
    /// The caller context selected when the start was first reserved. These
    /// fields are immutable once persisted so a retry cannot move an
    /// accepted start into another agent or project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

/// Result of claiming a suggestion start operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionStartClaim {
    Reserved(SuggestionStartReservation),
    Bound(SuggestionBinding),
}

/// Durable link from a suggestion to its normal visible thread and turn run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionBinding {
    pub thread_id: ThreadId,
    pub run_id: TurnRunId,
}

#[derive(Debug, thiserror::Error)]
pub enum SuggestionsStoreError {
    #[error("suggestion document serialization failed: {reason}")]
    Serialization { reason: String },
    #[error("suggestion store filesystem operation {operation} failed: {reason}")]
    Filesystem {
        operation: &'static str,
        reason: String,
    },
    #[error("invalid {kind} id: {reason}")]
    InvalidId { kind: &'static str, reason: String },
    #[error("suggestion generation {generation_id} is not current")]
    GenerationNotCurrent { generation_id: GenerationId },
    #[error("suggestion generation {generation_id} is already in progress")]
    GenerationInProgress {
        generation_id: GenerationId,
        client_action_id: Option<GenerationActionId>,
    },
    #[error("suggestion {suggestion_id} was not found")]
    SuggestionNotFound { suggestion_id: SuggestionId },
    #[error("suggestion {suggestion_id} is dismissed")]
    SuggestionDismissed { suggestion_id: SuggestionId },
    #[error("suggestion {suggestion_id} already has a different start reservation")]
    StartReservationConflict { suggestion_id: SuggestionId },
}

/// Object-safe persistence port for product suggestion orchestration.
#[async_trait]
pub trait SuggestionsStore: Send + Sync {
    async fn read(
        &self,
        scope: &ResourceScope,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError>;

    async fn begin_generation(
        &self,
        scope: &ResourceScope,
        request: BeginGenerationRequest,
    ) -> Result<GenerationLeaseClaim, SuggestionsStoreError>;

    /// Atomically bind the accepted turn and release the request lease.  The
    /// resulting [`GenerationState::Pending`] is settled only by a durable
    /// process-journal observer, so the HTTP/request future may return.
    async fn bind_generation_run(
        &self,
        scope: &ResourceScope,
        generation_id: &GenerationId,
        lease_owner: &str,
        run_id: TurnRunId,
        now: DateTime<Utc>,
    ) -> Result<SuggestionDocument, SuggestionsStoreError>;

    /// Settle a submitted generation from a terminal run commit.  Matching is
    /// by durable public id and run id rather than a request lease, making
    /// replay and restart idempotent. A terminal commit may arrive while the
    /// request still has the pre-submit `Generating` state, so that state is
    /// also accepted when its run id has not been recorded yet.
    async fn complete_generation_for_run(
        &self,
        scope: &ResourceScope,
        public_id: &str,
        run_id: TurnRunId,
        generated: Vec<SuggestionRecord>,
        now: DateTime<Utc>,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError>;

    async fn fail_generation(
        &self,
        scope: &ResourceScope,
        generation_id: &GenerationId,
        lease_owner: &str,
        reason: String,
        now: DateTime<Utc>,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError>;

    /// Record a bounded safe terminal failure from a durable run commit.
    async fn fail_generation_for_run(
        &self,
        scope: &ResourceScope,
        public_id: &str,
        run_id: TurnRunId,
        reason: String,
        now: DateTime<Utc>,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError>;

    async fn dismiss(
        &self,
        scope: &ResourceScope,
        suggestion_id: &SuggestionId,
        now: DateTime<Utc>,
    ) -> Result<SuggestionDocument, SuggestionsStoreError>;

    async fn reserve_start(
        &self,
        scope: &ResourceScope,
        suggestion_id: &SuggestionId,
        reservation: SuggestionStartReservation,
        now: DateTime<Utc>,
    ) -> Result<SuggestionStartClaim, SuggestionsStoreError>;

    async fn complete_start(
        &self,
        scope: &ResourceScope,
        suggestion_id: &SuggestionId,
        reservation: &SuggestionStartReservation,
        binding: SuggestionBinding,
        now: DateTime<Utc>,
    ) -> Result<SuggestionBinding, SuggestionsStoreError>;
}

/// Filesystem implementation of the assistant-local suggestion document.
pub struct FilesystemSuggestionsStore<F: RootFilesystem + ?Sized> {
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F: RootFilesystem + ?Sized> FilesystemSuggestionsStore<F> {
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }
}

#[async_trait]
impl<F> SuggestionsStore for FilesystemSuggestionsStore<F>
where
    F: RootFilesystem + ?Sized + Send + Sync + 'static,
{
    /// Read the document without materializing an empty record.
    async fn read(
        &self,
        scope: &ResourceScope,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError> {
        let path = document_path(scope)?;
        let entry = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(|error| filesystem_error("read suggestion document", error))?;
        entry
            .map(|versioned| decode_document(&versioned.entry.body))
            .transpose()
    }

    /// Atomically claim generation execution when no live owner exists.
    async fn begin_generation(
        &self,
        scope: &ResourceScope,
        request: BeginGenerationRequest,
    ) -> Result<GenerationLeaseClaim, SuggestionsStoreError> {
        let BeginGenerationRequest {
            generation_id,
            public_id,
            accept_key,
            client_action_id,
            prompt_schema_version,
            lease_owner,
            lease_expires_at,
            now,
        } = request;
        let client_action_id = client_action_id.map(GenerationActionId::new).transpose()?;
        self.update(scope, "begin suggestion generation", move |current| {
            let generation_id = generation_id.clone();
            let public_id = public_id.clone();
            let accept_key = accept_key.clone();
            let client_action_id = client_action_id.clone();
            let lease_owner = lease_owner.clone();
            async move {
                let mut document = current.unwrap_or_else(|| SuggestionDocument::new(now));
                if let Some(historical) = document.generation_history.iter().find(|entry| {
                    client_action_id
                        .as_ref()
                        .is_some_and(|action| entry.client_action_id.as_ref() == Some(action))
                }) {
                    let state = historical.as_generation_state();
                    return Ok(CasApply::no_op(
                        document,
                        GenerationLeaseClaim::HistoricalReplay { state },
                    ));
                }
                let new_state = || GenerationState::Generating {
                    generation_id: generation_id.clone(),
                    public_id: public_id.clone(),
                    accept_key: accept_key.clone(),
                    client_action_id: client_action_id.clone(),
                    prompt_schema_version,
                    run_id: None,
                    lease_owner: lease_owner.clone(),
                    lease_expires_at,
                };
                let (state, acquired) = match &document.generation {
                    GenerationState::Never => (new_state(), true),
                    GenerationState::Ready {
                        client_action_id: existing_action,
                        ..
                    }
                    | GenerationState::Failed {
                        client_action_id: existing_action,
                        ..
                    } if client_action_id.is_some() && existing_action == &client_action_id => {
                        (document.generation.clone(), false)
                    }
                    GenerationState::Ready { .. } | GenerationState::Failed { .. } => {
                        (new_state(), true)
                    }
                    GenerationState::Generating {
                        generation_id: existing_generation,
                        public_id: existing_public_id,
                        accept_key: existing_accept_key,
                        client_action_id: existing_client_action_id,
                        prompt_schema_version: existing_prompt_schema_version,
                        run_id: None,
                        lease_expires_at: existing_lease_expires_at,
                        ..
                    } if *existing_lease_expires_at <= now => {
                        // The accept door may have succeeded immediately
                        // before the request crashed, so an expired
                        // pre-bind lease must never mint a replacement
                        // identity.  Reclaim only the fencing lease and keep
                        // the durable public/idempotency keys.  A retry then
                        // replays the same accept and converges on either the
                        // existing run or a single newly accepted run.
                        (
                            GenerationState::Generating {
                                generation_id: existing_generation.clone(),
                                public_id: existing_public_id.clone(),
                                accept_key: existing_accept_key.clone(),
                                client_action_id: existing_client_action_id.clone(),
                                prompt_schema_version: *existing_prompt_schema_version,
                                run_id: None,
                                lease_owner: lease_owner.clone(),
                                lease_expires_at,
                            },
                            true,
                        )
                    }
                    GenerationState::Generating {
                        client_action_id: existing_action,
                        ..
                    }
                    | GenerationState::Pending {
                        client_action_id: existing_action,
                        ..
                    } if client_action_id.is_some() && existing_action == &client_action_id => {
                        (document.generation.clone(), false)
                    }
                    GenerationState::Generating {
                        generation_id: existing_generation,
                        client_action_id: existing_action,
                        ..
                    }
                    | GenerationState::Pending {
                        generation_id: existing_generation,
                        client_action_id: existing_action,
                        ..
                    } => {
                        return Err(SuggestionsStoreError::GenerationInProgress {
                            generation_id: existing_generation.clone(),
                            client_action_id: existing_action.clone(),
                        });
                    }
                };
                if acquired {
                    // A generation replaces the visible projection, while
                    // terminal state and cards remain durable for replay and
                    // audit. Product reads filter both to the current ready
                    // generation.
                    if let Some(history) = generation_history_entry(&document.generation) {
                        document.generation_history.push(history);
                    }
                    document.generation = state.clone();
                    document.updated_at = now;
                }
                Ok(CasApply::new(
                    document,
                    if acquired {
                        GenerationLeaseClaim::Acquired { state }
                    } else {
                        GenerationLeaseClaim::CurrentReplay { state }
                    },
                ))
            }
        })
        .await
    }

    async fn complete_generation_for_run(
        &self,
        scope: &ResourceScope,
        public_id: &str,
        run_id: TurnRunId,
        generated: Vec<SuggestionRecord>,
        now: DateTime<Utc>,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError> {
        let public_id = public_id.to_owned();
        self.update(scope, "settle suggestion generation", move |current| {
            let public_id = public_id.clone();
            let generated = generated.clone();
            async move {
                let Some(mut document) = current else {
                    return Ok(CasApply::no_op(SuggestionDocument::new(now), None));
                };
                let Some(correlation) =
                    terminal_generation_correlation(&document.generation, &public_id, run_id)
                else {
                    return Ok(CasApply::no_op(document, None));
                };

                // A completed generation replaces the visible projection but
                // retains prior records for durable replay and audit.
                document.suggestions.extend(generated);
                document.generation = GenerationState::Ready {
                    generation_id: correlation.generation_id,
                    completed_at: now,
                    client_action_id: correlation.client_action_id,
                };
                document.updated_at = now;
                let outcome = document.clone();
                Ok(CasApply::new(document, Some(outcome)))
            }
        })
        .await
    }

    async fn bind_generation_run(
        &self,
        scope: &ResourceScope,
        generation_id: &GenerationId,
        lease_owner: &str,
        run_id: TurnRunId,
        now: DateTime<Utc>,
    ) -> Result<SuggestionDocument, SuggestionsStoreError> {
        let generation_id = generation_id.clone();
        let lease_owner = lease_owner.to_owned();
        self.update(scope, "bind suggestion generation run", move |current| {
            let generation_id = generation_id.clone();
            let lease_owner = lease_owner.clone();
            async move {
                let Some(mut document) = current else {
                    return Err(SuggestionsStoreError::GenerationNotCurrent {
                        generation_id: generation_id.clone(),
                    });
                };
                let GenerationState::Generating {
                    generation_id: current_generation_id,
                    public_id,
                    client_action_id,
                    run_id: current_run_id,
                    lease_owner: current_owner,
                    ..
                } = document.generation.clone()
                else {
                    return Err(SuggestionsStoreError::GenerationNotCurrent {
                        generation_id: generation_id.clone(),
                    });
                };
                if current_generation_id != generation_id || current_owner != lease_owner {
                    return Err(SuggestionsStoreError::GenerationNotCurrent {
                        generation_id: generation_id.clone(),
                    });
                }
                if let Some(current_run_id) = current_run_id
                    && current_run_id != run_id
                {
                    return Err(SuggestionsStoreError::GenerationNotCurrent {
                        generation_id: generation_id.clone(),
                    });
                }
                document.generation = GenerationState::Pending {
                    generation_id: generation_id.clone(),
                    public_id: public_id.clone(),
                    client_action_id: client_action_id.clone(),
                    run_id,
                };
                document.updated_at = now;
                Ok(CasApply::new(document.clone(), document))
            }
        })
        .await
    }

    /// Record a terminal pre-submit failure. New submitted runs settle through
    /// [`Self::fail_generation_for_run`]. Failed generation responses expose
    /// no active cards, while retained records remain durable.
    async fn fail_generation(
        &self,
        scope: &ResourceScope,
        generation_id: &GenerationId,
        lease_owner: &str,
        reason: String,
        now: DateTime<Utc>,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError> {
        let generation_id = generation_id.clone();
        let lease_owner = lease_owner.to_owned();
        let reason = bounded_failure_reason(reason);
        self.update(scope, "fail suggestion generation", move |current| {
            let generation_id = generation_id.clone();
            let lease_owner = lease_owner.clone();
            let reason = reason.clone();
            async move {
                let Some(mut document) = current else {
                    return Ok(CasApply::no_op(SuggestionDocument::new(now), None));
                };
                let GenerationState::Generating {
                    generation_id: current_generation_id,
                    lease_owner: current_owner,
                    client_action_id,
                    ..
                } = &document.generation
                else {
                    return Ok(CasApply::no_op(document, None));
                };
                if current_generation_id != &generation_id || current_owner != &lease_owner {
                    return Ok(CasApply::no_op(document, None));
                }
                document.generation = GenerationState::Failed {
                    generation_id: generation_id.clone(),
                    failed_at: now,
                    reason: reason.clone(),
                    client_action_id: client_action_id.clone(),
                };
                document.updated_at = now;
                let outcome = document.clone();
                Ok(CasApply::new(document, Some(outcome)))
            }
        })
        .await
    }

    async fn fail_generation_for_run(
        &self,
        scope: &ResourceScope,
        public_id: &str,
        run_id: TurnRunId,
        reason: String,
        now: DateTime<Utc>,
    ) -> Result<Option<SuggestionDocument>, SuggestionsStoreError> {
        let public_id = public_id.to_owned();
        let reason = bounded_failure_reason(reason);
        self.update(
            scope,
            "settle failed suggestion generation",
            move |current| {
                let public_id = public_id.clone();
                let reason = reason.clone();
                async move {
                    let Some(mut document) = current else {
                        return Ok(CasApply::no_op(SuggestionDocument::new(now), None));
                    };
                    let Some(correlation) =
                        terminal_generation_correlation(&document.generation, &public_id, run_id)
                    else {
                        return Ok(CasApply::no_op(document, None));
                    };
                    document.generation = GenerationState::Failed {
                        generation_id: correlation.generation_id,
                        failed_at: now,
                        reason,
                        client_action_id: correlation.client_action_id,
                    };
                    document.updated_at = now;
                    let outcome = document.clone();
                    Ok(CasApply::new(document, Some(outcome)))
                }
            },
        )
        .await
    }

    /// Soft-dismiss an active suggestion. The record remains retained.
    async fn dismiss(
        &self,
        scope: &ResourceScope,
        suggestion_id: &SuggestionId,
        now: DateTime<Utc>,
    ) -> Result<SuggestionDocument, SuggestionsStoreError> {
        let suggestion_id = suggestion_id.clone();
        self.update(scope, "dismiss suggestion", move |current| {
            let suggestion_id = suggestion_id.clone();
            async move {
                let mut document =
                    current.ok_or_else(|| SuggestionsStoreError::SuggestionNotFound {
                        suggestion_id: suggestion_id.clone(),
                    })?;
                let current_generation_id = match &document.generation {
                    GenerationState::Ready { generation_id, .. } => generation_id,
                    _ => {
                        return Err(SuggestionsStoreError::SuggestionNotFound {
                            suggestion_id: suggestion_id.clone(),
                        });
                    }
                };
                let suggestion = document
                    .suggestions
                    .iter_mut()
                    .find(|item| {
                        item.id == suggestion_id && item.generation_id == *current_generation_id
                    })
                    .ok_or_else(|| SuggestionsStoreError::SuggestionNotFound {
                        suggestion_id: suggestion_id.clone(),
                    })?;
                if suggestion.dismissed_at.is_none() {
                    suggestion.dismissed_at = Some(now);
                    suggestion.updated_at = now;
                    document.updated_at = now;
                }
                let outcome = document.clone();
                Ok(CasApply::new(document, outcome))
            }
        })
        .await
    }

    /// Reserve the exact idempotency keys that every replay of this start must
    /// use, before creating a thread or submitting a turn.
    async fn reserve_start(
        &self,
        scope: &ResourceScope,
        suggestion_id: &SuggestionId,
        reservation: SuggestionStartReservation,
        now: DateTime<Utc>,
    ) -> Result<SuggestionStartClaim, SuggestionsStoreError> {
        let suggestion_id = suggestion_id.clone();
        self.update(scope, "reserve suggestion start", move |current| {
            let suggestion_id = suggestion_id.clone();
            let reservation = reservation.clone();
            async move {
                let mut document =
                    current.ok_or_else(|| SuggestionsStoreError::SuggestionNotFound {
                        suggestion_id: suggestion_id.clone(),
                    })?;
                let current_generation_id = match &document.generation {
                    GenerationState::Ready { generation_id, .. } => generation_id.clone(),
                    _ => {
                        return Err(SuggestionsStoreError::SuggestionNotFound {
                            suggestion_id: suggestion_id.clone(),
                        });
                    }
                };
                let suggestion = document
                    .suggestions
                    .iter_mut()
                    .find(|item| item.id == suggestion_id)
                    .ok_or_else(|| SuggestionsStoreError::SuggestionNotFound {
                        suggestion_id: suggestion_id.clone(),
                    })?;
                if suggestion.generation_id != current_generation_id {
                    return Err(SuggestionsStoreError::SuggestionNotFound {
                        suggestion_id: suggestion_id.clone(),
                    });
                }
                if suggestion.dismissed_at.is_some() {
                    return Err(SuggestionsStoreError::SuggestionDismissed {
                        suggestion_id: suggestion_id.clone(),
                    });
                }
                match &suggestion.start_reservation {
                    Some(existing) if existing != &reservation => {
                        return Err(SuggestionsStoreError::StartReservationConflict {
                            suggestion_id: suggestion_id.clone(),
                        });
                    }
                    Some(_) => {}
                    None => {
                        if let Some(binding) = &suggestion.binding {
                            let binding = binding.clone();
                            return Ok(CasApply::no_op(
                                document,
                                SuggestionStartClaim::Bound(binding),
                            ));
                        }
                        suggestion.start_reservation = Some(reservation.clone());
                        suggestion.updated_at = now;
                        document.updated_at = now;
                    }
                }
                if let Some(binding) = &suggestion.binding {
                    let binding = binding.clone();
                    return Ok(CasApply::no_op(
                        document,
                        SuggestionStartClaim::Bound(binding),
                    ));
                }
                Ok(CasApply::new(
                    document,
                    SuggestionStartClaim::Reserved(reservation),
                ))
            }
        })
        .await
    }

    /// Complete a previously reserved start. Replaying the same binding is
    /// idempotent; a mismatched reservation or binding fails closed.
    async fn complete_start(
        &self,
        scope: &ResourceScope,
        suggestion_id: &SuggestionId,
        reservation: &SuggestionStartReservation,
        binding: SuggestionBinding,
        now: DateTime<Utc>,
    ) -> Result<SuggestionBinding, SuggestionsStoreError> {
        let suggestion_id = suggestion_id.clone();
        let reservation = reservation.clone();
        self.update(scope, "complete suggestion start", move |current| {
            let suggestion_id = suggestion_id.clone();
            let reservation = reservation.clone();
            let binding = binding.clone();
            async move {
                let mut document =
                    current.ok_or_else(|| SuggestionsStoreError::SuggestionNotFound {
                        suggestion_id: suggestion_id.clone(),
                    })?;
                let suggestion = document
                    .suggestions
                    .iter_mut()
                    .find(|item| item.id == suggestion_id)
                    .ok_or_else(|| SuggestionsStoreError::SuggestionNotFound {
                        suggestion_id: suggestion_id.clone(),
                    })?;
                if suggestion.start_reservation.as_ref() != Some(&reservation) {
                    return Err(SuggestionsStoreError::StartReservationConflict {
                        suggestion_id: suggestion_id.clone(),
                    });
                }
                if let Some(existing) = &suggestion.binding {
                    if existing != &binding {
                        return Err(SuggestionsStoreError::StartReservationConflict {
                            suggestion_id: suggestion_id.clone(),
                        });
                    }
                    let existing = existing.clone();
                    return Ok(CasApply::no_op(document, existing));
                }
                suggestion.binding = Some(binding.clone());
                suggestion.updated_at = now;
                document.updated_at = now;
                Ok(CasApply::new(document, binding))
            }
        })
        .await
    }
}

fn bounded_failure_reason(reason: String) -> String {
    let bounded: String = reason
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_FAILURE_REASON_CHARS)
        .collect();
    if bounded.is_empty() {
        SAFE_FAILURE_REASON.to_string()
    } else {
        bounded
    }
}

#[cfg(test)]
#[path = "suggestions_store_tests.rs"]
mod tests;
