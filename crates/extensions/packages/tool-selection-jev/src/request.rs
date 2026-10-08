//! Request bodies: the `state`, one `noul` question per tool, and the split of
//! a large catalog into slices that each fit the decisions API's two token
//! limits.
//!
//! Each tool's description rides in its own question, not in `state`. The
//! API counts `state` against both of its limits (once with the longest
//! question, once with every question), so a description in `state` would
//! be counted in the tighter budget; in its question it is counted there
//! only when it is the longest one. That roughly doubles the tools a slice
//! holds.

use ironclaw_loop_contracts::{
    ConversationContext, MAX_CONVERSATION_CONTEXT_BYTES, ToolSelectionCandidate,
};
use serde_json::{Map, Value, json};

/// Most estimated tokens one request's `state` (overhead, conversation and
/// tool entries) plus its single longest question may carry. The decisions
/// API's published limit is 32,000 tokens; this keeps a margin below it,
/// since the estimate is only an estimate.
pub(crate) const DEFAULT_MAX_STATE_AND_QUESTION_TOKENS: usize = 30_000;

/// Most estimated tokens one whole request (`state` plus every question) may
/// carry. The decisions API's published limit is 64,000 tokens; this keeps
/// a margin below it.
pub(crate) const DEFAULT_MAX_REQUEST_TOKENS: usize = 60_000;

/// Bytes per estimated token. JSON punctuation and identifiers tokenize
/// worse than prose, so this errs towards more tokens (smaller slices);
/// measured requests came to about 3.8 to 4.3 bytes a token.
const BYTES_PER_TOKEN: usize = 3;

/// Tokens set aside in every slice for the top-level JSON shape, the model
/// name, the question type fields and the API's own framing, which was
/// measured at about 215 tokens a request.
const SLICE_OVERHEAD_TOKENS: usize = 256;

/// Bytes a map key costs beyond the name itself: its quotes, the colon and
/// the comma after the entry.
const MAP_KEY_FRAMING_BYTES: usize = 4;

/// Longest description sent for one tool, in bytes; a longer one is cut on a
/// character boundary.
const MAX_DESCRIPTION_BYTES: usize = 1_024;

/// Most parameter names sent for one tool; the rest are left out.
const MAX_PARAMETERS: usize = 64;

/// Longest parameter name sent, in bytes; a longer one is cut on a
/// character boundary.
const MAX_PARAMETER_NAME_BYTES: usize = 128;

/// The decisions API's two limits, as estimated-token budgets for one
/// request (slice). The fixed per-request overhead is counted in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenLimits {
    /// `state` plus the single longest question.
    pub(crate) state_and_longest_question: usize,
    /// `state` plus every question.
    pub(crate) request: usize,
}

impl TokenLimits {
    pub(crate) const DEFAULT: Self = Self {
        state_and_longest_question: DEFAULT_MAX_STATE_AND_QUESTION_TOKENS,
        request: DEFAULT_MAX_REQUEST_TOKENS,
    };
}

fn estimate_tokens(bytes: usize) -> usize {
    bytes.div_ceil(BYTES_PER_TOKEN)
}

fn truncate_to_char_boundary(text: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// One tool's `state.tools` entry: its first [`MAX_PARAMETERS`] top-level
/// parameter names, each cut to [`MAX_PARAMETER_NAME_BYTES`], so one
/// oversized tool cannot fill a request. The description is in its question.
fn tool_state(candidate: &ToolSelectionCandidate) -> Value {
    let names: Vec<&str> = candidate
        .definition
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::keys)
        .take(MAX_PARAMETERS)
        .map(|name| truncate_to_char_boundary(name, MAX_PARAMETER_NAME_BYTES))
        .collect();
    json!({ "parameters": names })
}

/// The question asked of one tool, carrying its description (cut to
/// [`MAX_DESCRIPTION_BYTES`]). The backticked names point at fields of the
/// `state`, which is how Jev questions refer to structured state.
fn question(candidate: &ToolSelectionCandidate) -> Value {
    let name = candidate.name();
    let description = truncate_to_char_boundary(candidate.description(), MAX_DESCRIPTION_BYTES);
    json!({
        "type": "noul",
        "instructions": format!(
            "How likely is it that `tools.{name}` will be used in the following `conversation`, \
             given that it is described as: {description}?"
        ),
    })
}

/// The `state.conversation` list: the user messages, oldest first. The host
/// bounds the context it builds; a context built anywhere else is cut here
/// to the same [`MAX_CONVERSATION_CONTEXT_BYTES`] over all messages.
fn conversation(context: &ConversationContext) -> Vec<&str> {
    let mut remaining = MAX_CONVERSATION_CONTEXT_BYTES;
    let mut kept = Vec::new();
    for message in context.user_messages() {
        let cut = truncate_to_char_boundary(message, remaining);
        if !cut.is_empty() {
            kept.push(cut);
        }
        if cut.len() < message.len() {
            // Nothing after a cut message is sent, so the conversation
            // never skips one and resumes with a later one.
            break;
        }
        remaining -= cut.len();
    }
    kept
}

/// Estimated tokens every slice carries whatever its tools: the fixed
/// overhead and the conversation.
fn fixed_tokens(context: &ConversationContext) -> usize {
    SLICE_OVERHEAD_TOKENS + estimate_tokens(json!(conversation(context)).to_string().len())
}

/// A slice's running estimate: its `state.tools` entries, all of its
/// questions, and its longest question.
#[derive(Debug, Clone, Copy, Default)]
struct SliceTotals {
    state: usize,
    questions: usize,
    longest_question: usize,
}

impl SliceTotals {
    /// The totals with `candidate` added: its `state.tools` entry and its
    /// question, each with the map key it sits under.
    fn with(self, candidate: &ToolSelectionCandidate) -> Self {
        let key = candidate.name().len() + MAP_KEY_FRAMING_BYTES;
        let state = estimate_tokens(tool_state(candidate).to_string().len() + key);
        let question = estimate_tokens(question(candidate).to_string().len() + key);
        Self {
            state: self.state.saturating_add(state),
            questions: self.questions.saturating_add(question),
            longest_question: self.longest_question.max(question),
        }
    }

    /// Whether the slice, with the `fixed` tokens, is within both limits.
    fn fits(self, fixed: usize, limits: TokenLimits) -> bool {
        let state = fixed.saturating_add(self.state);
        state.saturating_add(self.longest_question) <= limits.state_and_longest_question
            && state.saturating_add(self.questions) <= limits.request
    }
}

/// Split the candidates into slices, each a run of consecutive candidate
/// indices in catalog order, so that every slice's estimate stays within
/// both limits. A tool too large for a slice of its own still gets one. The
/// split depends only on the request, so it is the same on every run.
pub(crate) fn plan_slices(
    context: &ConversationContext,
    candidates: &[ToolSelectionCandidate],
    limits: TokenLimits,
) -> Vec<Vec<usize>> {
    let fixed = fixed_tokens(context);
    let mut slices: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut totals = SliceTotals::default();
    for (index, candidate) in candidates.iter().enumerate() {
        if !current.is_empty() && !totals.with(candidate).fits(fixed, limits) {
            slices.push(std::mem::take(&mut current));
            totals = SliceTotals::default();
        }
        current.push(index);
        totals = totals.with(candidate);
    }
    if !current.is_empty() {
        slices.push(current);
    }
    slices
}

/// The request body for one slice. Tools are keyed by name; the maps are
/// ordered by key, so the body is the same on every run.
pub(crate) fn slice_body(
    model: &str,
    context: &ConversationContext,
    candidates: &[ToolSelectionCandidate],
    slice: &[usize],
) -> Value {
    let mut tools = Map::new();
    let mut questions = Map::new();
    for candidate in slice.iter().filter_map(|index| candidates.get(*index)) {
        tools.insert(candidate.name().to_string(), tool_state(candidate));
        questions.insert(candidate.name().to_string(), question(candidate));
    }
    json!({
        "model": model,
        "state": {
            "conversation": conversation(context),
            "tools": tools,
        },
        "questions": questions,
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::ids::CapabilityId;
    use ironclaw_loop_contracts::ProviderToolDefinition;

    use super::*;

    const DEFAULTS: TokenLimits = TokenLimits::DEFAULT;

    fn candidate_with(index: usize, description: &str, schema: Value) -> ToolSelectionCandidate {
        ToolSelectionCandidate {
            definition: ProviderToolDefinition::from_parts(
                CapabilityId::new(format!("demo.tool_{index:04}")).expect("id"),
                format!("tool_{index:04}__run"),
                description,
                schema,
            )
            .expect("definition"),
            est_schema_tokens: 50,
        }
    }

    fn candidate(index: usize, description: &str) -> ToolSelectionCandidate {
        let schema = json!({"type": "object", "properties": {"query": {}, "limit": {}}});
        candidate_with(index, description, schema)
    }

    /// `count` tools, each with a description of at least 1 KiB (cut to
    /// 1 KiB when sent), the largest a request carries.
    fn full_catalog(count: usize) -> Vec<ToolSelectionCandidate> {
        (0..count)
            .map(|index| candidate(index, &format!("Tool {index}. {}", "d".repeat(1_024))))
            .collect()
    }

    fn short_context() -> ConversationContext {
        ConversationContext::new(vec!["run the report".to_string()])
    }

    fn totals(candidates: &[ToolSelectionCandidate], slice: &[usize]) -> SliceTotals {
        slice.iter().fold(SliceTotals::default(), |totals, index| {
            totals.with(&candidates[*index])
        })
    }

    #[test]
    fn the_body_is_deterministic_and_its_text_is_cut_on_character_boundaries() {
        // 2-byte characters: 1 KiB of description is 512 of them.
        let long = candidate(0, &"é".repeat(1_000));
        let instructions = question(&long)["instructions"].to_string();
        assert!(instructions.contains(&format!(": {}?", "é".repeat(512))));

        let schema = json!({"properties": {"é".repeat(100): {}}});
        let entry = tool_state(&candidate_with(0, "d", schema));
        assert_eq!(entry, json!({"parameters": ["é".repeat(64)]}));
        for schema in [json!({"type": "object"}), json!({"properties": []})] {
            let entry = tool_state(&candidate_with(0, "d", schema));
            assert_eq!(entry, json!({"parameters": []}));
        }

        // The body's shape is pinned in `tests/jev_classifier.rs`; here, that
        // it is byte for byte the same on every run.
        let candidates = full_catalog(40);
        let slice: Vec<usize> = (0..40).collect();
        let bytes = || slice_body("jev-latest", &short_context(), &candidates, &slice).to_string();
        assert_eq!(bytes(), bytes());

        // An unbounded context is cut to the port's bound, oldest first.
        let context = ConversationContext::new(vec![
            "é".repeat(MAX_CONVERSATION_CONTEXT_BYTES / 2 - 1),
            "ab".to_string(),
            "dropped".to_string(),
        ]);
        let kept = conversation(&context);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[1], "ab");
        assert_eq!(
            kept.iter().map(|message| message.len()).sum::<usize>(),
            MAX_CONVERSATION_CONTEXT_BYTES
        );

        // One byte left and a 2-byte character next: nothing more is sent,
        // rather than an empty message and then the start of a later one.
        let context = ConversationContext::new(vec![
            "a".repeat(MAX_CONVERSATION_CONTEXT_BYTES - 1),
            "é".to_string(),
            "z".to_string(),
        ]);
        assert_eq!(conversation(&context).len(), 1);
    }

    /// With full descriptions in `state`, the state-and-longest-question
    /// limit capped a slice near 75 tools. In the questions they leave that
    /// limit with room to spare, and the whole-request limit decides where a
    /// slice ends, at about twice as many tools.
    #[test]
    fn slices_cover_the_catalog_in_order_within_both_limits() {
        let context = short_context();
        let fixed = fixed_tokens(&context);
        let candidates = full_catalog(1_000);
        let counts: Vec<usize> = [100, 500, 1_000]
            .into_iter()
            .map(|count| plan_slices(&context, &candidates[..count], DEFAULTS).len())
            .collect();
        assert_eq!(counts, vec![1, 4, 7]);

        let slices = plan_slices(&context, &candidates, DEFAULTS);
        assert_eq!(slices, plan_slices(&context, &candidates, DEFAULTS));
        let covered: Vec<usize> = slices.iter().flatten().copied().collect();
        assert_eq!(covered, (0..1_000).collect::<Vec<_>>());
        for slice in &slices {
            assert!(totals(&candidates, slice).fits(fixed, DEFAULTS));
        }
        // One more tool would break the whole-request limit, not the other.
        let first = &slices[0];
        assert!(first.len() >= 140, "{} tools", first.len());
        let grown = totals(&candidates, first).with(&candidates[first.len()]);
        assert!(fixed + grown.state + grown.questions > DEFAULTS.request);
        assert!(
            fixed + grown.state + grown.longest_question <= DEFAULTS.state_and_longest_question / 2
        );

        // A long conversation takes room from every slice.
        let long = ConversationContext::new(vec!["word ".repeat(3_000)]);
        assert!(plan_slices(&long, &candidates, DEFAULTS).len() > slices.len());
    }

    #[test]
    fn a_tool_too_large_for_any_slice_still_gets_its_own() {
        let limits = TokenLimits {
            state_and_longest_question: 500,
            request: 700,
        };
        let candidates = vec![
            candidate(0, "Small."),
            candidate(1, &"x".repeat(4_096)),
            candidate(2, "Small."),
            candidate(3, "Small."),
        ];
        let context = short_context();
        assert!(!totals(&candidates, &[1]).fits(fixed_tokens(&context), limits));
        assert_eq!(
            plan_slices(&context, &candidates, limits),
            vec![vec![0], vec![1], vec![2, 3]]
        );
        assert!(plan_slices(&context, &[], limits).is_empty());
    }

    /// Requests measured against a hosted decisions endpoint: body bytes and
    /// the input tokens it reported. The estimate must never come in under
    /// what the endpoint counts.
    #[test]
    fn the_estimate_never_under_counts_what_the_endpoint_reported() {
        for (bytes, reported) in [(764, 414), (2_422, 846), (14_100, 3_252), (14_450, 3_237)] {
            assert!(SLICE_OVERHEAD_TOKENS + estimate_tokens(bytes) >= reported);
        }
    }
}
