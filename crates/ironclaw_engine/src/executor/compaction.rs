//! Context compaction and token counting.
//!
//! When message history approaches the model's context limit, compaction
//! asks the LLM to summarize progress and resets the history. This follows
//! the official RLM pattern (compaction at 85% of context limit).

use std::sync::Arc;

use tracing::debug;

use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::types::error::EngineError;
use crate::types::message::{MessageRole, ThreadMessage};
use crate::types::step::{LlmResponse, TokenUsage};

/// Characters per token estimate when no tokenizer is available.
/// Conservative estimate (official RLM uses 4).
const CHARS_PER_TOKEN: usize = 4;

/// Estimate token count for a list of messages.
///
/// Uses character length / `CHARS_PER_TOKEN` as a rough estimate.
/// The official RLM uses tiktoken when available; we use this fallback
/// since we don't depend on a Python tokenizer.
pub fn estimate_tokens(messages: &[ThreadMessage]) -> usize {
    let total_chars: usize = messages
        .iter()
        .map(|m| {
            m.content.len()
                + m.action_name.as_ref().map_or(0, |n| n.len())
                + 4 // overhead per message (role token, delimiters)
        })
        .sum();
    total_chars.div_ceil(CHARS_PER_TOKEN)
}

/// Check if compaction should be triggered.
///
/// Returns `true` when estimated token count exceeds `threshold_pct` of
/// the model's context limit.
pub fn should_compact(
    messages: &[ThreadMessage],
    model_context_limit: usize,
    threshold_pct: f64,
) -> bool {
    let tokens = estimate_tokens(messages);
    let threshold = (model_context_limit as f64 * threshold_pct) as usize;
    tokens >= threshold
}

/// The compaction prompt sent to the LLM.
const COMPACTION_PROMPT: &str = "\
Summarize your progress so far in a concise but complete way. Include:
1. What you have accomplished
2. Key intermediate results and variable values
3. What still needs to be done
4. Any errors encountered and how they were handled

Preserve all information needed to continue the task. Be specific about data values.";

/// Compact the message history by asking the LLM to summarize.
///
/// Returns the new (shorter) message list and the token usage from the
/// summarization call. The original messages are replaced with:
/// `[system_prompt, summary, continuation_note]`
///
/// The full original messages are returned separately so the caller can
/// store them (e.g., in a `history` variable or event log).
pub async fn compact_messages(
    messages: &[ThreadMessage],
    llm: &Arc<dyn LlmBackend>,
    compaction_count: u32,
) -> Result<CompactionResult, EngineError> {
    // Build a summarization request from existing messages + prompt
    let mut summarize_messages = messages.to_vec();
    summarize_messages.push(ThreadMessage::user(COMPACTION_PROMPT.to_string()));

    let config = LlmCallConfig {
        force_text: true,
        ..LlmCallConfig::default()
    };

    let output = llm.complete(&summarize_messages, &[], &config).await?;

    let summary_text = match output.response {
        LlmResponse::Text(t) => t,
        LlmResponse::ActionCalls { content, .. } | LlmResponse::Code { content, .. } => {
            content.unwrap_or_else(|| "[compaction produced no summary]".into())
        }
    };

    // Preserve the system prompt (first message if it's a system message)
    let system_msg = messages
        .iter()
        .find(|m| m.role == MessageRole::System)
        .cloned();

    // Build compacted history
    let mut compacted = Vec::new();
    if let Some(sys) = system_msg {
        compacted.push(sys);
    }
    compacted.push(ThreadMessage::assistant(summary_text.clone()));
    compacted.push(ThreadMessage::system(format!(
        "Your conversation has been compacted {n} time(s). \
         The summary above captures your progress. Continue working on the task.",
        n = compaction_count + 1,
    )));

    let tokens_before = estimate_tokens(messages);
    let tokens_after = estimate_tokens(&compacted);

    debug!(
        tokens_before,
        tokens_after,
        compaction_count = compaction_count + 1,
        "context compacted"
    );

    Ok(CompactionResult {
        compacted_messages: compacted,
        summary: summary_text,
        tokens_used: output.usage,
        tokens_before,
        tokens_after,
    })
}

/// Result of a compaction operation.
pub struct CompactionResult {
    /// The new (shorter) message list.
    pub compacted_messages: Vec<ThreadMessage>,
    /// The summary text produced by the LLM.
    pub summary: String,
    /// Tokens used by the summarization LLM call.
    pub tokens_used: TokenUsage,
    /// Estimated token count before compaction.
    pub tokens_before: usize,
    /// Estimated token count after compaction.
    pub tokens_after: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_empty() {
        assert_eq!(estimate_tokens(&[]), 0);
    }

    #[test]
    fn estimate_tokens_basic() {
        let msgs = vec![
            ThreadMessage::system("Hello world"), // 11 chars + 4 overhead = 15 / 4 = 3.75
            ThreadMessage::user("Hi"),            // 2 chars + 4 = 6 / 4 = 1.5
        ];
        let tokens = estimate_tokens(&msgs);
        // (11+4 + 2+4) / 4 = 21/4 = 5.25 → 6 (ceiling)
        assert!(tokens > 0);
        assert!(tokens < 100);
    }

    #[test]
    fn should_compact_below_threshold() {
        let msgs = vec![ThreadMessage::user("short message")];
        assert!(!should_compact(&msgs, 128_000, 0.85));
    }

    #[test]
    fn should_compact_above_threshold() {
        // Create a message large enough to trigger compaction at low limit
        let big = "x".repeat(1000);
        let msgs = vec![ThreadMessage::user(big)];
        // 1000 chars / 4 = 250 tokens. Context limit 200, threshold 85% = 170
        assert!(should_compact(&msgs, 200, 0.85));
    }
}
