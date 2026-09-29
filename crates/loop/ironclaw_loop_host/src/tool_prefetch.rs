//! Turn-start tool selection: advertise only the tools a conversation's
//! opening request predicts, frozen while the provider's prompt cache could
//! be warm.
//!
//! With progressive disclosure alone, every request advertises the core tools
//! plus the discovery bridges, and any other tool costs a `tool_search` round
//! trip before its first use. Turn-start selection ranks the whole authorized
//! catalog (core tools included) against the conversation's opening request
//! and advertises the best-ranked tools up to `max_tools`, plus a small
//! always-on floor. Everything else is deferred and stays reachable through
//! `tool_search` → `tool_call`.
//!
//! # Why the list is frozen
//!
//! The request's `tools` array is part of the provider's cached prompt
//! prefix: changing it re-bills the whole prompt. So a conversation selects
//! once, records the result as the `initial` entry of its append-only
//! selection history (`ironclaw_threads::ToolSelectionHistory`), and every
//! later call rebuilds the byte-identical array from the entry in force,
//! never re-ranking, while the cache could be warm. Nothing is promoted.
//!
//! # Re-selection
//!
//! Once the cache has expired, changing the array costs nothing, so with
//! re-selection on ([`ToolReselectionConfig`]) the host chooses again from
//! the conversation so far at the next turn boundary, before the turn's
//! first model call and never mid-turn. It does so only when:
//!
//! - the conversation has been idle since its last model call for longer
//!   than the provider's cache lifetime plus a margin (`cache_cold`). The
//!   lifetime comes from the provider when it is known (Anthropic's cache
//!   retention; no cache at all means every turn boundary qualifies) and from
//!   the configured lifetime otherwise;
//! - the model or provider changed since that call (`model_change`), since
//!   caches are per model; or
//! - a selected tool was revoked (`revoked`): the array changes anyway, so
//!   the host re-selects instead of only removing it. If that re-selection
//!   fails, the revoked tools are still removed.
//!
//! Nothing else re-selects; in particular compaction does not, because it
//! rewrites only the messages, which follow the tools in the cached prefix.
//! Guessing "cold" wrongly costs a full-price prompt, guessing "warm" wrongly
//! only a missed re-selection, so every unknown leans towards warm: no
//! recorded model call, an unreadable record, or an unknown model never
//! re-selects. The time and model of the last call come from the
//! conversation's durable `ToolSelectionActivity`, written after each model
//! call, so a restart changes neither.
//!
//! A re-selection ranks against a bounded window of the conversation's user
//! messages (the most recent `context_messages`, plus the opening message if
//! it fits) and keeps every tool the conversation has called successfully
//! ("sticky" tools, counted first against `max_tools`), subject to
//! authorization and availability. When the result equals the list in force
//! nothing is appended; when the classifier fails the list in force is kept.
//!
//! # Availability
//!
//! A slot in the frozen list is wasted on a tool the acting user cannot use,
//! so when composition supplies a `ToolAvailabilityPredicate`
//! (`ironclaw_loop_contracts`), only authorized tools it reports as available
//! are ranked; a failed or timed-out lookup excludes the tool (fail closed).
//! Revocation is stricter: a frozen tool is revoked only on a definite
//! "unavailable" answer, never on a failed lookup, because a revoked tool is
//! never added back. A tool left out stays callable through `tool_search` →
//! `tool_call`, which is where its extension's setup prompt appears.
//!
//! # One ranking per segment, merged by rank
//!
//! Rankers read only the start of a query: BM25F reads a bounded number of
//! terms, and an embedding model a few hundred tokens. One query joined from
//! the whole conversation would therefore see only its newest topic, or
//! whichever topic has the most words. So the local classifier cuts the
//! conversation into segments (`conversation::segments`: each user
//! message, newest first, and a long message cut at paragraph or sentence
//! boundaries into pieces of at most [`ToolPrefetchConfig::segment_bytes`],
//! at most [`ToolPrefetchConfig::context_messages`] segments in all), ranks
//! each segment separately against the same fitted index (one
//! `search_many` call, so a dense ranker embeds every segment in one
//! request), and merges the rankings round-robin by rank: every segment's
//! first tool, then every segment's second, and so on, each tool once, until
//! `max_tools` or the token budget is reached.
//!
//! Round-robin was chosen over reciprocal-rank fusion across segments
//! because fusion adds up a tool's reciprocal ranks, so a tool ranked fifth
//! by three segments of one topic outranks the first tool of a topic that
//! fills one segment; with a tight `max_tools` the minority topic would get
//! nothing. Round-robin guarantees each segment its top tools first, whatever
//! the other segments say. Ties within one rank go to the newer segment.
//!
//! BM25F ranks only tools that share a term with a segment, so lexical
//! selection may fill fewer than `max_tools` slots; it never pads with
//! unrelated tools.
//!
//! # Thresholds
//!
//! Selection fills `max_tools` by rank; score thresholds are optional knobs,
//! off (0) by default, because a threshold does not calibrate across score
//! scales. (Scores are only comparable within one ranker's scale, see
//! `ironclaw_loop_contracts::tool_retrieval`: the reciprocal-rank-fusion
//! scale's former relative default admitted 87 tools at `max_tools` 100 in
//! the results run, BM25F's 15.) When set, both apply per segment:
//!
//! - [`ToolPrefetchConfig::min_similarity`], an absolute threshold, only to
//!   cosine scores (`dense-cosine-*` rankers); BM25F and reciprocal-rank
//!   fusion scores ignore it.
//! - [`ToolPrefetchConfig::min_relative`], a score divided by the top score
//!   of the same segment's ranking, to every ranker.
//!
//! The token budget stays as a guard against a few huge schemas: the merge
//! stops at the first tool that does not fit it.
//!
//! # Classifiers
//!
//! Choosing from the candidates is the `ToolSelectionClassifier` port's job
//! (`ironclaw_loop_contracts`). The ranker and merge above are the
//! host-bundled classifier, `RankingToolClassifier`; a deployment may bind one
//! other classifier instead ([`ToolPrefetchConfig::with_classifier`]), and
//! then the ranker never runs. Either way the host owns everything around
//! the choice: the candidates, the floor, the checks on the answer (unknown,
//! duplicate or pinned names and invalid scores are dropped, and the caps are
//! enforced), and the history record.
//!
//! When the local classifier fails, the run keeps the ordinary disclosure
//! surface and nothing is recorded, so a later run selects instead. When a
//! bound classifier fails at a conversation's first selection, the host
//! freezes the core tool set instead (the core tools the user is authorized
//! for, plus the floor and extras: what the conversation would advertise
//! without selection) and records it with the failure's label, so the
//! conversation keeps it even if the classifier recovers later.
//!
//! # Confidentiality
//!
//! Every user message is cut to `MAX_CONTEXT_MESSAGE_BYTES` and a
//! re-selection's window to the configured message count and
//! `MAX_CONVERSATION_CONTEXT_BYTES`; no message or segment is ever logged
//! (only counts, tool names, ranks and scores), and [`ConversationContext`]'s
//! `Debug` prints only its size. Selection logs at `debug!` on
//! [`TOOL_PREFETCH_LOG_TARGET`] only.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{capability_surface::CapabilitySurfacePolicy, ids::CapabilityId};
use ironclaw_loop_contracts::{
    ChosenTool, ConversationContext, LoopRunContext, ProviderToolDefinition,
    TOOL_AVAILABILITY_LOOKUP_TIMEOUT, ToolAvailability, ToolAvailabilityPredicate,
    ToolRetrievalProvider, ToolSelectionCandidate, ToolSelectionClassifier, ToolSelectionError,
    ToolSelectionRequest,
};
use ironclaw_threads::{
    AppendToolSelectionEntryRequest, MessageKind, SessionThreadError, SessionThreadService,
    ThreadScope, ToolSelectionEffectiveFrom, ToolSelectionEntry, ToolSelectionHistory,
    ToolSelectionReason, ToolSelectionScore,
};
use tracing::debug;

use crate::{
    HostManagedModelGateway, HostManagedPromptCacheProfile, ThreadScopeResolver,
    ToolDisclosureMode, accepted_task_message_id,
    tool_disclosure::{
        ActiveSet, CapabilityCatalog, TOOL_CALL_NAME, TOOL_DESCRIBE_NAME, TOOL_SEARCH_NAME,
        advertised_bridge_tokens, frozen_active_set, is_core_tool_definition,
        tool_search_description_excluding,
    },
};

mod conversation;
mod local_classifier;
mod reselect;

pub use conversation::{
    MAX_CONTEXT_MESSAGES, MAX_CONTEXT_SEGMENT_BYTES, MIN_CONTEXT_SEGMENT_BYTES,
};

use local_classifier::{
    LOCAL_CLASSIFIER_NAME, LexicalSelectionRetrieval, RankingToolClassifier, SelectionFloor,
    advertised_names, selection_floor,
};

/// `tracing` target for every selection log line.
pub(crate) const TOOL_PREFETCH_LOG_TARGET: &str = "ironclaw::reborn::tool_prefetch";

/// Tools every selected surface advertises when they are authorized: the
/// three discovery bridges, which reach everything that is not advertised,
/// and `result_read`, without which a large tool result cannot be read.
pub const TOOL_PREFETCH_MANDATORY_FLOOR: [&str; 4] = [
    TOOL_SEARCH_NAME,
    TOOL_DESCRIBE_NAME,
    TOOL_CALL_NAME,
    "result_read",
];

/// Which ranker turn-start selection uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPrefetchRanking {
    /// The host-bundled BM25F ranker, scoring every term match.
    Lexical,
    /// The ranker bound behind `tool_search` (dense or hybrid).
    Semantic,
}

impl ToolPrefetchRanking {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Semantic => "semantic",
        }
    }
}

/// Default [`ToolPrefetchConfig::context_messages`]; the operator-facing
/// default lives with the setting's parser (`ironclaw_config`) and matches.
const DEFAULT_CONTEXT_MESSAGES: usize = 16;

/// Default [`ToolPrefetchConfig::segment_bytes`]: about 512 tokens of prose,
/// what a small embedding model such as `bge-small` reads of one input.
const DEFAULT_SEGMENT_BYTES: usize = 2_048;

/// Validated turn-start selection settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolPrefetchConfig {
    ranking: ToolPrefetchRanking,
    max_tools: usize,
    token_budget: u32,
    min_similarity: f32,
    min_relative: f32,
    always: Vec<String>,
    /// Most user messages a selection reads, and most segments the local
    /// classifier ranks.
    context_messages: usize,
    /// Most bytes of one segment the local classifier ranks.
    segment_bytes: usize,
    /// A deployment-bound classifier replacing the local one. `None` keeps
    /// the ranker and merge.
    classifier: Option<BoundClassifier>,
    reselection: ToolReselectionConfig,
}

/// When a conversation chooses its tools again after its first selection
/// (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReselectionConfig {
    enabled: bool,
    cache_lifetime: Duration,
    cache_margin: Duration,
}

impl ToolReselectionConfig {
    /// The operator-facing defaults live with the setting's parser
    /// (`ironclaw_config`). `cache_lifetime` applies to providers whose cache
    /// lifetime the host cannot know; `cache_margin` is added to every
    /// lifetime. How much of the conversation a re-selection reads is
    /// [`ToolPrefetchConfig::context_messages`].
    pub fn new(enabled: bool, cache_lifetime: Duration, cache_margin: Duration) -> Self {
        Self {
            enabled,
            cache_lifetime,
            cache_margin,
        }
    }

    /// Never re-select: the first selection stays for the whole
    /// conversation, narrowed only by revocations.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            cache_lifetime: Duration::from_secs(3_600),
            cache_margin: Duration::from_secs(60),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Lifetime assumed for a provider whose cache lifetime is unknown.
    pub fn cache_lifetime(&self) -> Duration {
        self.cache_lifetime
    }

    pub fn cache_margin(&self) -> Duration {
        self.cache_margin
    }
}

/// A deployment-bound [`ToolSelectionClassifier`], compared by identity.
#[derive(Clone)]
struct BoundClassifier(Arc<dyn ToolSelectionClassifier>);

impl fmt::Debug for BoundClassifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BoundClassifier")
            .field(&self.0.classifier_name())
            .finish()
    }
}

impl PartialEq for BoundClassifier {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

/// Why a [`ToolPrefetchConfig`] or its binding was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ToolPrefetchConfigError {
    #[error(
        "the always-on tool floor ({floor} tools: the {mandatory} mandatory tools plus the \
         configured extras) exceeds the maximum of {max_tools} advertised tools; raise the \
         maximum or list fewer extras"
    )]
    FloorExceedsMaxTools {
        floor: usize,
        mandatory: usize,
        max_tools: usize,
    },
    #[error("{name} must be a number between 0 and 1, got {value}")]
    ThresholdOutOfRange { name: &'static str, value: f32 },
    #[error(
        "semantic tool selection needs a dense or hybrid tool ranker, but only the native \
         BM25F ranker is bound"
    )]
    SemanticNeedsBoundRanker,
    #[error("the configured tool selection classifier `{name}` was not bound")]
    ClassifierNotBound { name: String },
    #[error("the tool selection context must hold between 1 and {max} messages, got {value}")]
    ContextMessagesOutOfRange { value: usize, max: usize },
    #[error("the tool selection segment size must be between {min} and {max} bytes, got {value}")]
    SegmentBytesOutOfRange {
        value: usize,
        min: usize,
        max: usize,
    },
}

impl ToolPrefetchConfig {
    /// Validate settings. The operator-facing defaults live with the
    /// setting's parser (`ironclaw_config`); this type only checks them.
    /// Both thresholds are off at 0 (the default); see the module docs.
    /// `always` names extra floor tools (provider names or capability ids);
    /// duplicates and names of mandatory tools count once. The context
    /// bounds start at their defaults ([`Self::with_context`]).
    pub fn new(
        ranking: ToolPrefetchRanking,
        max_tools: usize,
        token_budget: u32,
        min_similarity: f32,
        min_relative: f32,
        always: Vec<String>,
    ) -> Result<Self, ToolPrefetchConfigError> {
        check_unit_interval("min_similarity", min_similarity)?;
        check_unit_interval("min_relative", min_relative)?;
        let mut extras: Vec<String> = Vec::new();
        for name in always {
            let name = name.trim().to_string();
            if !name.is_empty() && !extras.contains(&name) {
                extras.push(name);
            }
        }
        let floor = TOOL_PREFETCH_MANDATORY_FLOOR.len()
            + extras
                .iter()
                .filter(|name| !TOOL_PREFETCH_MANDATORY_FLOOR.contains(&name.as_str()))
                .count();
        if floor > max_tools {
            return Err(ToolPrefetchConfigError::FloorExceedsMaxTools {
                floor,
                mandatory: TOOL_PREFETCH_MANDATORY_FLOOR.len(),
                max_tools,
            });
        }
        Ok(Self {
            ranking,
            max_tools,
            token_budget,
            min_similarity,
            min_relative,
            always: extras,
            context_messages: DEFAULT_CONTEXT_MESSAGES,
            segment_bytes: DEFAULT_SEGMENT_BYTES,
            classifier: None,
            reselection: ToolReselectionConfig::disabled(),
        })
    }

    /// Bound how much of the conversation a selection reads: at most
    /// `messages` user messages (a re-selection's window), cut into at most
    /// `messages` segments of at most `segment_bytes` each for the local
    /// classifier, which ranks every segment separately. `messages` must be
    /// between 1 and [`MAX_CONTEXT_MESSAGES`], and `segment_bytes` between
    /// [`MIN_CONTEXT_SEGMENT_BYTES`] and [`MAX_CONTEXT_SEGMENT_BYTES`].
    pub fn with_context(
        mut self,
        messages: usize,
        segment_bytes: usize,
    ) -> Result<Self, ToolPrefetchConfigError> {
        if !(1..=MAX_CONTEXT_MESSAGES).contains(&messages) {
            return Err(ToolPrefetchConfigError::ContextMessagesOutOfRange {
                value: messages,
                max: MAX_CONTEXT_MESSAGES,
            });
        }
        if !(MIN_CONTEXT_SEGMENT_BYTES..=MAX_CONTEXT_SEGMENT_BYTES).contains(&segment_bytes) {
            return Err(ToolPrefetchConfigError::SegmentBytesOutOfRange {
                value: segment_bytes,
                min: MIN_CONTEXT_SEGMENT_BYTES,
                max: MAX_CONTEXT_SEGMENT_BYTES,
            });
        }
        self.context_messages = messages;
        self.segment_bytes = segment_bytes;
        Ok(self)
    }

    /// Re-select as `reselection` says. Without this call a conversation
    /// never re-selects.
    pub fn with_reselection(mut self, reselection: ToolReselectionConfig) -> Self {
        self.reselection = reselection;
        self
    }

    pub fn reselection(&self) -> &ToolReselectionConfig {
        &self.reselection
    }

    /// Choose tools with `classifier` instead of the local ranker and
    /// merge, which then never run (the ranking and thresholds are
    /// ignored). A failure of `classifier` at a conversation's first
    /// selection freezes the core tool set; see the module docs.
    pub fn with_classifier(mut self, classifier: Arc<dyn ToolSelectionClassifier>) -> Self {
        self.classifier = Some(BoundClassifier(classifier));
        self
    }

    /// Name of the classifier that chooses: `local`, or the bound one's.
    pub fn classifier_name(&self) -> &str {
        self.classifier
            .as_ref()
            .map_or(LOCAL_CLASSIFIER_NAME, |bound| bound.0.classifier_name())
    }

    pub fn ranking(&self) -> ToolPrefetchRanking {
        self.ranking
    }

    /// Most tools the `tools` array may hold, floor and extras included.
    pub fn max_tools(&self) -> usize {
        self.max_tools
    }

    /// Most estimated schema tokens the advertised tools may add up to.
    pub fn token_budget(&self) -> u32 {
        self.token_budget
    }

    /// Absolute cosine threshold, 0 (off) by default; ignored by
    /// non-cosine rankers.
    pub fn min_similarity(&self) -> f32 {
        self.min_similarity
    }

    /// Relative threshold, 0 (off) by default.
    pub fn min_relative(&self) -> f32 {
        self.min_relative
    }

    /// Most user messages a selection reads, and most segments the local
    /// classifier ranks.
    pub fn context_messages(&self) -> usize {
        self.context_messages
    }

    /// Most bytes of one segment the local classifier ranks.
    pub fn segment_bytes(&self) -> usize {
        self.segment_bytes
    }
}

fn check_unit_interval(name: &'static str, value: f32) -> Result<(), ToolPrefetchConfigError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ToolPrefetchConfigError::ThresholdOutOfRange { name, value })
    }
}

/// The availability step between the authorized catalog and the ranker.
///
/// Candidates are the whole authorized catalog; this step sorts them by
/// whether the acting user can use them right now. Only tools known to be
/// available are admitted to a new selection. The same step decides when a
/// frozen tool counts as revoked, and there only a definite "unavailable"
/// answer counts: a tool whose availability is unknown (the lookup failed or
/// timed out) stays in the frozen selection, because a revoked tool is never
/// added back and one slow lookup must not remove it for good.
#[async_trait]
pub(crate) trait ToolPrefetchCandidateFilter: Send + Sync + fmt::Debug {
    /// Sort `candidates` for this run.
    async fn availability(
        &self,
        run_context: &LoopRunContext,
        candidates: &[&ProviderToolDefinition],
    ) -> CandidateAvailability;
}

/// Candidate names sorted by availability; every candidate is in exactly
/// one set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CandidateAvailability {
    /// Known to be usable: admitted to a new selection.
    pub(crate) available: BTreeSet<String>,
    /// Not known either way: not admitted, but not revoked either.
    pub(crate) unknown: BTreeSet<String>,
    /// Known to be unusable: not admitted, and revoked when frozen.
    pub(crate) unavailable: BTreeSet<String>,
}

impl CandidateAvailability {
    /// Every candidate admitted.
    fn all_available(candidates: &[&ProviderToolDefinition]) -> Self {
        Self {
            available: candidates
                .iter()
                .map(|definition| definition.name.to_string())
                .collect(),
            ..Self::default()
        }
    }

    /// The names a frozen selection keeps: everything not definitely
    /// unavailable.
    fn retained(&self) -> BTreeSet<String> {
        self.available.union(&self.unknown).cloned().collect()
    }
}

/// The default [`ToolPrefetchCandidateFilter`]: every authorized tool is
/// available.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PassThroughCandidateFilter;

#[async_trait]
impl ToolPrefetchCandidateFilter for PassThroughCandidateFilter {
    async fn availability(
        &self,
        _run_context: &LoopRunContext,
        candidates: &[&ProviderToolDefinition],
    ) -> CandidateAvailability {
        CandidateAvailability::all_available(candidates)
    }
}

/// How long the whole availability call may take. The predicate bounds each
/// of its lookups by [`TOOL_AVAILABILITY_LOOKUP_TIMEOUT`]; this backstop only
/// fires for a predicate that breaks that contract, and then every candidate
/// counts as unknown.
const AVAILABILITY_CALL_BACKSTOP: Duration =
    TOOL_AVAILABILITY_LOOKUP_TIMEOUT.saturating_add(Duration::from_millis(200));

/// The [`ToolPrefetchCandidateFilter`] backed by the composition-supplied
/// [`ToolAvailabilityPredicate`].
#[derive(Debug, Clone)]
pub(crate) struct PredicateCandidateFilter {
    predicate: Arc<dyn ToolAvailabilityPredicate>,
    backstop: Duration,
}

impl PredicateCandidateFilter {
    pub(crate) fn new(predicate: Arc<dyn ToolAvailabilityPredicate>) -> Self {
        Self {
            predicate,
            backstop: AVAILABILITY_CALL_BACKSTOP,
        }
    }
}

#[async_trait]
impl ToolPrefetchCandidateFilter for PredicateCandidateFilter {
    async fn availability(
        &self,
        run_context: &LoopRunContext,
        candidates: &[&ProviderToolDefinition],
    ) -> CandidateAvailability {
        let capability_ids: Vec<CapabilityId> = candidates
            .iter()
            .map(|definition| definition.capability_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let answers = match tokio::time::timeout(
            self.backstop,
            self.predicate.availability(run_context, &capability_ids),
        )
        .await
        {
            Ok(answers) => answers,
            Err(_) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    candidates = candidates.len(),
                    "tool availability lookup overran its deadline; every candidate counts as unknown"
                );
                BTreeMap::new()
            }
        };
        let mut sorted = CandidateAvailability::default();
        let mut unavailable_reasons: Vec<(&str, &'static str)> = Vec::new();
        for definition in candidates {
            let name = definition.name.to_string();
            match answers
                .get(&definition.capability_id)
                .copied()
                .unwrap_or(ToolAvailability::Unknown)
            {
                ToolAvailability::Available => {
                    sorted.available.insert(name);
                }
                ToolAvailability::Unknown => {
                    sorted.unknown.insert(name);
                }
                ToolAvailability::Unavailable(reason) => {
                    unavailable_reasons.push((definition.name.as_str(), reason.as_str()));
                    sorted.unavailable.insert(name);
                }
            }
        }
        if !sorted.unknown.is_empty() || !sorted.unavailable.is_empty() {
            debug!(
                target: TOOL_PREFETCH_LOG_TARGET,
                available_count = sorted.available.len(),
                unknown = ?sorted.unknown,
                unavailable = ?unavailable_reasons,
                "tool availability narrowed the selection candidates"
            );
        }
        sorted
    }
}

/// Tells turn-start selection which model a run's first call will use and
/// how long that provider keeps the prompt cached.
pub trait PromptCacheProfileSource: Send + Sync {
    fn prompt_cache_profile(&self, run_context: &LoopRunContext) -> HostManagedPromptCacheProfile;
}

/// [`PromptCacheProfileSource`] over the runtime's host-managed model
/// gateway: the profile of the run's model profile on its primary route.
pub struct GatewayPromptCacheProfiles<G: ?Sized> {
    gateway: Arc<G>,
}

impl<G: ?Sized> GatewayPromptCacheProfiles<G> {
    pub fn new(gateway: Arc<G>) -> Self {
        Self { gateway }
    }
}

impl<G> PromptCacheProfileSource for GatewayPromptCacheProfiles<G>
where
    G: HostManagedModelGateway + ?Sized,
{
    fn prompt_cache_profile(&self, run_context: &LoopRunContext) -> HostManagedPromptCacheProfile {
        let model_profile_id = &run_context.resolved_run_profile.model_profile_id;
        let route = run_context.resolved_model_route.as_ref();
        match self.gateway.resolve_for_scope(&run_context.scope) {
            Some(gateway) => gateway.prompt_cache_profile(model_profile_id, 0, route),
            None => self
                .gateway
                .prompt_cache_profile(model_profile_id, 0, route),
        }
    }
}

/// Turn-start selection bound to one runtime: the settings, the ranker, and
/// the thread store that holds each conversation's selection history.
#[derive(Clone)]
pub(crate) struct ToolPrefetch {
    config: ToolPrefetchConfig,
    classifier: SelectionClassifier,
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
    candidate_filter: Arc<dyn ToolPrefetchCandidateFilter>,
    /// The model and cache lifetime of a run's calls; `None` knows neither,
    /// so only the configured lifetime applies and a model change is never
    /// detected.
    cache_profiles: Option<Arc<dyn PromptCacheProfileSource>>,
}

impl fmt::Debug for ToolPrefetch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolPrefetch")
            .field("config", &self.config)
            .field("classifier", &self.classifier.get().classifier_name())
            .finish_non_exhaustive()
    }
}

/// The one classifier a selection calls, and what its failure means.
#[derive(Clone)]
enum SelectionClassifier {
    /// The ranker and merge. A failure keeps the ordinary surface and
    /// records nothing.
    Local(Arc<RankingToolClassifier>),
    /// A deployment-bound classifier. A failure at a conversation's first
    /// selection freezes the core tool set.
    Bound(Arc<dyn ToolSelectionClassifier>),
}

impl SelectionClassifier {
    fn get(&self) -> &dyn ToolSelectionClassifier {
        match self {
            Self::Local(local) => local.as_ref(),
            Self::Bound(bound) => bound.as_ref(),
        }
    }
}

/// Everything [`ToolPrefetch::active_set`] needs from the disclosure port.
pub(crate) struct PrefetchSurface<'a> {
    pub(crate) run_context: &'a LoopRunContext,
    pub(crate) catalog: &'a CapabilityCatalog,
    pub(crate) policy: &'a CapabilitySurfacePolicy,
    pub(crate) mode: ToolDisclosureMode,
}

impl ToolPrefetch {
    /// Bind `config`. With a classifier bound in `config`, it alone
    /// chooses and `bound_ranker` is unused. Otherwise lexical selection
    /// ranks with BM25F and semantic selection ranks with `bound_ranker`,
    /// which must not be the native BM25F ranker.
    pub(crate) fn new(
        config: ToolPrefetchConfig,
        bound_ranker: Arc<dyn ToolRetrievalProvider>,
        thread_service: Arc<dyn SessionThreadService>,
        thread_scope: ThreadScope,
    ) -> Result<Self, ToolPrefetchConfigError> {
        let classifier = match &config.classifier {
            Some(BoundClassifier(bound)) => SelectionClassifier::Bound(Arc::clone(bound)),
            None => {
                let ranker: Arc<dyn ToolRetrievalProvider> = match config.ranking {
                    ToolPrefetchRanking::Lexical => Arc::new(LexicalSelectionRetrieval),
                    ToolPrefetchRanking::Semantic => {
                        if bound_ranker.ranker_version()
                            == crate::tool_search::NativeBm25fToolRetrieval.ranker_version()
                        {
                            return Err(ToolPrefetchConfigError::SemanticNeedsBoundRanker);
                        }
                        bound_ranker
                    }
                };
                SelectionClassifier::Local(Arc::new(RankingToolClassifier::new(
                    config.clone(),
                    ranker,
                )))
            }
        };
        Ok(Self {
            config,
            classifier,
            thread_service,
            thread_scope,
            candidate_filter: Arc::new(PassThroughCandidateFilter),
            cache_profiles: None,
        })
    }

    /// Learn each run's model and prompt-cache lifetime from `profiles`.
    pub(crate) fn with_prompt_cache_profiles(
        mut self,
        profiles: Arc<dyn PromptCacheProfileSource>,
    ) -> Self {
        self.cache_profiles = Some(profiles);
        self
    }

    /// Filter candidates by `predicate` instead of admitting every
    /// authorized tool.
    pub(crate) fn with_availability(
        mut self,
        predicate: Arc<dyn ToolAvailabilityPredicate>,
    ) -> Self {
        self.candidate_filter = Arc::new(PredicateCandidateFilter::new(predicate));
        self
    }

    /// The conversation's frozen `tools` array for this run, selecting it
    /// first when the conversation has none yet.
    ///
    /// `None` means turn-start selection does not apply to this run and the
    /// caller keeps the ordinary disclosure surface: the run has no accepted
    /// user message with text to select from, the ranker failed, or the
    /// history could not be read. Nothing is recorded in those cases, so a
    /// later run selects instead.
    pub(crate) async fn active_set(&self, surface: PrefetchSurface<'_>) -> Option<ActiveSet> {
        let run_context = surface.run_context;
        let scope = ThreadScopeResolver::resolve_for_turn(
            &self.thread_scope,
            &run_context.scope,
            run_context.actor(),
        );
        let candidates: Vec<(&ProviderToolDefinition, u32)> = surface
            .catalog
            .effective_definitions_with_tokens(surface.policy)
            .collect();
        let candidate_definitions: Vec<&ProviderToolDefinition> = candidates
            .iter()
            .map(|(definition, _)| *definition)
            .collect();
        let availability = self
            .candidate_filter
            .availability(run_context, &candidate_definitions)
            .await;
        // A new selection admits only tools known to be available; a frozen
        // one loses only tools known to be unavailable.
        let available = &availability.available;
        let retained = availability.retained();
        let available_candidates: Vec<(&ProviderToolDefinition, u32)> = candidates
            .iter()
            .copied()
            .filter(|(definition, _)| available.contains(definition.name.as_str()))
            .collect();

        // Two attempts: a lost append race re-reads once and serves whatever
        // the winner recorded.
        for _ in 0..2 {
            let history = match self
                .thread_service
                .read_tool_selection_history(&scope, &run_context.thread_id)
                .await
            {
                Ok(history) => history,
                Err(error) => {
                    debug!(
                        target: TOOL_PREFETCH_LOG_TARGET,
                        error_kind = error.kind_name(),
                        "tool selection history read failed; this run keeps the ordinary tool surface"
                    );
                    return None;
                }
            };
            let outcome = match history {
                Some(history) => {
                    self.serve_recorded(
                        &surface,
                        &scope,
                        &history,
                        &retained,
                        &available_candidates,
                        available,
                    )
                    .await
                }
                None => {
                    self.select_initial(&surface, &scope, &available_candidates, available)
                        .await
                }
            };
            match outcome {
                Served::Active(active) => return Some(active),
                Served::NotApplicable => return None,
                Served::LostRace => continue,
            }
        }
        debug!(
            target: TOOL_PREFETCH_LOG_TARGET,
            "tool selection history kept changing; this run keeps the ordinary tool surface"
        );
        None
    }

    /// Serve the entry in force, appending a new entry first when one of
    /// its tools is no longer authorized or is definitely unavailable, or
    /// when re-selection is on and the prompt cache can no longer be warm.
    /// `retained` names the authorized tools not known to be unavailable;
    /// `candidates` and `admitted` are what a new selection may choose from.
    async fn serve_recorded(
        &self,
        surface: &PrefetchSurface<'_>,
        scope: &ThreadScope,
        history: &ToolSelectionHistory,
        retained: &BTreeSet<String>,
        candidates: &[(&ProviderToolDefinition, u32)],
        admitted: &BTreeSet<String>,
    ) -> Served {
        let available = retained;
        let run_context = surface.run_context;
        let Some(entry) = history.in_force_at(run_context.turn_id, None) else {
            return Served::NotApplicable;
        };
        let frozen = frozen_active_set(
            surface.catalog,
            &entry.advertised,
            entry.tool_search_description.as_deref(),
            available,
        );
        let reselection = reselect::Reselection {
            surface,
            scope,
            history,
            entry,
            candidates,
            admitted,
        };
        if !frozen.unavailable.is_empty()
            && self.config.reselection.enabled()
            && let Some(served) = self
                .reselect(&reselection, ToolSelectionReason::Revoked, None)
                .await
        {
            return served;
        }
        if frozen.unavailable.is_empty() {
            if let Some((reason, activity)) =
                self.reselect_trigger(run_context, scope, history).await
                && let Some(served) = self.reselect(&reselection, reason, Some(activity)).await
            {
                return served;
            }
            debug!(
                target: TOOL_PREFETCH_LOG_TARGET,
                reason = entry.reason.as_str(),
                entries = history.entries.len(),
                advertised_tool_count = frozen.active.definitions.len(),
                est_schema_tokens = frozen.active.advertised_tokens,
                "serving the conversation's recorded tool selection"
            );
            return Served::Active(frozen.active);
        }

        // Security beats cache: drop what was revoked and record it.
        let unavailable: BTreeSet<&str> = frozen.unavailable.iter().map(String::as_str).collect();
        let advertised: Vec<String> = entry
            .advertised
            .iter()
            .filter(|name| !unavailable.contains(name.as_str()))
            .cloned()
            .collect();
        let advertised_set: BTreeSet<String> = advertised.iter().cloned().collect();
        let revoked = ToolSelectionEntry {
            effective_from: self.effective_from(run_context, scope).await,
            reason: ToolSelectionReason::Revoked,
            scores: entry
                .scores
                .iter()
                .filter(|score| advertised_set.contains(&score.name))
                .cloned()
                .collect(),
            ranker_version: entry.ranker_version.clone(),
            // The index lists what is not advertised, so it is rebuilt from
            // the current authorized catalog: a revoked tool must not stay
            // named in it.
            tool_search_description: Some(tool_search_description_excluding(
                surface.catalog,
                surface.policy,
                surface.mode,
                &advertised_set,
            )),
            advertised,
            fallback_reason: None,
            recorded_at: Utc::now(),
        };
        debug!(
            target: TOOL_PREFETCH_LOG_TARGET,
            revoked = ?frozen.unavailable,
            remaining_tool_count = revoked.advertised.len(),
            "removing revoked tools from the conversation's tool selection"
        );
        match self
            .thread_service
            .append_tool_selection_entry(AppendToolSelectionEntryRequest {
                scope: scope.clone(),
                thread_id: run_context.thread_id.clone(),
                expected_entries: history.entries.len(),
                entry: revoked.clone(),
            })
            .await
        {
            Ok(_) => {}
            Err(SessionThreadError::ToolSelectionHistoryConflict { .. }) => {
                return Served::LostRace;
            }
            Err(error) => {
                // The narrowed surface is served either way; revocation must
                // not wait on the audit record.
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "recording a tool revocation failed; serving the narrowed tool selection unrecorded"
                );
            }
        }
        Served::Active(
            frozen_active_set(
                surface.catalog,
                &revoked.advertised,
                revoked.tool_search_description.as_deref(),
                available,
            )
            .active,
        )
    }

    /// Rank the catalog against the opening request and record the result as
    /// the conversation's `initial` entry.
    async fn select_initial(
        &self,
        surface: &PrefetchSurface<'_>,
        scope: &ThreadScope,
        candidates: &[(&ProviderToolDefinition, u32)],
        available: &BTreeSet<String>,
    ) -> Served {
        let run_context = surface.run_context;
        if candidates.is_empty() {
            // Nothing is authorized: there is nothing to select from, and the
            // ordinary surface advertises nothing either.
            return Served::NotApplicable;
        }
        let Some((context, message_sequence)) = self.opening_request(run_context, scope).await
        else {
            return Served::NotApplicable;
        };
        let floor = selection_floor(
            &self.config.always,
            candidates,
            advertised_bridge_tokens(surface.catalog, surface.policy, surface.mode),
        );
        let request = ToolSelectionRequest {
            context,
            called_tools: Vec::new(),
            candidates: candidates
                .iter()
                .map(|(definition, tokens)| ToolSelectionCandidate {
                    definition: (*definition).clone(),
                    est_schema_tokens: *tokens,
                })
                .collect(),
            pinned: floor.names.clone(),
            max_tools: self.config.max_tools,
            token_budget: self.config.token_budget,
            reserved_tools: floor.reserved_tools,
            reserved_tokens: floor.reserved_tokens,
        };
        let classifier = self.classifier.get();
        let started = std::time::Instant::now();
        let outcome = classifier.classify(&request).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let recorded = match outcome {
            Ok(selection) => {
                let chosen = accept_chosen(&request, selection.chosen);
                RecordedChoice {
                    advertised: advertised_names(
                        &floor.names,
                        chosen.iter().map(|tool| tool.name.as_str()),
                    ),
                    scores: chosen
                        .into_iter()
                        .map(|tool| ToolSelectionScore {
                            name: tool.name,
                            score: tool.score,
                        })
                        .collect(),
                    scorer: Some(selection.scorer).filter(|scorer| valid_scorer(scorer)),
                    fallback_reason: None,
                }
            }
            Err(error) => match &self.classifier {
                SelectionClassifier::Local(_) => {
                    debug!(
                        target: TOOL_PREFETCH_LOG_TARGET,
                        classifier = classifier.classifier_name(),
                        ranking = self.config.ranking.as_str(),
                        error_kind = error.kind_label(),
                        latency_ms,
                        "turn-start tool ranking failed; this run keeps the ordinary tool surface"
                    );
                    return Served::NotApplicable;
                }
                SelectionClassifier::Bound(_) => core_set_fallback(candidates, &floor, &error),
            },
        };
        let advertised_set: BTreeSet<String> = recorded.advertised.iter().cloned().collect();
        let entry = ToolSelectionEntry {
            effective_from: ToolSelectionEffectiveFrom {
                turn_id: run_context.turn_id,
                message_sequence,
            },
            reason: ToolSelectionReason::Initial,
            advertised: recorded.advertised,
            scores: recorded.scores,
            ranker_version: recorded.scorer,
            tool_search_description: Some(tool_search_description_excluding(
                surface.catalog,
                surface.policy,
                surface.mode,
                &advertised_set,
            )),
            fallback_reason: recorded.fallback_reason,
            recorded_at: Utc::now(),
        };
        let frozen = frozen_active_set(
            surface.catalog,
            &entry.advertised,
            entry.tool_search_description.as_deref(),
            available,
        )
        .active;
        self.log_selection(&entry, &floor, frozen.advertised_tokens, latency_ms);
        match self
            .thread_service
            .append_tool_selection_entry(AppendToolSelectionEntryRequest {
                scope: scope.clone(),
                thread_id: run_context.thread_id.clone(),
                expected_entries: 0,
                entry,
            })
            .await
        {
            Ok(_) => {}
            Err(SessionThreadError::ToolSelectionHistoryConflict { .. }) => {
                return Served::LostRace;
            }
            Err(error) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "recording the initial tool selection failed; serving it unrecorded"
                );
            }
        }
        Served::Active(frozen)
    }

    /// The run's accepted user message as a conversation context, and its
    /// thread sequence. `None` when the run has no accepted user message with
    /// text (a trigger fire, an attachment-only message, a missing row).
    async fn opening_request(
        &self,
        run_context: &LoopRunContext,
        scope: &ThreadScope,
    ) -> Option<(ConversationContext, Option<u64>)> {
        let Some(message_id) = accepted_task_message_id(run_context) else {
            debug!(
                target: TOOL_PREFETCH_LOG_TARGET,
                "run has no accepted user message; this run keeps the ordinary tool surface"
            );
            return None;
        };
        let record = match self
            .thread_service
            .read_thread_message(scope, &run_context.thread_id, message_id)
            .await
        {
            Ok(Some(record)) if record.kind == MessageKind::User => record,
            Ok(_) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    "accepted user message is unavailable; this run keeps the ordinary tool surface"
                );
                return None;
            }
            Err(error) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "accepted user message read failed; this run keeps the ordinary tool surface"
                );
                return None;
            }
        };
        let context = conversation::opening_request(record.content.as_deref()?);
        if context.is_empty() {
            debug!(
                target: TOOL_PREFETCH_LOG_TARGET,
                "accepted user message has no text; this run keeps the ordinary tool surface"
            );
            return None;
        }
        Some((context, Some(record.sequence)))
    }

    /// The turn a new entry takes effect from.
    async fn effective_from(
        &self,
        run_context: &LoopRunContext,
        scope: &ThreadScope,
    ) -> ToolSelectionEffectiveFrom {
        let message_sequence = match accepted_task_message_id(run_context) {
            Some(message_id) => self
                .thread_service
                .read_thread_message(scope, &run_context.thread_id, message_id)
                .await
                .ok()
                .flatten()
                .map(|record| record.sequence),
            None => None,
        };
        ToolSelectionEffectiveFrom {
            turn_id: run_context.turn_id,
            message_sequence,
        }
    }

    /// Log a first selection. `est_schema_tokens` is the estimate of the
    /// array actually served (bridges included, with the frozen `tool_search`
    /// description), the same figure the "serving" line reports later for
    /// this entry.
    fn log_selection(
        &self,
        entry: &ToolSelectionEntry,
        floor: &SelectionFloor,
        est_schema_tokens: u32,
        latency_ms: u64,
    ) {
        let chosen: Vec<(&str, f32)> = entry
            .scores
            .iter()
            .map(|score| (score.name.as_str(), score.score))
            .collect();
        debug!(
            target: TOOL_PREFETCH_LOG_TARGET,
            classifier = self.classifier.get().classifier_name(),
            scorer = entry.ranker_version.as_deref().unwrap_or(""),
            fallback = entry.fallback_reason.as_deref().unwrap_or(""),
            advertised_tool_count = entry.advertised.len(),
            est_schema_tokens,
            floor = ?floor.names,
            chosen = ?chosen,
            latency_ms,
            "selected the conversation's tools from its opening request"
        );
    }
}

/// What a first selection records: from a classifier's answer, or from the
/// core-set fallback.
struct RecordedChoice {
    advertised: Vec<String>,
    scores: Vec<ToolSelectionScore>,
    scorer: Option<String>,
    fallback_reason: Option<String>,
}

/// The classifier's answer, checked: only candidates that are not pinned,
/// once each, with a finite non-negative score, in the classifier's order
/// until `max_tools` or the token budget is reached. A classifier cannot
/// grant authority, and the floor is the host's to add.
fn accept_chosen(request: &ToolSelectionRequest, chosen: Vec<ChosenTool>) -> Vec<ChosenTool> {
    let tokens_by_name: BTreeMap<&str, u32> = request
        .candidates
        .iter()
        .map(|candidate| (candidate.name(), candidate.est_schema_tokens))
        .collect();
    let mut kept_names: BTreeSet<String> = request.pinned.iter().cloned().collect();
    let mut count = request.reserved_tools;
    let mut tokens = request.reserved_tokens;
    let mut kept = Vec::new();
    let mut dropped = 0_usize;
    let mut truncated = 0_usize;
    for tool in chosen {
        let est_schema_tokens = tokens_by_name.get(tool.name.as_str()).copied();
        let Some(est_schema_tokens) = est_schema_tokens.filter(|_| {
            tool.score.is_finite() && tool.score >= 0.0 && !kept_names.contains(&tool.name)
        }) else {
            dropped += 1;
            continue;
        };
        if count >= request.max_tools
            || tokens.saturating_add(est_schema_tokens) > request.token_budget
        {
            truncated += 1;
            continue;
        }
        count += 1;
        tokens = tokens.saturating_add(est_schema_tokens);
        kept_names.insert(tool.name.clone());
        kept.push(tool);
    }
    if dropped > 0 || truncated > 0 {
        debug!(
            target: TOOL_PREFETCH_LOG_TARGET,
            dropped,
            truncated,
            "the tool classifier's answer broke the selection contract and was repaired"
        );
    }
    kept
}

/// Whether a classifier's scale identifier can be recorded as-is.
fn valid_scorer(scorer: &str) -> bool {
    !scorer.is_empty() && scorer.len() <= 256
}

/// The fallback when a bound classifier fails at a conversation's first
/// selection: the core tools among the candidates plus the floor, recorded
/// with the failure's label. It is what the conversation would advertise
/// without turn-start selection, and everything else stays reachable
/// through `tool_search` -> `tool_call`.
fn core_set_fallback(
    candidates: &[(&ProviderToolDefinition, u32)],
    floor: &SelectionFloor,
    error: &ToolSelectionError,
) -> RecordedChoice {
    let core = candidates
        .iter()
        .filter(|(definition, _)| is_core_tool_definition(definition))
        .map(|(definition, _)| definition.name.as_str());
    RecordedChoice {
        advertised: advertised_names(&floor.names, core),
        scores: Vec::new(),
        scorer: None,
        fallback_reason: Some(error.kind_label().to_string()),
    }
}

/// What one attempt at serving a run produced.
enum Served {
    Active(ActiveSet),
    /// Selection does not apply to this run; keep the ordinary surface.
    NotApplicable,
    /// Another writer appended first; re-read and serve what it recorded.
    LostRace,
}
