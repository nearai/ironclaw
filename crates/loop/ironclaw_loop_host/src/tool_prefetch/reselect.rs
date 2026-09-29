//! Re-selection: choosing a conversation's tools again once its prompt cache
//! can no longer be warm, or when a selected tool was revoked. See the parent
//! module's docs for when it runs.

use std::collections::BTreeSet;

use chrono::Utc;
use ironclaw_llm::PromptCacheLifetime;
use ironclaw_loop_contracts::{
    ConversationContext, LoopRunContext, ProviderToolDefinition, ToolSelectionCandidate,
    ToolSelectionRequest,
};
use ironclaw_threads::{
    AppendToolSelectionEntryRequest, MessageKind, MessageStatus, SessionThreadError,
    ThreadMessageRangeRequest, ThreadScope, ToolSelectionActivity, ToolSelectionEffectiveFrom,
    ToolSelectionEntry, ToolSelectionHistory, ToolSelectionReason, ToolSelectionScore,
};
use tracing::debug;

use super::{
    PrefetchSurface, Served, TOOL_PREFETCH_LOG_TARGET, ToolPrefetch, accept_chosen,
    local_classifier::{advertised_names, selection_floor},
    valid_scorer,
};
use crate::{
    ThreadScopeResolver, accepted_task_message_id,
    tool_disclosure::{
        advertised_bridge_tokens, frozen_active_set, tool_search_description_excluding,
    },
};

/// How many transcript rows before the run's message a re-selection scans
/// for user messages. Tool results and replies sit between user messages,
/// so this is several rows per user message the window may hold; the
/// window's message count and byte ceiling bound what is kept.
const WINDOW_SCAN_MESSAGES: u64 = 256;

/// Everything one re-selection reads.
pub(super) struct Reselection<'a> {
    pub(super) surface: &'a PrefetchSurface<'a>,
    pub(super) scope: &'a ThreadScope,
    pub(super) history: &'a ToolSelectionHistory,
    /// The entry in force, which a re-selection replaces.
    pub(super) entry: &'a ToolSelectionEntry,
    /// Authorized candidates known to be available, in catalog order.
    pub(super) candidates: &'a [(&'a ProviderToolDefinition, u32)],
    /// Their names.
    pub(super) admitted: &'a BTreeSet<String>,
}

impl ToolPrefetch {
    /// Whether the prompt cache behind the entry in force can no longer be
    /// warm at this turn boundary, and why; with the conversation's activity
    /// record, which the re-selection reads next. Every unknown answers
    /// "warm".
    pub(super) async fn reselect_trigger(
        &self,
        run_context: &LoopRunContext,
        scope: &ThreadScope,
        history: &ToolSelectionHistory,
    ) -> Option<(ToolSelectionReason, ToolSelectionActivity)> {
        let config = &self.config.reselection;
        if !config.enabled() {
            return None;
        }
        // This turn already chose (a resumed run, or a surface refresh
        // before the first model call): serve that choice.
        if history
            .entries
            .iter()
            .any(|entry| entry.effective_from.turn_id == run_context.turn_id)
        {
            return None;
        }
        let activity = match self
            .thread_service
            .read_tool_selection_activity(scope, &run_context.thread_id)
            .await
        {
            Ok(Some(activity)) => activity,
            Ok(None) => return None,
            Err(error) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "tool selection activity read failed; keeping the selection in force"
                );
                return None;
            }
        };
        let mark = activity.last_model_call.as_ref()?;
        // The model was already called this turn: the cache is warm.
        if mark.turn_id == run_context.turn_id {
            return None;
        }
        let profile = self
            .cache_profiles
            .as_ref()
            .map(|profiles| profiles.prompt_cache_profile(run_context))
            .unwrap_or_default();
        if let (Some(previous), Some(current)) = (mark.model.as_deref(), profile.model.as_deref())
            && previous != current
        {
            return Some((ToolSelectionReason::ModelChange, activity));
        }
        let idle = Utc::now()
            .signed_duration_since(mark.called_at)
            .to_std()
            .unwrap_or_default();
        let lifetime = match profile.lifetime {
            PromptCacheLifetime::Disabled => {
                return Some((ToolSelectionReason::CacheCold, activity));
            }
            PromptCacheLifetime::Known(lifetime) => lifetime,
            PromptCacheLifetime::Unknown => config.cache_lifetime(),
        };
        (idle > lifetime.saturating_add(config.cache_margin()))
            .then_some((ToolSelectionReason::CacheCold, activity))
    }

    /// Choose the conversation's tools again and record the result with
    /// `reason`. `None` keeps the entry in force: the result is unchanged,
    /// the classifier failed, or there is no conversation text to rank.
    pub(super) async fn reselect(
        &self,
        reselection: &Reselection<'_>,
        reason: ToolSelectionReason,
        activity: Option<ToolSelectionActivity>,
    ) -> Option<Served> {
        let surface = reselection.surface;
        let run_context = surface.run_context;
        let candidates = reselection.candidates;
        if candidates.is_empty() {
            return None;
        }
        let activity = match activity {
            Some(activity) => Some(activity),
            None => self
                .thread_service
                .read_tool_selection_activity(reselection.scope, &run_context.thread_id)
                .await
                .ok()
                .flatten(),
        };
        let called_tools = activity
            .map(|activity| activity.called_tools)
            .unwrap_or_default();
        let (context, message_sequence) = self
            .conversation_window(run_context, reselection.scope, reselection.history)
            .await?;
        let floor = selection_floor(
            &self.config.always,
            candidates,
            advertised_bridge_tokens(surface.catalog, surface.policy, surface.mode),
        );
        // Sticky tools: every tool the conversation called that is still a
        // candidate, counted first against the caps after the floor.
        let mut pinned = floor.names.clone();
        let mut reserved_tools = floor.reserved_tools;
        let mut reserved_tokens = floor.reserved_tokens;
        for name in &called_tools {
            if pinned.contains(name) || !reselection.admitted.contains(name) {
                continue;
            }
            let Some(tokens) = candidates
                .iter()
                .find(|(definition, _)| definition.name.as_str() == name)
                .map(|(_, tokens)| *tokens)
            else {
                continue;
            };
            if reserved_tools >= self.config.max_tools
                || reserved_tokens.saturating_add(tokens) > self.config.token_budget
            {
                break;
            }
            pinned.push(name.clone());
            reserved_tools += 1;
            reserved_tokens = reserved_tokens.saturating_add(tokens);
        }
        let sticky_count = pinned.len() - floor.names.len();
        let request = ToolSelectionRequest {
            context,
            called_tools,
            candidates: candidates
                .iter()
                .map(|(definition, tokens)| ToolSelectionCandidate {
                    definition: (*definition).clone(),
                    est_schema_tokens: *tokens,
                })
                .collect(),
            pinned,
            max_tools: self.config.max_tools,
            token_budget: self.config.token_budget,
            reserved_tools,
            reserved_tokens,
        };
        let classifier = self.classifier.get();
        let started = std::time::Instant::now();
        let outcome = classifier.classify(&request).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let selection = match outcome {
            Ok(selection) => selection,
            Err(error) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    classifier = classifier.classifier_name(),
                    reason = reason.as_str(),
                    error_kind = error.kind_label(),
                    latency_ms,
                    "tool re-selection failed; keeping the selection in force"
                );
                return None;
            }
        };
        let chosen = accept_chosen(&request, selection.chosen);
        let advertised = advertised_names(
            &request.pinned,
            chosen.iter().map(|tool| tool.name.as_str()),
        );
        if advertised == reselection.entry.advertised {
            debug!(
                target: TOOL_PREFETCH_LOG_TARGET,
                classifier = classifier.classifier_name(),
                reason = reason.as_str(),
                advertised_tool_count = advertised.len(),
                latency_ms,
                "tool re-selection chose the selection in force; nothing recorded"
            );
            return None;
        }
        let advertised_set: BTreeSet<String> = advertised.iter().cloned().collect();
        let entry = ToolSelectionEntry {
            effective_from: ToolSelectionEffectiveFrom {
                turn_id: run_context.turn_id,
                message_sequence,
            },
            reason,
            advertised,
            scores: chosen
                .into_iter()
                .map(|tool| ToolSelectionScore {
                    name: tool.name,
                    score: tool.score,
                })
                .collect(),
            ranker_version: Some(selection.scorer).filter(|scorer| valid_scorer(scorer)),
            tool_search_description: Some(tool_search_description_excluding(
                surface.catalog,
                surface.policy,
                surface.mode,
                &advertised_set,
            )),
            fallback_reason: None,
            recorded_at: Utc::now(),
        };
        let previous: BTreeSet<&str> = reselection
            .entry
            .advertised
            .iter()
            .map(String::as_str)
            .collect();
        let added: Vec<&str> = entry
            .advertised
            .iter()
            .map(String::as_str)
            .filter(|name| !previous.contains(name))
            .collect();
        let removed: Vec<&str> = previous
            .iter()
            .copied()
            .filter(|name| !advertised_set.contains(*name))
            .collect();
        debug!(
            target: TOOL_PREFETCH_LOG_TARGET,
            classifier = classifier.classifier_name(),
            reason = reason.as_str(),
            advertised_tool_count = entry.advertised.len(),
            sticky_count,
            added = ?added,
            removed = ?removed,
            latency_ms,
            "re-selected the conversation's tools from the conversation so far"
        );
        match self
            .thread_service
            .append_tool_selection_entry(AppendToolSelectionEntryRequest {
                scope: reselection.scope.clone(),
                thread_id: run_context.thread_id.clone(),
                expected_entries: reselection.history.entries.len(),
                entry: entry.clone(),
            })
            .await
        {
            Ok(_) => {}
            Err(SessionThreadError::ToolSelectionHistoryConflict { .. }) => {
                return Some(Served::LostRace);
            }
            Err(error) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "recording the tool re-selection failed; serving it unrecorded"
                );
            }
        }
        Some(Served::Active(
            frozen_active_set(
                surface.catalog,
                &entry.advertised,
                entry.tool_search_description.as_deref(),
                reselection.admitted,
            )
            .active,
        ))
    }

    /// The conversation window a re-selection ranks against, and the
    /// thread sequence of the run's accepted message. `None` when the run
    /// has no accepted user message to anchor the window, or no text.
    async fn conversation_window(
        &self,
        run_context: &LoopRunContext,
        scope: &ThreadScope,
        history: &ToolSelectionHistory,
    ) -> Option<(ConversationContext, Option<u64>)> {
        let Some(message_id) = accepted_task_message_id(run_context) else {
            debug!(
                target: TOOL_PREFETCH_LOG_TARGET,
                "run has no accepted user message; keeping the selection in force"
            );
            return None;
        };
        let anchor = match self
            .thread_service
            .read_thread_message(scope, &run_context.thread_id, message_id)
            .await
        {
            Ok(Some(record)) => record.sequence,
            Ok(None) => return None,
            Err(error) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "accepted user message read failed; keeping the selection in force"
                );
                return None;
            }
        };
        let after = anchor.saturating_sub(WINDOW_SCAN_MESSAGES);
        let recent = self.user_texts(scope, run_context, after, anchor).await?;
        // The opening message, when it is older than the scanned rows.
        let opening_sequence = history
            .entries
            .first()
            .and_then(|entry| entry.effective_from.message_sequence)
            .filter(|sequence| *sequence <= after);
        let opening = match opening_sequence {
            Some(sequence) => self
                .user_texts(scope, run_context, sequence.saturating_sub(1), sequence)
                .await
                .and_then(|texts| texts.into_iter().next()),
            None => None,
        };
        let context = super::conversation::window(
            recent.iter().rev().map(String::as_str),
            opening.as_deref(),
            self.config.context_messages,
        );
        if context.is_empty() {
            return None;
        }
        Some((context, Some(anchor)))
    }

    /// The text of the user messages in `(after, through]`, oldest first.
    async fn user_texts(
        &self,
        scope: &ThreadScope,
        run_context: &LoopRunContext,
        after: u64,
        through: u64,
    ) -> Option<Vec<String>> {
        match self
            .thread_service
            .list_thread_messages_range(ThreadMessageRangeRequest {
                scope: scope.clone(),
                thread_id: run_context.thread_id.clone(),
                after_sequence: after,
                through_sequence: through,
            })
            .await
        {
            Ok(range) => Some(
                range
                    .messages
                    .into_iter()
                    .filter(|message| {
                        message.kind == MessageKind::User
                            && !matches!(
                                message.status,
                                MessageStatus::RejectedBusy
                                    | MessageStatus::Redacted
                                    | MessageStatus::Deleted
                            )
                    })
                    .filter_map(|message| message.content)
                    .collect(),
            ),
            Err(error) => {
                debug!(
                    target: TOOL_PREFETCH_LOG_TARGET,
                    error_kind = error.kind_name(),
                    "conversation read failed; keeping the selection in force"
                );
                None
            }
        }
    }

    /// Record that the conversation called `tool_name` successfully, so a
    /// later re-selection keeps it. Best effort, and only when re-selection
    /// is on.
    pub(crate) async fn record_called_tool(&self, run_context: &LoopRunContext, tool_name: String) {
        if !self.config.reselection.enabled() {
            return;
        }
        let scope = ThreadScopeResolver::resolve_for_turn(
            &self.thread_scope,
            &run_context.scope,
            run_context.actor(),
        );
        if let Err(error) = self
            .thread_service
            .record_tool_selection_activity(ironclaw_threads::RecordToolSelectionActivityRequest {
                scope,
                thread_id: run_context.thread_id.clone(),
                update: ironclaw_threads::ToolSelectionActivityUpdate::CalledTool(tool_name),
            })
            .await
        {
            debug!(
                target: TOOL_PREFETCH_LOG_TARGET,
                error_kind = error.kind_name(),
                "recording a called tool for re-selection failed"
            );
        }
    }
}
