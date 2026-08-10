//! Deferred-tool retrieval port for the agent loop host.
//!
//! This module defines the [`ToolRetrievalProvider`] / [`ToolRetrievalIndex`]
//! pair — the loop-host port that ranks the authorized deferred-tool corpus for
//! a `tool_search` bridge call. The host-bundled provider is a bounded BM25F
//! ranker; this port is what lets a deployment bind a different one (a dense
//! retriever, a hosted ranking service) without the loop host naming it.
//!
//! # Why two traits
//!
//! Ranking is fitted, not stateless. The host rebuilds the corpus only when the
//! authorized surface actually changes (turn boundary, surface version, or
//! definition fingerprint), then serves many `tool_search` calls against that
//! one fitted corpus. Splitting "fit an index" from "search it" keeps that
//! amortization in the port instead of forcing every provider to re-derive
//! per-call state or cache behind an interior mutex.
//!
//! # Authorization contract
//!
//! [`ToolRetrievalProvider::fit`] is only ever handed the **effective
//! authorized** definitions. Providers must not widen that set, and must not
//! let a denied definition influence corpus statistics, ordering, or result
//! counts — the caller relies on denied schemas being unable to affect IDF or
//! rank. A provider that fetches externally must not transmit schemas it was
//! not given.
//!
//! # Determinism contract
//!
//! For one fitted index, the same `(query, limit)` must produce the same
//! [`ToolSearchOutcome`] — including the tie-break order between equal scores.
//! Two runs over an identical corpus must rank identically; the loop host
//! records search rank into turn state and a nondeterministic ranker would make
//! disclosure order irreproducible.
//!
//! # Confidentiality contract
//!
//! Implementations must not log raw queries or schema text. The loop host emits
//! only the query *class*, result count, and latency for exactly this reason.

use std::fmt::Debug;
use std::sync::Arc;

use super::host::ProviderToolDefinition;

/// How a provider classified the query it was given.
///
/// This is reported in host telemetry in place of the raw query, so it must
/// stay coarse enough to be non-identifying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSearchQueryClass {
    /// The query matched a tool's exact identifier (capability id or provider
    /// tool name), so ranking was an identifier lookup rather than retrieval.
    ExactIdentifier,
    /// The query was ranked lexically/semantically against the corpus.
    Lexical,
    /// Nothing matched, or the query carried no usable terms.
    NoMatch,
}

impl ToolSearchQueryClass {
    /// Stable telemetry label. Kept as an explicit match so a new variant has
    /// to choose its label deliberately rather than inherit a derived name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactIdentifier => "exact_identifier",
            Self::Lexical => "lexical",
            Self::NoMatch => "no_match",
        }
    }
}

/// The ranked result of one `tool_search` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSearchOutcome {
    /// Provider tool names, best match first, already truncated to the
    /// requested limit.
    pub names: Vec<String>,
    /// How the query was classified, for telemetry.
    pub query_class: ToolSearchQueryClass,
}

impl ToolSearchOutcome {
    /// The empty outcome — no usable query terms, or nothing matched.
    pub fn no_match() -> Self {
        Self {
            names: Vec::new(),
            query_class: ToolSearchQueryClass::NoMatch,
        }
    }
}

/// An index fitted over one authorized deferred-tool corpus.
///
/// Held by the loop host for as long as the authorized surface is unchanged,
/// and queried once per `tool_search` bridge call.
pub trait ToolRetrievalIndex: Send + Sync + Debug {
    /// Rank the fitted corpus against `query`, returning at most `limit` names.
    ///
    /// `limit` is already clamped by the caller. A `limit` of zero must return
    /// [`ToolSearchOutcome::no_match`] rather than an unbounded result.
    fn search(&self, query: &str, limit: usize) -> ToolSearchOutcome;
}

/// Fits a [`ToolRetrievalIndex`] over the authorized corpus.
///
/// One provider is bound per deployment and reused across turns; `fit` is
/// called on every genuine surface change, so it must be cheap enough to run
/// inside a turn and must not block on network I/O without a bound.
pub trait ToolRetrievalProvider: Send + Sync + Debug {
    /// Stable ranker identifier recorded in host telemetry (for example
    /// `"bounded-bm25f-v1"`). Changing ranking behavior must change this, so a
    /// trace can be attributed to the ranker that produced it.
    fn ranker_version(&self) -> &str;

    /// Fit an index over the **effective authorized** definitions.
    ///
    /// See the module-level authorization contract: the provider must treat
    /// this slice as the complete and only corpus.
    fn fit(&self, definitions: &[ProviderToolDefinition]) -> Arc<dyn ToolRetrievalIndex>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_class_labels_are_stable() {
        assert_eq!(
            ToolSearchQueryClass::ExactIdentifier.as_str(),
            "exact_identifier"
        );
        assert_eq!(ToolSearchQueryClass::Lexical.as_str(), "lexical");
        assert_eq!(ToolSearchQueryClass::NoMatch.as_str(), "no_match");
    }
}
