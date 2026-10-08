//! Turn-start tool selection: advertise the deferred tools a conversation's
//! opening request predicts, beside the ordinary disclosure surface.
//!
//! With progressive disclosure alone, a wide catalog advertises only the core
//! tools and the discovery bridges, and any other tool costs a `tool_search`
//! round trip before its first use. Turn-start selection asks the bound
//! [`ToolSelectionClassifier`] which deferred tools the opening request is
//! likely to need and advertises those as well, up to
//! [`ToolSelectionConfig::max_tools`]. Core tools and bridges are never
//! candidates: they stay advertised, so prompt text that names them stays
//! true, and everything not selected stays reachable through `tool_search`
//! → `tool_call`.
//!
//! # Chosen once, then frozen
//!
//! The request's `tools` array is part of the provider's cached prompt
//! prefix: changing it re-bills the whole prompt. So a conversation selects
//! once, at its first turn with user text, records the result
//! (`ironclaw_threads::ToolSelectionRecord`), and every later turn rebuilds
//! the same list from that record without asking the classifier. A selected
//! tool that later loses authorization is left out when the list is rebuilt.
//!
//! A classifier failure is recorded too, as an empty selection with the
//! failure's label: the conversation keeps the ordinary surface rather than
//! switching surfaces on a later turn while its cache is warm.
//!
//! A list is served only once it is recorded. When the record cannot be read
//! or written, the run keeps the ordinary surface and a later turn tries
//! again.
//!
//! # Confidentiality
//!
//! The classifier is handed the message the run was accepted with, cut to
//! `MAX_CONVERSATION_CONTEXT_BYTES`. For an ordinary conversation that is what
//! the user typed. A subagent runs on a thread of its own, so it selects for
//! itself, from the task its parent handed it: text the parent's model wrote.
//! Nothing here logs it: selection logs at
//! `debug!` on [`TOOL_SELECTION_LOG_TARGET`] carry only counts, tool names
//! and scores.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use chrono::Utc;
use ironclaw_host_api::{capability_surface::CapabilitySurfacePolicy, ids::CapabilityId};
use ironclaw_loop_contracts::{
    ChosenTool, ConversationContext, LoopRunContext, MAX_CONVERSATION_CONTEXT_BYTES,
    ToolSelectionCandidate, ToolSelectionClassifier, ToolSelectionError, ToolSelectionRequest,
};
use ironclaw_threads::{
    MAX_TOOL_SELECTION_SCORER_BYTES, MAX_TOOL_SELECTION_TOOLS, MessageKind,
    RecordToolSelectionRequest, SelectedTool, SessionThreadService, TOOL_SELECTION_SCHEMA_VERSION,
    ThreadScope, ToolSelectionFallbackReason, ToolSelectionRecord,
};
use tracing::debug;

use crate::{
    ThreadScopeResolver, accepted_task_message_id,
    tool_disclosure::{CapabilityCatalog, DisclosureCaps},
};

/// `tracing` target for every selection log line.
pub(crate) const TOOL_SELECTION_LOG_TARGET: &str = "ironclaw::reborn::tool_selection";

/// Why a [`ToolSelectionConfig`] was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolSelectionConfigError {
    #[error("[tool_selection] max_tools must be between 1 and {max}, got {value}")]
    MaxToolsOutOfRange { value: usize, max: usize },
    #[error("[tool_selection] token_budget must be more than 0")]
    ZeroTokenBudget,
}

/// Validated turn-start selection settings and the classifier that chooses.
#[derive(Clone)]
pub struct ToolSelectionConfig {
    max_tools: usize,
    token_budget: u32,
    classifier: Arc<dyn ToolSelectionClassifier>,
}

impl ToolSelectionConfig {
    /// `max_tools` is the most tools a conversation may select, and
    /// `token_budget` the most estimated schema tokens they may add up to.
    /// The operator-facing defaults live where the setting is resolved (the
    /// binary's runtime setup); this type only checks them.
    pub fn new(
        max_tools: usize,
        token_budget: u32,
        classifier: Arc<dyn ToolSelectionClassifier>,
    ) -> Result<Self, ToolSelectionConfigError> {
        if !(1..=MAX_TOOL_SELECTION_TOOLS).contains(&max_tools) {
            return Err(ToolSelectionConfigError::MaxToolsOutOfRange {
                value: max_tools,
                max: MAX_TOOL_SELECTION_TOOLS,
            });
        }
        if token_budget == 0 {
            return Err(ToolSelectionConfigError::ZeroTokenBudget);
        }
        Ok(Self {
            max_tools,
            token_budget,
            classifier,
        })
    }

    pub fn max_tools(&self) -> usize {
        self.max_tools
    }

    pub fn token_budget(&self) -> u32 {
        self.token_budget
    }

    pub fn classifier_name(&self) -> &str {
        self.classifier.classifier_name()
    }
}

impl fmt::Debug for ToolSelectionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolSelectionConfig")
            .field("max_tools", &self.max_tools)
            .field("token_budget", &self.token_budget)
            .field("classifier", &self.classifier.classifier_name())
            .finish()
    }
}

/// Turn-start selection bound to one runtime: the settings and the thread
/// store that holds each conversation's record.
#[derive(Clone)]
pub(crate) struct TurnStartToolSelection {
    config: ToolSelectionConfig,
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
}

impl TurnStartToolSelection {
    pub(crate) fn new(
        config: ToolSelectionConfig,
        thread_service: Arc<dyn SessionThreadService>,
        thread_scope: ThreadScope,
    ) -> Self {
        Self {
            config,
            thread_service,
            thread_scope,
        }
    }

    /// The capability ids of the tools this run's conversation selected,
    /// selecting them first when the conversation has no record yet.
    ///
    /// Empty when selection does not apply to this run: the surface is not
    /// deferred (every tool is advertised already), no deferred tool is
    /// authorized, the run has no user text to select from, or the record
    /// could not be read or written.
    pub(crate) async fn selected_tools(
        &self,
        run_context: &LoopRunContext,
        catalog: &CapabilityCatalog,
        policy: &CapabilitySurfacePolicy,
        caps: DisclosureCaps,
    ) -> Vec<CapabilityId> {
        if !catalog.defers(policy, caps) {
            return Vec::new();
        }
        let scope = ThreadScopeResolver::resolve_for_turn(
            &self.thread_scope,
            &run_context.scope,
            run_context.actor(),
        );
        match self
            .thread_service
            .read_tool_selection(&scope, &run_context.thread_id)
            .await
        {
            Ok(Some(record)) => return selected_ids(&record),
            Ok(None) => {}
            Err(error) => {
                debug!(
                    target: TOOL_SELECTION_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "tool selection record read failed; this run keeps the ordinary tool surface"
                );
                return Vec::new();
            }
        }
        let candidates: Vec<ToolSelectionCandidate> = catalog
            .deferred_definitions_with_tokens(policy)
            .map(|(definition, est_schema_tokens)| ToolSelectionCandidate {
                definition: definition.clone(),
                est_schema_tokens,
            })
            .collect();
        if candidates.is_empty() {
            return Vec::new();
        }
        let Some(context) = self.opening_request(run_context, &scope).await else {
            debug!(
                target: TOOL_SELECTION_LOG_TARGET,
                "no accepted user message with text could be used; this run keeps the ordinary tool surface"
            );
            return Vec::new();
        };
        let request = ToolSelectionRequest {
            context,
            candidates,
            max_tools: self.config.max_tools,
            token_budget: self.config.token_budget,
        };
        let started = std::time::Instant::now();
        let outcome = self.config.classifier.classify(&request).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let record = match outcome {
            Ok(selection) => {
                let chosen = accept_chosen(&request, selection.chosen);
                debug!(
                    target: TOOL_SELECTION_LOG_TARGET,
                    classifier = self.config.classifier.classifier_name(),
                    scorer = selection.scorer.as_str(),
                    candidates = request.candidates.len(),
                    chosen = ?chosen,
                    latency_ms,
                    "selected the conversation's tools from its opening request"
                );
                ToolSelectionRecord {
                    schema_version: TOOL_SELECTION_SCHEMA_VERSION,
                    turn_id: run_context.turn_id,
                    selected: chosen,
                    scorer: Some(selection.scorer).filter(|scorer| valid_scorer(scorer)),
                    fallback_reason: None,
                    recorded_at: Utc::now(),
                }
            }
            Err(error) => {
                debug!(
                    target: TOOL_SELECTION_LOG_TARGET,
                    classifier = self.config.classifier.classifier_name(),
                    error_kind = error.kind_label(),
                    latency_ms,
                    "turn-start tool selection failed; the conversation keeps the ordinary tool surface"
                );
                ToolSelectionRecord {
                    schema_version: TOOL_SELECTION_SCHEMA_VERSION,
                    turn_id: run_context.turn_id,
                    selected: Vec::new(),
                    scorer: None,
                    fallback_reason: Some(fallback_reason(&error)),
                    recorded_at: Utc::now(),
                }
            }
        };
        match self
            .thread_service
            .record_tool_selection(RecordToolSelectionRequest {
                scope,
                thread_id: run_context.thread_id.clone(),
                record,
            })
            .await
        {
            // The stored record, which is another run's when it recorded first.
            Ok(stored) => selected_ids(&stored),
            Err(error) => {
                // An unrecorded list would not survive to the next turn, so
                // serving it would change the array while the cache is warm.
                debug!(
                    target: TOOL_SELECTION_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "recording the tool selection failed; this run keeps the ordinary tool surface"
                );
                Vec::new()
            }
        }
    }

    /// The run's accepted user message as a selection context. `None` when
    /// the run has none (a trigger fire, say), it is blank, or it could not
    /// be read.
    async fn opening_request(
        &self,
        run_context: &LoopRunContext,
        scope: &ThreadScope,
    ) -> Option<ConversationContext> {
        let message_id = accepted_task_message_id(run_context)?;
        let record = match self
            .thread_service
            .read_thread_message(scope, &run_context.thread_id, message_id)
            .await
        {
            Ok(record) => record?,
            Err(error) => {
                debug!(
                    target: TOOL_SELECTION_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "accepted user message read failed"
                );
                return None;
            }
        };
        if record.kind != MessageKind::User {
            return None;
        }
        let text = bounded(record.content.as_deref()?.trim());
        (!text.is_empty()).then(|| ConversationContext::new(vec![text.to_string()]))
    }
}

/// `text` cut to [`MAX_CONVERSATION_CONTEXT_BYTES`] on a character boundary.
fn bounded(text: &str) -> &str {
    if text.len() <= MAX_CONVERSATION_CONTEXT_BYTES {
        return text;
    }
    let mut end = MAX_CONVERSATION_CONTEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

fn selected_ids(record: &ToolSelectionRecord) -> Vec<CapabilityId> {
    record
        .selected
        .iter()
        .map(|tool| tool.capability_id.clone())
        .collect()
}

/// The classifier's answer, checked: only candidates, once each, with a
/// finite non-negative score, in the classifier's order. A tool past
/// `max_tools`, or one that would take the total past the token budget, is
/// left out; a later, smaller one may still fit. A classifier cannot grant
/// authority.
fn accept_chosen(request: &ToolSelectionRequest, chosen: Vec<ChosenTool>) -> Vec<SelectedTool> {
    let candidates: BTreeMap<&str, &ToolSelectionCandidate> = request
        .candidates
        .iter()
        .map(|candidate| (candidate.name(), candidate))
        .collect();
    let mut kept_names: BTreeSet<&str> = BTreeSet::new();
    let mut tokens = 0_u32;
    let mut kept = Vec::new();
    let mut dropped = 0_usize;
    for tool in &chosen {
        let Some((name, candidate)) =
            candidates
                .get_key_value(tool.name.as_str())
                .filter(|(name, _)| {
                    tool.score.is_finite() && tool.score >= 0.0 && !kept_names.contains(**name)
                })
        else {
            dropped += 1;
            continue;
        };
        if kept.len() >= request.max_tools
            || tokens.saturating_add(candidate.est_schema_tokens) > request.token_budget
        {
            dropped += 1;
            continue;
        }
        tokens = tokens.saturating_add(candidate.est_schema_tokens);
        kept_names.insert(name);
        kept.push(SelectedTool {
            capability_id: candidate.definition.capability_id.clone(),
            score: tool.score,
        });
    }
    if dropped > 0 {
        debug!(
            target: TOOL_SELECTION_LOG_TARGET,
            dropped,
            "the tool classifier's answer broke the selection contract and was repaired"
        );
    }
    kept
}

/// Whether a classifier's scale identifier can be recorded as-is.
fn valid_scorer(scorer: &str) -> bool {
    !scorer.is_empty() && scorer.len() <= MAX_TOOL_SELECTION_SCORER_BYTES
}

/// How a classifier failure is recorded.
fn fallback_reason(error: &ToolSelectionError) -> ToolSelectionFallbackReason {
    match error {
        ToolSelectionError::Unavailable { .. } => ToolSelectionFallbackReason::Unavailable,
        ToolSelectionError::Timeout { .. } => ToolSelectionFallbackReason::Timeout,
        ToolSelectionError::Unauthorized => ToolSelectionFallbackReason::Unauthorized,
        ToolSelectionError::PaymentRequired => ToolSelectionFallbackReason::PaymentRequired,
        ToolSelectionError::Rejected { .. } => ToolSelectionFallbackReason::Rejected,
        ToolSelectionError::RateLimited => ToolSelectionFallbackReason::RateLimited,
        ToolSelectionError::InvalidOutput { .. } => ToolSelectionFallbackReason::InvalidOutput,
    }
}

#[cfg(test)]
mod tests;
