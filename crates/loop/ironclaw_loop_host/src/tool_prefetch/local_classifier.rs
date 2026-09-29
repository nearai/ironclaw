//! The host-bundled [`ToolSelectionClassifier`]: rank the candidates with a
//! tool ranker once per conversation segment, then merge the rankings by rank
//! so every segment's best tools come before any segment's weaker ones, until
//! `max_tools` or the token budget is reached. Score thresholds are optional
//! and off by default; see the parent module.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use ironclaw_loop_contracts::{
    ChosenTool, ConversationContext, ProviderToolDefinition, RankedTool, ToolRetrievalError,
    ToolRetrievalIndex, ToolRetrievalProvider, ToolSearchOutcome, ToolSearchQueryClass,
    ToolSelection, ToolSelectionClassifier, ToolSelectionError, ToolSelectionRequest,
};
use tracing::debug;

use super::{
    TOOL_PREFETCH_LOG_TARGET, TOOL_PREFETCH_MANDATORY_FLOOR, ToolPrefetchConfig,
    ToolPrefetchRanking,
};
use crate::{
    tool_disclosure::definition_matches_provider_name,
    tool_search::{AuthorizedToolSearchIndex, sanitize_provider_ranking},
};

/// Ranker version recorded for lexical selection: bounded BM25F without the
/// query-term coverage rule `tool_search` applies (a first message rarely
/// covers a tool's terms), reading up to [`MAX_SEGMENT_QUERY_TERMS`] terms of
/// each conversation segment.
pub(crate) const LEXICAL_SELECTION_RANKER_VERSION: &str = "bounded-bm25f-terms-v2";

/// Unique terms lexical selection reads from one segment. `tool_search`
/// reads 32 terms of a query; a segment is prose up to a few KiB long, and a
/// tool named only near its end must still count.
const MAX_SEGMENT_QUERY_TERMS: usize = 512;

/// Most skipped candidates one selection log line lists.
const MAX_LOGGED_SKIPPED: usize = 20;

/// Name the local classifier reports in logs.
pub(crate) const LOCAL_CLASSIFIER_NAME: &str = "local";

/// Whether a ranker's scores are cosine similarities, the only scale the
/// absolute threshold means anything on.
fn is_cosine_scale(ranking: ToolPrefetchRanking, ranker_version: &str) -> bool {
    ranking == ToolPrefetchRanking::Semantic && ranker_version.starts_with("dense-cosine")
}

/// Lexical selection ranker: BM25F scoring every document that shares a term
/// with the query (up to [`MAX_SEGMENT_QUERY_TERMS`] of them), without
/// `tool_search`'s coverage rule or exact-identifier bonus. English function words are dropped from the query first: an
/// opening request is prose, not a search query, and a tool that shares only
/// "what" or "the" with it is no evidence at all.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LexicalSelectionRetrieval;

#[async_trait]
impl ToolRetrievalProvider for LexicalSelectionRetrieval {
    fn ranker_version(&self) -> &str {
        LEXICAL_SELECTION_RANKER_VERSION
    }

    async fn fit(
        &self,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        Ok(Arc::new(LexicalSelectionIndex(
            AuthorizedToolSearchIndex::new(definitions.iter()),
        )))
    }
}

#[derive(Debug)]
struct LexicalSelectionIndex(AuthorizedToolSearchIndex);

#[async_trait]
impl ToolRetrievalIndex for LexicalSelectionIndex {
    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
        let ranked = self.0.score_terms_reading(
            &without_function_words(query),
            limit,
            MAX_SEGMENT_QUERY_TERMS,
        );
        Ok(ToolSearchOutcome {
            query_class: if ranked.is_empty() {
                ToolSearchQueryClass::NoMatch
            } else {
                ToolSearchQueryClass::Lexical
            },
            ranked,
        })
    }
}

/// Words that carry no tool-relevant meaning in an opening request.
const FUNCTION_WORDS: &[&str] = &[
    "a", "about", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be", "been", "but",
    "by", "can", "could", "did", "do", "does", "for", "from", "had", "has", "have", "he", "her",
    "here", "him", "his", "how", "i", "if", "in", "into", "is", "it", "its", "just", "me", "might",
    "my", "no", "not", "of", "on", "or", "our", "please", "she", "should", "so", "some", "than",
    "that", "the", "their", "them", "then", "there", "these", "they", "this", "those", "to", "us",
    "was", "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "would",
    "you", "your",
];

fn without_function_words(query: &str) -> String {
    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter(|word| {
            !FUNCTION_WORDS
                .iter()
                .any(|function_word| word.eq_ignore_ascii_case(function_word))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Why a ranked candidate was not advertised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipReason {
    BelowMinSimilarity,
    BelowMinRelative,
    MaxTools,
    TokenBudget,
}

/// One ranked candidate the selection kept.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SelectedTool {
    pub(crate) name: String,
    /// Best 1-based rank over the segments' rankings.
    pub(crate) rank: usize,
    /// Best score over the segments' rankings (on the ranker's scale).
    pub(crate) score: f32,
    /// Index of the segment whose ranking admitted it.
    pub(crate) segment: usize,
    pub(crate) est_schema_tokens: u32,
}

/// One ranked candidate the selection dropped.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SkippedTool {
    pub(crate) name: String,
    /// 1-based rank where the merge first met it.
    pub(crate) rank: usize,
    pub(crate) score: f32,
    pub(crate) reason: SkipReason,
}

/// The always-advertised part of a selection: the bridges, plus the
/// authorized mandatory and extra floor tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectionFloor {
    /// Catalog floor tools, in floor order (mandatory first, then extras).
    pub(crate) names: Vec<String>,
    /// Array slots the floor takes, bridges included.
    pub(crate) reserved_tools: usize,
    /// Estimated schema tokens the floor takes, bridges included.
    pub(crate) reserved_tokens: u32,
}

/// The floor for one catalog. A floor name never grants authority: it only
/// matches a candidate.
pub(crate) fn selection_floor(
    always: &[String],
    candidates: &[(&ProviderToolDefinition, u32)],
    bridge_tokens: u32,
) -> SelectionFloor {
    let mut advertised: BTreeSet<String> = BTreeSet::new();
    let mut names = Vec::new();
    let mut reserved_tokens = bridge_tokens;
    let floor_names = TOOL_PREFETCH_MANDATORY_FLOOR
        .iter()
        .copied()
        .filter(|name| !crate::tool_disclosure::is_bridge_name(name))
        .chain(always.iter().map(String::as_str));
    for floor_name in floor_names {
        for (definition, tokens) in candidates {
            if definition_matches_provider_name(definition, floor_name)
                && advertised.insert(definition.name.to_string())
            {
                names.push(definition.name.to_string());
                reserved_tokens = reserved_tokens.saturating_add(*tokens);
            }
        }
    }
    SelectionFloor {
        reserved_tools: bridge_count() + names.len(),
        names,
        reserved_tokens,
    }
}

fn bridge_count() -> usize {
    TOOL_PREFETCH_MANDATORY_FLOOR
        .iter()
        .filter(|name| crate::tool_disclosure::is_bridge_name(name))
        .count()
}

/// The advertised names for a floor plus chosen catalog tools, in the order
/// the `tools` array carries them: catalog tools by provider name, then the
/// bridges.
pub(crate) fn advertised_names<'a>(
    floor: &'a [String],
    chosen: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    floor
        .iter()
        .map(String::as_str)
        .chain(chosen)
        .map(str::to_string)
        .collect::<BTreeSet<String>>()
        .into_iter()
        .chain(
            TOOL_PREFETCH_MANDATORY_FLOOR
                .iter()
                .filter(|name| crate::tool_disclosure::is_bridge_name(name))
                .map(|name| name.to_string()),
        )
        .collect()
}

/// The result of ranking one catalog against one conversation context.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolSelectionPlan {
    /// Every advertised name, in the order the `tools` array carries them:
    /// catalog tools by provider name, then the bridges.
    pub(crate) advertised: Vec<String>,
    /// The floor tools (mandatory and extras) that were authorized.
    pub(crate) floor: Vec<String>,
    pub(crate) selected: Vec<SelectedTool>,
    pub(crate) skipped: Vec<SkippedTool>,
    /// Estimated schema tokens of everything advertised.
    pub(crate) est_schema_tokens: u32,
}

/// The inputs [`plan_selection`] reads, gathered so the function stays pure.
#[cfg(test)]
pub(crate) struct SelectionInputs<'a> {
    pub(crate) config: &'a ToolPrefetchConfig,
    /// Authorized, available catalog definitions with their estimated schema
    /// tokens, in catalog order.
    pub(crate) candidates: &'a [(&'a ProviderToolDefinition, u32)],
    /// Estimated tokens of the three bridges as advertised.
    pub(crate) bridge_tokens: u32,
    /// One sanitized ranking per conversation segment, in segment order
    /// (newest message first): best first, names from `candidates` only.
    pub(crate) rankings: &'a [Vec<RankedTool>],
    pub(crate) ranker_version: &'a str,
}

/// Choose the advertised tools: the floor, then the segments' rankings
/// merged by rank ([`merge_rankings`]), until `max_tools` or the token
/// budget is reached. Pure: the same inputs always produce the same plan.
/// The production path runs the same steps, with the floor computed by the
/// host and the choice made behind the classifier port.
#[cfg(test)]
pub(crate) fn plan_selection(inputs: SelectionInputs<'_>) -> ToolSelectionPlan {
    let floor = selection_floor(
        &inputs.config.always,
        inputs.candidates,
        inputs.bridge_tokens,
    );
    let (selected, skipped) = merge_rankings(
        inputs.config,
        inputs.candidates,
        &floor,
        inputs.rankings,
        inputs.ranker_version,
    );
    let est_schema_tokens = selected.iter().fold(floor.reserved_tokens, |sum, tool| {
        sum.saturating_add(tool.est_schema_tokens)
    });
    ToolSelectionPlan {
        advertised: advertised_names(&floor.names, selected.iter().map(|tool| tool.name.as_str())),
        floor: floor.names,
        selected,
        skipped,
        est_schema_tokens,
    }
}

/// Merge one ranking per segment into the tools to advertise past the
/// floor, and the ranked tools left out, with why.
///
/// The merge is round-robin by rank: every segment's first-ranked tool, in
/// segment order, then every segment's second, and so on. A tool already
/// taken is not taken again (its best rank and score are kept). So each
/// segment's best tools get in before any segment's weaker ones, and a topic
/// that fills only one message of many still gets its top tools, however
/// many words the other topics have. The merge stops adding at `max_tools`
/// or at the first tool that does not fit the token budget.
///
/// Thresholds apply per segment, against that segment's top score, and only
/// when configured (both are 0, off, by default); the absolute one applies
/// only to cosine scores.
fn merge_rankings(
    config: &ToolPrefetchConfig,
    candidates: &[(&ProviderToolDefinition, u32)],
    floor: &SelectionFloor,
    rankings: &[Vec<RankedTool>],
    ranker_version: &str,
) -> (Vec<SelectedTool>, Vec<SkippedTool>) {
    let cosine = is_cosine_scale(config.ranking, ranker_version);
    let tokens_by_name: BTreeMap<&str, u32> = candidates
        .iter()
        .map(|(definition, tokens)| (definition.name.as_str(), *tokens))
        .collect();
    let mut advertised_catalog: BTreeSet<&str> = floor.names.iter().map(String::as_str).collect();
    let mut count = floor.reserved_tools;
    let mut tokens = floor.reserved_tokens;

    let mut selected: Vec<SelectedTool> = Vec::new();
    let mut selected_at: BTreeMap<&str, usize> = BTreeMap::new();
    let mut skipped: Vec<SkippedTool> = Vec::new();
    let mut skipped_names: BTreeSet<&str> = BTreeSet::new();
    let mut capped: Option<SkipReason> = None;
    let depth = rankings.iter().map(Vec::len).max().unwrap_or(0);
    for index in 0..depth {
        let rank = index + 1;
        for (segment, ranking) in rankings.iter().enumerate() {
            let Some(tool) = ranking.get(index) else {
                continue;
            };
            let name = tool.name.as_str();
            if let Some(position) = selected_at.get(name) {
                if let Some(kept) = selected.get_mut(*position) {
                    kept.score = kept.score.max(tool.score);
                }
                continue;
            }
            if advertised_catalog.contains(name) {
                continue;
            }
            let Some(est_schema_tokens) = tokens_by_name.get(name).copied() else {
                continue;
            };
            let top_score = ranking.first().map_or(0.0, |top| top.score);
            let reason = if let Some(reason) = capped {
                Some(reason)
            } else if cosine && tool.score < config.min_similarity {
                Some(SkipReason::BelowMinSimilarity)
            } else if top_score <= 0.0 || tool.score < config.min_relative * top_score {
                Some(SkipReason::BelowMinRelative)
            } else if count >= config.max_tools {
                capped = Some(SkipReason::MaxTools);
                capped
            } else if tokens.saturating_add(est_schema_tokens) > config.token_budget {
                capped = Some(SkipReason::TokenBudget);
                capped
            } else {
                None
            };
            if let Some(reason) = reason {
                if skipped_names.insert(name) {
                    skipped.push(SkippedTool {
                        name: tool.name.clone(),
                        rank,
                        score: tool.score,
                        reason,
                    });
                }
                continue;
            }
            count += 1;
            tokens = tokens.saturating_add(est_schema_tokens);
            advertised_catalog.insert(name);
            selected_at.insert(name, selected.len());
            selected.push(SelectedTool {
                name: tool.name.clone(),
                rank,
                score: tool.score,
                segment,
                est_schema_tokens,
            });
        }
    }
    // A tool one segment's threshold left out may have been taken from
    // another segment's ranking.
    skipped.retain(|tool| !selected_at.contains_key(tool.name.as_str()));
    (selected, skipped)
}

/// The ranker-plus-merge classifier: the settings and the ranker they
/// apply to (BM25F for lexical selection, the bound ranker for semantic).
#[derive(Debug)]
pub(crate) struct RankingToolClassifier {
    config: ToolPrefetchConfig,
    ranker: Arc<dyn ToolRetrievalProvider>,
}

/// The rankings of one classification, one per segment, and how they were
/// repaired.
struct SegmentRankings {
    rankings: Vec<Vec<RankedTool>>,
    repaired: usize,
}

impl RankingToolClassifier {
    pub(crate) fn new(config: ToolPrefetchConfig, ranker: Arc<dyn ToolRetrievalProvider>) -> Self {
        Self { config, ranker }
    }

    /// Rank the candidates against every segment of `context`, in one
    /// `search_many` call on one fitted index.
    async fn rank(
        &self,
        candidates: &[(&ProviderToolDefinition, u32)],
        context: &ConversationContext,
        pinned: usize,
    ) -> Result<SegmentRankings, ToolRetrievalError> {
        let segments = super::conversation::segments(
            context,
            self.config.segment_bytes,
            self.config.context_messages,
        );
        if segments.is_empty() {
            return Ok(SegmentRankings {
                rankings: Vec::new(),
                repaired: 0,
            });
        }
        let definitions: Vec<ProviderToolDefinition> = candidates
            .iter()
            .map(|(definition, _)| (*definition).clone())
            .collect();
        let corpus: BTreeSet<String> = definitions
            .iter()
            .map(|definition| definition.name.to_string())
            .collect();
        let limit = self
            .config
            .max_tools
            .saturating_add(TOOL_PREFETCH_MANDATORY_FLOOR.len())
            .saturating_add(self.config.always.len())
            .saturating_add(pinned)
            .min(definitions.len());
        let index = self.ranker.fit(&definitions).await?;
        let mut outcomes = index.search_many(&segments, limit).await?.into_iter();
        let mut rankings = Vec::with_capacity(segments.len());
        let mut repaired = 0_usize;
        // One ranking per segment: a missing outcome is an empty ranking and
        // an extra one is ignored, both counted as repairs.
        for _ in &segments {
            let ranked = outcomes
                .next()
                .map(|outcome| outcome.ranked)
                .unwrap_or_else(|| {
                    repaired += 1;
                    Vec::new()
                });
            let (ranked, repairs) = sanitize_provider_ranking(ranked, &corpus, limit);
            if repairs.any() {
                repaired += 1;
            }
            rankings.push(ranked);
        }
        repaired += outcomes.count();
        Ok(SegmentRankings { rankings, repaired })
    }
}

#[async_trait]
impl ToolSelectionClassifier for RankingToolClassifier {
    fn classifier_name(&self) -> &str {
        LOCAL_CLASSIFIER_NAME
    }

    async fn classify(
        &self,
        request: &ToolSelectionRequest,
    ) -> Result<ToolSelection, ToolSelectionError> {
        let candidates: Vec<(&ProviderToolDefinition, u32)> = request
            .candidates
            .iter()
            .map(|candidate| (&candidate.definition, candidate.est_schema_tokens))
            .collect();
        let ranker_version = self.ranker.ranker_version().to_string();
        let SegmentRankings { rankings, repaired } = self
            .rank(&candidates, &request.context, request.pinned.len())
            .await?;
        let floor = SelectionFloor {
            names: request.pinned.clone(),
            reserved_tools: request.reserved_tools,
            reserved_tokens: request.reserved_tokens,
        };
        let (selected, skipped) = merge_rankings(
            &self.config,
            &candidates,
            &floor,
            &rankings,
            &ranker_version,
        );
        // Names, ranks, scores and counts only: never a segment's text.
        let segment_results: Vec<usize> = rankings.iter().map(Vec::len).collect();
        let selected_ranks: Vec<(&str, usize, f32, usize)> = selected
            .iter()
            .map(|tool| (tool.name.as_str(), tool.rank, tool.score, tool.segment))
            .collect();
        let skipped_ranks: Vec<(&str, usize, f32, SkipReason)> = skipped
            .iter()
            .take(MAX_LOGGED_SKIPPED)
            .map(|tool| (tool.name.as_str(), tool.rank, tool.score, tool.reason))
            .collect();
        debug!(
            target: TOOL_PREFETCH_LOG_TARGET,
            ranking = self.config.ranking.as_str(),
            ranker_version = %ranker_version,
            segment_count = rankings.len(),
            segment_results = ?segment_results,
            repaired_segments = repaired,
            selected = ?selected_ranks,
            skipped_count = skipped.len(),
            skipped = ?skipped_ranks,
            "ranked the candidates against each conversation segment and merged by rank"
        );
        Ok(ToolSelection {
            chosen: selected
                .into_iter()
                .map(|tool| ChosenTool::new(tool.name, tool.score))
                .collect(),
            scorer: ranker_version,
        })
    }
}
