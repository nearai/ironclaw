//! Request bodies: the `state`, one `noul` question per tool, and the split of
//! a large catalog into slices that each fit the model's context limits.

use ironclaw_loop_contracts::{ConversationContext, ToolSelectionCandidate};
use serde_json::{Map, Value, json};

/// Bytes per estimated token. JSON punctuation and identifiers tokenize
/// worse than prose, so this errs towards more tokens (smaller slices).
const BYTES_PER_TOKEN: usize = 3;

/// Tokens set aside in every slice for the top-level JSON shape, the model
/// name and the question type fields.
const SLICE_OVERHEAD_TOKENS: usize = 256;

/// The question asked of each tool. The backticked names point at fields of
/// the `state`, which is how Jev questions refer to structured state.
pub(crate) fn instructions_for(name: &str) -> String {
    format!("How likely is it that `tools.{name}` will be used in the following `conversation`?")
}

fn estimate_tokens(bytes: usize) -> usize {
    bytes.div_ceil(BYTES_PER_TOKEN)
}

/// One tool's `state.tools` entry.
fn tool_state(candidate: &ToolSelectionCandidate) -> Value {
    json!({
        "description": candidate.description(),
        "parameters": candidate.parameter_names(),
    })
}

fn question(name: &str) -> Value {
    json!({"type": "noul", "instructions": instructions_for(name)})
}

/// The `state.conversation` list: the context's user messages, oldest
/// first, as many of the newest as fit in half of `max_slice_tokens` (the
/// other half is left for tools). Jev scores the whole conversation at once,
/// so it gets the whole window rather than the local rankers' segments; only
/// a slice budget configured far below the default ever drops a message.
/// The newest message is always sent, cut to fit when it alone is larger.
fn conversation(context: &ConversationContext, max_slice_tokens: usize) -> Value {
    let max_bytes = (max_slice_tokens / 2).saturating_mul(BYTES_PER_TOKEN);
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0_usize;
    for message in context.user_messages().iter().rev() {
        // Each message also costs its quotes and comma in the JSON list.
        let cost = message.len().saturating_add(3);
        if used.saturating_add(cost) <= max_bytes {
            used += cost;
            kept.push(message);
        } else {
            if kept.is_empty() {
                kept.push(truncate_to_char_boundary(message, max_bytes));
            }
            break;
        }
    }
    kept.reverse();
    Value::from(kept)
}

fn truncate_to_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Split the candidates into slices, each a run of consecutive candidate
/// indices in catalog order, so that every slice's estimated tokens
/// (conversation, see [`conversation`], tool entries and questions) stay within
/// `max_slice_tokens`. A tool too large for a slice of its own still gets
/// one. The split depends only on the request, so it is the same on every
/// run.
pub(crate) fn plan_slices(
    context: &ConversationContext,
    candidates: &[ToolSelectionCandidate],
    max_slice_tokens: usize,
) -> Vec<Vec<usize>> {
    let fixed = SLICE_OVERHEAD_TOKENS
        + estimate_tokens(conversation(context, max_slice_tokens).to_string().len());
    let budget = max_slice_tokens.saturating_sub(fixed);
    let mut slices: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut used = 0_usize;
    for (index, candidate) in candidates.iter().enumerate() {
        let name = candidate.name();
        // The name appears twice: as the `tools` key and as the question id.
        let cost = estimate_tokens(
            tool_state(candidate).to_string().len()
                + question(name).to_string().len()
                + 2 * name.len(),
        );
        if !current.is_empty() && used.saturating_add(cost) > budget {
            slices.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(index);
        used = used.saturating_add(cost);
    }
    if !current.is_empty() {
        slices.push(current);
    }
    slices
}

/// The request body for one slice. Tools are keyed by name; the map is
/// ordered by key, so the body is the same on every run.
pub(crate) fn slice_body(
    model: &str,
    context: &ConversationContext,
    max_slice_tokens: usize,
    candidates: &[ToolSelectionCandidate],
    slice: &[usize],
) -> Value {
    let mut tools = Map::new();
    let mut questions = Map::new();
    for candidate in slice.iter().filter_map(|index| candidates.get(*index)) {
        tools.insert(candidate.name().to_string(), tool_state(candidate));
        questions.insert(candidate.name().to_string(), question(candidate.name()));
    }
    json!({
        "model": model,
        "state": {
            "conversation": conversation(context, max_slice_tokens),
            "tools": tools,
        },
        "questions": questions,
    })
}
