//! Tool retrieval port for the agent loop host.
//!
//! This module defines the [`ToolRetrievalProvider`] / [`ToolRetrievalIndex`]
//! pair: the loop-host port that ranks a list of authorized tool definitions
//! against a query. The host-bundled provider is a bounded BM25F ranker; this
//! port is what lets a deployment bind a different one (a dense retriever, a
//! hosted ranking service) without the loop host naming it.
//!
//! # Why two traits
//!
//! Ranking is fitted, not stateless. The host fits an index only when the
//! authorized surface actually changes (turn boundary, surface version, or
//! definition fingerprint), then serves many searches against that one fitted
//! index. Splitting "fit an index" from "search it" keeps that amortization in
//! the port instead of forcing every provider to re-derive per-call state or
//! cache behind an interior mutex.
//!
//! # What gets ranked
//!
//! A fitted index ranks exactly the definitions handed to
//! [`ToolRetrievalProvider::fit`], whatever they are. A provider must not
//! assume the corpus is the deferred set, must not skip or down-weight tools
//! because of their names, and must not add tools of its own. The
//! `tool_search` bridge fits over the authorized catalog it searches; a
//! turn-start tool selector fits over the whole authorized catalog, core tools
//! included. Both are ordinary callers of the same port.
//!
//! # Authorization contract
//!
//! [`ToolRetrievalProvider::fit`] is only ever handed the **effective
//! authorized** definitions. Providers must not widen that set, and must not
//! let a denied definition influence corpus statistics, ordering, scores, or
//! result counts: the caller relies on denied schemas being unable to affect
//! IDF or rank. A provider that fetches externally must not transmit schemas it
//! was not given. The host still re-checks every returned name against the
//! fitted corpus and drops anything else, so a provider that breaks this
//! contract cannot widen what the model sees.
//!
//! # Determinism contract
//!
//! For one fitted index, the same `(query, limit)` must produce the same
//! [`ToolSearchOutcome`], including the tie-break order between equal scores,
//! whether it is searched alone or as one of the queries of
//! [`ToolRetrievalIndex::search_many`].
//! Two fits over an identical corpus must rank identically; the loop host
//! records search rank into turn state, and a nondeterministic ranker would
//! make disclosure order irreproducible.
//!
//! # Confidentiality contract
//!
//! Implementations must not log raw queries or schema text, and a
//! [`ToolRetrievalError`] must never carry either. The loop host emits only
//! the query *class*, result count, error kind, and latency for exactly this
//! reason.
//!
//! # Scores
//!
//! Every [`RankedTool`] carries a score. Scores are finite and non-negative,
//! and higher is better. [`ToolSearchOutcome::ranked`] is ordered by score
//! descending, then by the provider's own stable tie-break.
//!
//! The scale belongs to the provider and is identified by
//! [`ToolRetrievalProvider::ranker_version`]. Scores are comparable only
//! within one search, and between searches made by the same `ranker_version`.
//! A relative threshold (a score divided by the top score of the same search)
//! therefore works for any ranker, while an absolute threshold has to be set
//! per ranker version.
//!
//! # Failures
//!
//! `fit` and `search` are fallible and async, so a provider may call a bounded
//! remote or on-device model. A failure is reported as a [`ToolRetrievalError`]
//! and the host turns it into a model-visible tool failure; it never ends the
//! run, and the host never silently swaps in a different ranker.
//!
//! # Corpus owners and background indexing
//!
//! A provider that keeps per-document state (an embedding cache, a durable
//! vector store) may partition it by the user whose authorized corpus it is:
//! [`ToolRetrievalProvider::fit_for_owner`] names that [`ToolCorpusOwner`],
//! and [`ToolRetrievalProvider::index_in_background`] lets the host hand it a
//! user's authorized catalog ahead of any turn, when the catalog changes, so
//! the next fit finds the work done. Both default to the owner-blind
//! behavior, so a stateless ranker ignores them. Everything above applies
//! unchanged: the definitions handed over are the effective authorized set
//! for that owner, and nothing learnt for one owner may be read back for
//! another.

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use ironclaw_host_api::ids::{TenantId, UserId};

use crate::host::{LoopSafeSummary, ProviderToolDefinition};

/// The user whose effective authorized catalog a corpus is.
///
/// Set by the host from the run's trusted scope, never from model or request
/// input. A provider uses it only to partition state it keeps per document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToolCorpusOwner {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl ToolCorpusOwner {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self { tenant_id, user_id }
    }
}

/// How a fitted index came by its document vectors, for telemetry: counts
/// only, never text. A lexical ranker has nothing to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolIndexFitReport {
    /// Documents whose vector already existed (in memory or a durable store).
    pub stored: usize,
    /// Of `stored`, the ones read back from a durable store by this fit.
    pub loaded: usize,
    /// Documents embedded while this fit waited.
    pub embedded: usize,
    /// Documents without a vector in this index when the fit returned. With
    /// `dense_fallback` set the index holds no dense vectors, so this is
    /// the whole corpus.
    pub missing: usize,
    /// Set by a fusing ranker whose dense side is absent from this index:
    /// why (`timeout`, an error kind, or `absent`). `None` when it is used.
    pub dense_fallback: Option<&'static str>,
}

/// How a provider classified the query it was given.
///
/// This is reported in host telemetry in place of the raw query, so it must
/// stay coarse enough to be non-identifying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSearchQueryClass {
    /// The query matched a tool's exact identifier (capability id or provider
    /// tool name), so that tool was ranked first as an identifier lookup.
    ExactIdentifier,
    /// The query was ranked against the corpus (lexically, semantically, or
    /// both, as the provider chooses).
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

/// One ranked tool: the provider tool name and the provider's score for it.
///
/// See the module docs for what a score means and when two scores may be
/// compared.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedTool {
    /// Provider tool name of a definition from the fitted corpus.
    pub name: String,
    /// Finite, non-negative relevance score; higher is better. The scale is
    /// the provider's, identified by its `ranker_version`.
    pub score: f32,
}

impl RankedTool {
    pub fn new(name: impl Into<String>, score: f32) -> Self {
        Self {
            name: name.into(),
            score,
        }
    }
}

/// The ranked result of one search.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSearchOutcome {
    /// Ranked tools, best first: score descending, then the provider's stable
    /// tie-break. At most the requested limit.
    pub ranked: Vec<RankedTool>,
    /// How the query was classified, for telemetry.
    pub query_class: ToolSearchQueryClass,
}

impl ToolSearchOutcome {
    /// The empty outcome: no usable query terms, or nothing matched.
    pub fn no_match() -> Self {
        Self {
            ranked: Vec::new(),
            query_class: ToolSearchQueryClass::NoMatch,
        }
    }

    /// The ranked tool names, best first.
    pub fn names(&self) -> Vec<&str> {
        self.ranked.iter().map(|tool| tool.name.as_str()).collect()
    }

    /// The ranked tool names, best first, consuming the outcome.
    pub fn into_names(self) -> Vec<String> {
        self.ranked.into_iter().map(|tool| tool.name).collect()
    }
}

/// Why a fit or a search failed.
///
/// Every variant is safe to record: none carries the query text or any
/// schema text, and a provider must not smuggle either into a `reason`.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ToolRetrievalError {
    /// The ranking backend (a model, a service, a local file) could not be
    /// reached or is not ready.
    #[error("tool retrieval is unavailable: {reason}")]
    Unavailable { reason: LoopSafeSummary },
    /// The provider gave up after its own time bound.
    #[error("tool retrieval timed out after {elapsed:?}")]
    Timeout { elapsed: Duration },
    /// The backend answered, but with output the provider could not turn into
    /// a ranking (malformed response, wrong dimensions, and so on).
    #[error("tool retrieval returned invalid output: {reason}")]
    InvalidOutput { reason: LoopSafeSummary },
    /// The corpus handed to `fit` is larger than the provider supports.
    #[error("tool retrieval corpus of {definitions} definitions exceeds the limit of {limit}")]
    CorpusTooLarge { definitions: usize, limit: usize },
}

impl ToolRetrievalError {
    /// Stable telemetry label for the failure kind. Hosts log this instead of
    /// the error's display text.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unavailable",
            Self::Timeout { .. } => "timeout",
            Self::InvalidOutput { .. } => "invalid_output",
            Self::CorpusTooLarge { .. } => "corpus_too_large",
        }
    }
}

/// An index fitted over one authorized list of tool definitions.
///
/// Held by the loop host for as long as the authorized surface is unchanged,
/// and searched once per query.
#[async_trait]
pub trait ToolRetrievalIndex: Send + Sync + Debug {
    /// Rank the fitted corpus against `query`, returning at most `limit`
    /// tools.
    ///
    /// `limit` is already clamped by the caller. A `limit` of zero must return
    /// [`ToolSearchOutcome::no_match`] rather than an unbounded result.
    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError>;

    /// Rank the fitted corpus against each of `queries` separately: one
    /// outcome per query, in query order, each exactly what
    /// [`Self::search`] returns for that query and `limit`.
    ///
    /// This is not a new kind of query: every query is ranked on its own and
    /// the outcomes are never combined here (combining them is the caller's
    /// job). It exists so a provider whose backend answers several queries
    /// at once (an embedding endpoint takes a batch) can make one request
    /// instead of one per query. The default searches the queries one after
    /// another. Any failure fails the whole call.
    async fn search_many(
        &self,
        queries: &[&str],
        limit: usize,
    ) -> Result<Vec<ToolSearchOutcome>, ToolRetrievalError> {
        let mut outcomes = Vec::with_capacity(queries.len());
        for query in queries {
            outcomes.push(self.search(query, limit).await?);
        }
        Ok(outcomes)
    }

    /// How this index came by its vectors, when the ranker has any.
    fn fit_report(&self) -> Option<ToolIndexFitReport> {
        None
    }
}

/// Fits a [`ToolRetrievalIndex`] over a list of authorized definitions.
///
/// One provider is bound per deployment and reused across turns; `fit` is
/// called on every genuine surface change, so it must be cheap enough to run
/// inside a turn and must bound any network or model I/O it performs.
#[async_trait]
pub trait ToolRetrievalProvider: Send + Sync + Debug {
    /// Stable ranker identifier (for example `"bounded-bm25f-v1"`). It names
    /// the score scale and keys the host's fitted-index cache, so changing
    /// ranking behavior or scale must change it.
    fn ranker_version(&self) -> &str;

    /// Fit an index over the **effective authorized** definitions.
    ///
    /// See the module-level contracts: the provider must treat this slice as
    /// the complete and only corpus, and rank every entry in it on equal
    /// terms.
    async fn fit(
        &self,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError>;

    /// [`Self::fit`] for `owner`'s authorized catalog, so per-document state
    /// can be kept per owner. Must rank exactly as [`Self::fit`] would.
    async fn fit_for_owner(
        &self,
        owner: &ToolCorpusOwner,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        let _ = owner;
        self.fit(definitions).await
    }

    /// Prepare for a later fit over `owner`'s authorized `definitions`
    /// without waiting for it: the host calls this off the turn path when
    /// the catalog changes. It must return promptly and bound whatever work
    /// it starts; a ranker with nothing to prepare ignores it.
    fn index_in_background(&self, owner: &ToolCorpusOwner, definitions: &[ProviderToolDefinition]) {
        let _ = (owner, definitions);
    }
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

    #[test]
    fn error_kind_labels_are_stable_and_carry_no_detail() {
        let reason = LoopSafeSummary::new("backend offline").expect("valid summary");
        assert_eq!(
            ToolRetrievalError::Unavailable {
                reason: reason.clone()
            }
            .kind_label(),
            "unavailable"
        );
        assert_eq!(
            ToolRetrievalError::Timeout {
                elapsed: Duration::from_millis(5)
            }
            .kind_label(),
            "timeout"
        );
        assert_eq!(
            ToolRetrievalError::InvalidOutput { reason }.kind_label(),
            "invalid_output"
        );
        assert_eq!(
            ToolRetrievalError::CorpusTooLarge {
                definitions: 2,
                limit: 1
            }
            .kind_label(),
            "corpus_too_large"
        );
    }

    /// Ranks every query as one tool named after it; fails on `"fail"`.
    #[derive(Debug)]
    struct EchoIndex;

    #[async_trait]
    impl ToolRetrievalIndex for EchoIndex {
        async fn search(
            &self,
            query: &str,
            limit: usize,
        ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
            if query == "fail" {
                return Err(ToolRetrievalError::Timeout {
                    elapsed: Duration::from_millis(1),
                });
            }
            Ok(ToolSearchOutcome {
                ranked: vec![RankedTool::new(query, limit as f32)],
                query_class: ToolSearchQueryClass::Lexical,
            })
        }
    }

    #[tokio::test]
    async fn search_many_defaults_to_one_search_per_query_in_order() {
        let outcomes = EchoIndex
            .search_many(&["b", "a"], 3)
            .await
            .expect("searched");
        assert_eq!(
            outcomes
                .iter()
                .map(|outcome| outcome.ranked.clone())
                .collect::<Vec<_>>(),
            vec![
                vec![RankedTool::new("b", 3.0)],
                vec![RankedTool::new("a", 3.0)]
            ]
        );
        assert!(
            EchoIndex
                .search_many(&[], 3)
                .await
                .expect("nothing to search")
                .is_empty()
        );
        assert_eq!(
            EchoIndex
                .search_many(&["a", "fail"], 3)
                .await
                .map_err(|error| error.kind_label()),
            Err("timeout")
        );
    }

    #[test]
    fn outcome_names_follow_rank_order() {
        let outcome = ToolSearchOutcome {
            ranked: vec![RankedTool::new("b", 2.0), RankedTool::new("a", 1.0)],
            query_class: ToolSearchQueryClass::Lexical,
        };
        assert_eq!(outcome.names(), vec!["b", "a"]);
        assert_eq!(outcome.into_names(), vec!["b".to_string(), "a".to_string()]);
        assert!(ToolSearchOutcome::no_match().ranked.is_empty());
    }
}
