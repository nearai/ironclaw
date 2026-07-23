//! Prompt-cache continuity telemetry for the IronClaw model gateways.
//!
//! Benchmarks showed the provider KV cache hit rate collapsing from ~82% to
//! 29% past ~200 model calls per run (~3.5x input cost). This module tracks
//! per-run cache-read continuity across calls, detects each break as it
//! happens, and attributes it to cheap request-shape signals (tool surface or
//! system prompt changed). The gateway only constructs a
//! [`PromptCacheCallScope`] per request and records completed calls; all
//! thresholds, signatures, eviction, and logging live here.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use ironclaw_llm::{ChatMessage, CompletionResponse, Role, ToolCompletionResponse};
use ironclaw_turns::TurnRunId;
use tracing::debug;

/// Relative drop factor for cache-break detection: the current call must read
/// less than 95% of the previous call's cached tokens.
const PROMPT_CACHE_BREAK_RELATIVE_FACTOR: f64 = 0.95;
/// Absolute drop floor for cache-break detection: the cached-token drop must
/// exceed 10K tokens so small prefix churn on short prompts stays quiet.
const PROMPT_CACHE_BREAK_MIN_DROP_TOKENS: u64 = 10_000;
/// Bound on tracked per-run cache states so a long-lived gateway cannot
/// accumulate scope entries without limit.
const PROMPT_CACHE_MAX_TRACKED_SCOPES: usize = 1024;

/// Pure cache-break decision: previous call read a non-zero cached prefix and
/// the current call's cached read dropped by BOTH more than 5% relative and
/// more than 10K tokens absolute.
fn is_prompt_cache_break(previous_cache_read_tokens: u64, cache_read_tokens: u64) -> bool {
    if previous_cache_read_tokens == 0 {
        return false;
    }
    let relative_break = (cache_read_tokens as f64)
        < previous_cache_read_tokens as f64 * PROMPT_CACHE_BREAK_RELATIVE_FACTOR;
    let absolute_break = previous_cache_read_tokens.saturating_sub(cache_read_tokens)
        > PROMPT_CACHE_BREAK_MIN_DROP_TOKENS;
    relative_break && absolute_break
}

/// Provider-reported token usage for one completed model call, as consumed by
/// the prompt-cache activity log.
#[derive(Debug, Clone, Copy)]
pub(super) struct ModelCallCacheUsage {
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    input_tokens: u64,
}

impl ModelCallCacheUsage {
    pub(super) fn from_tool_response(response: &ToolCompletionResponse) -> Self {
        Self {
            cache_read_input_tokens: u64::from(response.cache_read_input_tokens),
            cache_creation_input_tokens: u64::from(response.cache_creation_input_tokens),
            input_tokens: u64::from(response.input_tokens),
        }
    }

    pub(super) fn from_completion_response(response: &CompletionResponse) -> Self {
        Self {
            cache_read_input_tokens: u64::from(response.cache_read_input_tokens),
            cache_creation_input_tokens: u64::from(response.cache_creation_input_tokens),
            input_tokens: u64::from(response.input_tokens),
        }
    }
}

/// Last observed call state for one run scope.
#[derive(Debug, Clone, Copy)]
struct LastCallCacheState {
    cache_read_input_tokens: u64,
    tool_definitions_hash: u64,
    system_prompt_hash: u64,
    observed_at: Instant,
}

/// Classification of one completed model call against the previous call in
/// the same run scope.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PromptCacheObservation {
    FirstCall,
    Continuity,
    Break {
        previous_cache_read_tokens: u64,
        tool_definitions_changed: bool,
        system_prompt_changed: bool,
    },
}

/// Per-run prompt-cache continuity tracker shared by all calls through one
/// gateway.
#[derive(Debug, Default)]
pub(super) struct PromptCacheActivityLog {
    scopes: Mutex<HashMap<TurnRunId, LastCallCacheState>>,
}

impl PromptCacheActivityLog {
    /// Classifies the call against the previous call in the same run scope
    /// and stores it as the new last-call state.
    fn observe_model_call(
        &self,
        run_id: TurnRunId,
        usage: ModelCallCacheUsage,
        tool_definitions_hash: u64,
        system_prompt_hash: u64,
    ) -> PromptCacheObservation {
        let mut scopes = self
            .scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let observation = match scopes.get(&run_id) {
            None => PromptCacheObservation::FirstCall,
            Some(previous)
                if is_prompt_cache_break(
                    previous.cache_read_input_tokens,
                    usage.cache_read_input_tokens,
                ) =>
            {
                PromptCacheObservation::Break {
                    previous_cache_read_tokens: previous.cache_read_input_tokens,
                    tool_definitions_changed: previous.tool_definitions_hash
                        != tool_definitions_hash,
                    system_prompt_changed: previous.system_prompt_hash != system_prompt_hash,
                }
            }
            Some(_) => PromptCacheObservation::Continuity,
        };
        let seconds_since_last_call = scopes
            .get(&run_id)
            .map(|previous| previous.observed_at.elapsed().as_secs_f64());
        if scopes.len() >= PROMPT_CACHE_MAX_TRACKED_SCOPES && !scopes.contains_key(&run_id) {
            let oldest = scopes
                .iter()
                .min_by_key(|(_, state)| state.observed_at)
                .map(|(scope_run_id, _)| *scope_run_id);
            if let Some(oldest) = oldest {
                scopes.remove(&oldest);
            }
        }
        scopes.insert(
            run_id,
            LastCallCacheState {
                cache_read_input_tokens: usage.cache_read_input_tokens,
                tool_definitions_hash,
                system_prompt_hash,
                observed_at: Instant::now(),
            },
        );
        drop(scopes);

        // Per-call cache series (bench debug capture keys off the
        // `ironclaw*` target; this module's natural target qualifies).
        debug!(
            run_id = %run_id,
            cache_read_input_tokens = usage.cache_read_input_tokens,
            cache_creation_input_tokens = usage.cache_creation_input_tokens,
            input_tokens = usage.input_tokens,
            "IronClaw model gateway prompt cache usage"
        );
        if let PromptCacheObservation::Break {
            previous_cache_read_tokens,
            tool_definitions_changed,
            system_prompt_changed,
        } = observation
        {
            // Internal diagnostics stay at debug!: info!/warn! render in the
            // REPL/TUI and would corrupt the interactive display.
            debug!(
                run_id = %run_id,
                prev_cache_read = previous_cache_read_tokens,
                cache_read = usage.cache_read_input_tokens,
                input_tokens = usage.input_tokens,
                seconds_since_last_call = seconds_since_last_call.unwrap_or(0.0),
                tool_definitions_changed,
                system_prompt_changed,
                "prompt cache break detected"
            );
        }
        observation
    }
}

/// One run's handle onto the gateway-wide [`PromptCacheActivityLog`].
#[derive(Clone)]
pub(super) struct PromptCacheCallScope {
    activity: Arc<PromptCacheActivityLog>,
    run_id: TurnRunId,
}

impl PromptCacheCallScope {
    pub(super) fn new(activity: Arc<PromptCacheActivityLog>, run_id: TurnRunId) -> Self {
        Self { activity, run_id }
    }

    pub(super) fn record(
        &self,
        usage: ModelCallCacheUsage,
        tool_definitions_hash: u64,
        system_prompt_hash: u64,
    ) {
        self.activity.observe_model_call(
            self.run_id,
            usage,
            tool_definitions_hash,
            system_prompt_hash,
        );
    }
}

/// Cheap order-sensitive signature over the advertised provider tool names.
pub(super) fn tool_definitions_cache_signature(tool_names: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tool_names.len().hash(&mut hasher);
    for name in tool_names {
        name.hash(&mut hasher);
    }
    hasher.finish()
}

/// Cheap signature over the first system message's content (0-input hash when
/// the request carries no system message).
pub(super) fn system_prompt_cache_signature(messages: &[ChatMessage]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Some(system) = messages
        .iter()
        .find(|message| matches!(message.role, Role::System))
    {
        system.content.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_usage(cache_read_input_tokens: u64) -> ModelCallCacheUsage {
        ModelCallCacheUsage {
            cache_read_input_tokens,
            cache_creation_input_tokens: 0,
            input_tokens: 1_000,
        }
    }

    #[test]
    fn prompt_cache_break_requires_both_relative_and_absolute_drop() {
        // Zero previous cached read: never a break, whatever the current value.
        assert!(!is_prompt_cache_break(0, 0));
        assert!(!is_prompt_cache_break(0, 50_000));

        // Small drop below both thresholds.
        assert!(!is_prompt_cache_break(100_000, 99_000));

        // 100% relative drop but only 1K tokens absolute: below the 10K floor.
        assert!(!is_prompt_cache_break(1_000, 0));

        // 40K tokens absolute but only 2% relative: above the 95% floor.
        assert!(!is_prompt_cache_break(2_000_000, 1_960_000));

        // Exactly at the boundaries is NOT a break (strict comparisons).
        assert!(!is_prompt_cache_break(200_000, 190_000)); // exactly 95%
        assert!(!is_prompt_cache_break(20_000, 10_000)); // exactly 10K drop

        // Genuine break: large relative and absolute drop.
        assert!(is_prompt_cache_break(200_000, 60_000));
        assert!(is_prompt_cache_break(100_000, 0));

        // Growth is never a break.
        assert!(!is_prompt_cache_break(100_000, 150_000));
    }

    #[test]
    fn prompt_cache_activity_log_classifies_first_continuity_and_break() {
        let log = PromptCacheActivityLog::default();
        let run_id = TurnRunId::new();
        let tools = tool_definitions_cache_signature(&["a".to_string()]);
        let prompt = system_prompt_cache_signature(&[ChatMessage::system("sys")]);

        assert_eq!(
            log.observe_model_call(run_id, cache_usage(0), tools, prompt),
            PromptCacheObservation::FirstCall
        );
        assert_eq!(
            log.observe_model_call(run_id, cache_usage(200_000), tools, prompt),
            PromptCacheObservation::Continuity
        );
        // Cache collapses AND the tool surface changed: break attributed to
        // the tool-definition change, not the (unchanged) system prompt.
        let changed_tools = tool_definitions_cache_signature(&["a".to_string(), "b".to_string()]);
        assert_eq!(
            log.observe_model_call(run_id, cache_usage(50_000), changed_tools, prompt),
            PromptCacheObservation::Break {
                previous_cache_read_tokens: 200_000,
                tool_definitions_changed: true,
                system_prompt_changed: false,
            }
        );
        // Cache collapses again with the tool surface now stable but the
        // system prompt changed: break attributed to the system prompt.
        let changed_prompt = system_prompt_cache_signature(&[ChatMessage::system("sys v2")]);
        assert_eq!(
            log.observe_model_call(run_id, cache_usage(10_000), changed_tools, changed_prompt),
            PromptCacheObservation::Break {
                previous_cache_read_tokens: 50_000,
                tool_definitions_changed: false,
                system_prompt_changed: true,
            }
        );
    }

    #[test]
    fn prompt_cache_activity_log_isolates_run_scopes() {
        let log = PromptCacheActivityLog::default();
        let tools = tool_definitions_cache_signature(&[]);
        let prompt = system_prompt_cache_signature(&[]);

        let first_run = TurnRunId::new();
        assert_eq!(
            log.observe_model_call(first_run, cache_usage(200_000), tools, prompt),
            PromptCacheObservation::FirstCall
        );
        // A different run with a tiny cached read is a FIRST call in its own
        // scope, not a break against the other run's 200K.
        let second_run = TurnRunId::new();
        assert_eq!(
            log.observe_model_call(second_run, cache_usage(1_000), tools, prompt),
            PromptCacheObservation::FirstCall
        );
    }
}
