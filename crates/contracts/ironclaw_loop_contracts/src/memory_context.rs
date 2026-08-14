//! Memory prompt context service port for the agent loop host.
//!
//! This module defines the [`MemoryPromptContextService`] trait — the loop-host
//! port that produces memory snippets for [`LoopContextBundle::memory_snippets`]
//! from a tenant/user/agent/project-scoped memory search.
//!
//! # Isolation guarantees
//!
//! Every request carries a [`TurnScope`] and [`TurnActor`] that together define
//! the tenant/user/agent/project boundary. Implementations must derive
//! [`MemoryDocumentScope`](ironclaw_memory::path::MemoryDocumentScope) from
//! these fields and pass it through to the memory backend so that cross-tenant
//! and cross-user isolation is enforced at the storage layer.
//!
//! # Determinism contract
//!
//! For the same run snapshot (identical backend results), the returned snippet
//! list must be ordered deterministically — score descending, then path
//! ascending — so that two calls with the same inputs produce identical output.

use async_trait::async_trait;

use super::host::{AgentLoopHostError, LoopContextSnippet};
use super::refs::ContextProfileId;
use ironclaw_host_api::turn::{TurnActor, TurnScope};

/// Request to load memory snippets for the current loop context.
#[derive(Debug, Clone)]
pub struct MemoryPromptContextRequest {
    /// Tenant/agent/project/thread isolation scope.
    pub scope: TurnScope,
    /// The acting user, used to derive `MemoryDocumentScope.user_id`.
    pub actor: TurnActor,
    /// Search query string, typically derived from the recent user message or
    /// thread context.
    pub query: String,
    /// Upper bound on the number of snippets returned.
    pub max_snippets: usize,
    /// Which context policy applies to this load.
    pub context_profile_id: ContextProfileId,
}

/// Port trait for loading memory snippets into the loop context bundle.
///
/// # Isolation guarantees
///
/// Implementations must enforce tenant/user/agent/project isolation by deriving
/// a [`MemoryDocumentScope`](ironclaw_memory::path::MemoryDocumentScope) from
/// the request's [`TurnScope`] and [`TurnActor`] fields. The scope must be
/// passed to the underlying memory backend so that cross-tenant and cross-user
/// data never leaks into a run's context.
///
/// # Determinism contract
///
/// For the same backend results, the returned snippet list must be ordered
/// deterministically (score descending, then path ascending) so that identical
/// inputs produce identical output across calls.
///
/// # Error handling
///
/// Backend failures must be mapped to [`AgentLoopHostError`] with
/// [`AgentLoopHostErrorKind::Unavailable`](super::host::AgentLoopHostErrorKind::Unavailable).
/// Raw backend error messages, filesystem paths, and internal identifiers must
/// never appear in the error's `safe_summary`.
/// Which retrieval lane a degradation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRetrievalLane {
    ShortTerm,
    LongTerm,
}

impl MemoryRetrievalLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShortTerm => "short_term",
            Self::LongTerm => "long_term",
        }
    }
}

/// Why a retrieval lane produced nothing. A closed vocabulary, never free text
/// and never a backend message — the label is safe to record and to render in
/// operator surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRetrievalFailureKind {
    /// The request itself was rejected (bad query or profile).
    Input,
    /// The provider or its storage backend was unreachable or errored.
    Unavailable,
}

impl MemoryRetrievalFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One lane that failed rather than returning nothing.
///
/// This is the distinction the previous contract could not express: a lane
/// error and a lane with no matching memory both arrived as an empty snippet
/// list, so "the memory backend is down" was indistinguishable from "the user
/// has no memory about this" in tests and in operator diagnostics alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRetrievalDegradation {
    pub lane: MemoryRetrievalLane,
    pub kind: MemoryRetrievalFailureKind,
}

impl MemoryRetrievalDegradation {
    pub fn new(lane: MemoryRetrievalLane, kind: MemoryRetrievalFailureKind) -> Self {
        Self { lane, kind }
    }
}

/// The result of one memory prompt-context load: the snippets that were
/// admitted, plus a typed record of every lane that failed on the way.
///
/// Mirrors [`LoopContextBundle`](super::host::LoopContextBundle), which carries
/// its successful payload alongside an explicit description of what was lost
/// (`recent_window_truncation`). Memory retrieval is best-effort — a failing
/// lane must never fail the turn — but "best-effort" is not "invisible":
/// `degradations` is empty exactly when every queried lane answered.
#[derive(Debug, Clone, Default)]
pub struct MemoryPromptContextLoad {
    pub snippets: Vec<LoopContextSnippet>,
    pub degradations: Vec<MemoryRetrievalDegradation>,
}

impl MemoryPromptContextLoad {
    /// A load where every queried lane answered.
    pub fn healthy(snippets: Vec<LoopContextSnippet>) -> Self {
        Self {
            snippets,
            degradations: Vec::new(),
        }
    }

    /// Whether any queried lane failed rather than simply matching nothing.
    pub fn is_degraded(&self) -> bool {
        !self.degradations.is_empty()
    }
}

#[async_trait]
pub trait MemoryPromptContextService: Send + Sync {
    async fn load_memory_snippets(
        &self,
        request: MemoryPromptContextRequest,
    ) -> Result<MemoryPromptContextLoad, AgentLoopHostError>;
}

/// No-op implementation that always returns an empty snippet list.
///
/// Used for backward compatibility and composability when no memory backend is
/// configured. This preserves the existing behavior where
/// `LoopContextBundle.memory_snippets` is always `Vec::new()`.
pub struct EmptyMemoryPromptContextService;

#[async_trait]
impl MemoryPromptContextService for EmptyMemoryPromptContextService {
    async fn load_memory_snippets(
        &self,
        _request: MemoryPromptContextRequest,
    ) -> Result<MemoryPromptContextLoad, AgentLoopHostError> {
        Ok(MemoryPromptContextLoad::default())
    }
}
