//! Memory-surface declaration vocabulary (`[memory]` in a v3 manifest).
//!
//! An extension declares `[memory]` to say "I am a backend for the host's
//! memory adapter." The host owns the stable `ironclaw.memory.*` tool surface
//! and the retrieve-before / record-after lifecycle; this descriptor only
//! declares which memory operation families the provider backs. Compose-time
//! binding selects exactly one memory provider (native by default), so the
//! model's memory interface stays stable while the backend swaps underneath —
//! it is never installed/removed or swapped at runtime. The concrete provider
//! is the manifest's `[runtime].service`; no connection or credential material
//! lives here (that is compose-time configuration).

use serde::{Deserialize, Serialize};

/// A family of memory operations a provider backs.
///
/// `document_store` is the always-present model tool surface
/// (`read`/`write`/`search`/`tree`); `context_retrieval` and `interaction_log`
/// gate the host-managed retrieve-before-run and record-after-turn lifecycle
/// the host wires for the bound provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOperationKind {
    /// Read/write/search/tree over persistent memory documents.
    DocumentStore,
    /// Two-lane retrieve-before-run memory context.
    ContextRetrieval,
    /// After-turn interaction recording.
    InteractionLog,
}

impl MemoryOperationKind {
    /// Stable wire token.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DocumentStore => "document_store",
            Self::ContextRetrieval => "context_retrieval",
            Self::InteractionLog => "interaction_log",
        }
    }
}

/// The `[memory]` surface of a v3 manifest: the extension is a memory provider
/// (a backend for the host memory adapter) and declares the operation families
/// it backs. Parsing/validation is fail-closed (unknown fields rejected); the
/// operation set must be non-empty and include [`MemoryOperationKind::DocumentStore`]
/// (the tool surface is mandatory) — enforced by the manifest parser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryDescriptor {
    /// The memory operation families this provider backs.
    pub operations: Vec<MemoryOperationKind>,
}

impl MemoryDescriptor {
    /// Whether this provider backs the given operation family.
    pub fn backs(&self, operation: MemoryOperationKind) -> bool {
        self.operations.contains(&operation)
    }
}
