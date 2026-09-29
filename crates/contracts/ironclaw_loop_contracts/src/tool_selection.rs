//! Tool selection port for the agent loop host.
//!
//! Turn-start tool selection decides which tools a conversation advertises in
//! its request's `tools` array. The loop host owns everything around that
//! decision: which tools are candidates (the authorized, available catalog),
//! the always-advertised floor, the bounds on the conversation text, the
//! durable selection history, and what happens when a selection fails. The
//! decision itself, "which of these candidates", is this port:
//! [`ToolSelectionClassifier`].
//!
//! The host-bundled implementation ranks the candidates with the bound
//! [`crate::ToolRetrievalProvider`] once per conversation segment and merges
//! the rankings by rank. A deployment may bind a different classifier (a hosted
//! classification model, say) without the loop host naming it. Exactly one
//! classifier is bound per deployment; they are never combined.
//!
//! # Output is untrusted
//!
//! A classifier cannot grant authority or remove the floor. The host drops
//! every returned name that is not a candidate, every pinned name (the host
//! advertises those itself), duplicates and invalid scores, and stops adding
//! tools once [`ToolSelectionRequest::max_tools`] or
//! [`ToolSelectionRequest::token_budget`] is reached. It then adds the floor.
//!
//! # When a classifier runs
//!
//! Before the first model call of a conversation's first turn, from the
//! opening request. After that, only at a turn boundary where the provider's
//! prompt cache can no longer be warm (the conversation sat idle past the
//! cache lifetime, or the model changed) or when a selected tool was
//! revoked; the context is then a bounded window of the conversation's user
//! messages, and `called_tools` lists the tools the conversation has called.
//! Every result is recorded in the conversation's selection history, and
//! every other turn, resume and replay rebuilds the same `tools` array from
//! that record, never calling the classifier: the array is part of the
//! provider's cached prompt prefix.
//!
//! # Confidentiality contract
//!
//! The request carries user text ([`ConversationContext`]) and tool
//! descriptions. Implementations must not log either, and a
//! [`ToolSelectionError`] must never carry them. `Debug` on the request and
//! the context prints sizes only. A classifier that sends the request to a
//! third party must say so in its operator documentation.
//!
//! # Determinism contract
//!
//! For the same request a classifier should return the same selection, ties
//! broken by candidate (catalog) order. A remote model may not be exactly
//! reproducible; that is acceptable because the host records the result
//! instead of recomputing it.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use crate::host::{LoopSafeSummary, ProviderToolDefinition};
use crate::tool_retrieval::ToolRetrievalError;

/// The most conversation text any selection may see, in bytes, over all of
/// its messages. The host builds every [`ConversationContext`] within it, so
/// a classifier that forwards the text (to a hosted model, say) can rely on
/// the bound.
pub const MAX_CONVERSATION_CONTEXT_BYTES: usize = 32 * 1_024;

/// The user messages a selection may see, oldest first.
///
/// At a conversation's first turn it holds only the first accepted user
/// message; a later re-selection passes a bounded window of the conversation.
/// The host (`ironclaw_loop_host`) builds it: it decides which messages go
/// in, cuts them to its bounds, drops blank ones, and keeps the total within
/// [`MAX_CONVERSATION_CONTEXT_BYTES`]. This type only carries the result. It
/// is never logged: `Debug` prints only message and byte counts.
#[derive(Clone, PartialEq, Eq)]
pub struct ConversationContext {
    user_messages: Vec<String>,
}

impl ConversationContext {
    /// A context holding `user_messages`, oldest first.
    pub fn new(user_messages: Vec<String>) -> Self {
        Self { user_messages }
    }

    /// The user messages, oldest first.
    pub fn user_messages(&self) -> &[String] {
        &self.user_messages
    }

    /// Whether the context holds no message.
    pub fn is_empty(&self) -> bool {
        self.user_messages.is_empty()
    }
}

impl fmt::Debug for ConversationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationContext")
            .field("messages", &self.user_messages.len())
            .field(
                "bytes",
                &self.user_messages.iter().map(String::len).sum::<usize>(),
            )
            .finish()
    }
}

/// One tool a classifier may choose: an authorized, available definition and
/// the host's estimate of the tokens its schema costs in the `tools` array.
#[derive(Clone, PartialEq)]
pub struct ToolSelectionCandidate {
    pub definition: ProviderToolDefinition,
    pub est_schema_tokens: u32,
}

impl ToolSelectionCandidate {
    /// The provider tool name the classifier answers with.
    pub fn name(&self) -> &str {
        self.definition.name.as_str()
    }

    /// The provider-safe description.
    pub fn description(&self) -> &str {
        &self.definition.description
    }

    /// Top-level parameter names from the tool's JSON schema, in schema
    /// order; empty when the schema declares none.
    pub fn parameter_names(&self) -> Vec<String> {
        self.definition
            .parameters
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .map(|properties| properties.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl fmt::Debug for ToolSelectionCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolSelectionCandidate")
            .field("name", &self.name())
            .field("est_schema_tokens", &self.est_schema_tokens)
            .finish_non_exhaustive()
    }
}

/// Everything one selection decides from.
#[derive(Clone, PartialEq)]
pub struct ToolSelectionRequest {
    /// The conversation text the selection is for.
    pub context: ConversationContext,
    /// Tools the conversation has already called, oldest first. Empty at a
    /// conversation's first turn.
    pub called_tools: Vec<String>,
    /// Every tool that may be chosen, in catalog order (which is the
    /// tie-break order). Pinned tools are included, so a classifier that
    /// scores the whole catalog sees them, but choosing one adds nothing.
    pub candidates: Vec<ToolSelectionCandidate>,
    /// Candidate names the host advertises whatever the classifier returns:
    /// the always-on floor and the operator's extras.
    pub pinned: Vec<String>,
    /// Most tools the `tools` array may hold, pinned tools and the host's
    /// discovery bridges included.
    pub max_tools: usize,
    /// Most estimated schema tokens the `tools` array may add up to, pinned
    /// tools and bridges included.
    pub token_budget: u32,
    /// How many of `max_tools` the pinned tools and bridges already take.
    pub reserved_tools: usize,
    /// How many of `token_budget` the pinned tools and bridges already take.
    pub reserved_tokens: u32,
}

impl ToolSelectionRequest {
    /// How many tools a classifier may choose.
    pub fn selectable_tools(&self) -> usize {
        self.max_tools.saturating_sub(self.reserved_tools)
    }

    /// How many estimated schema tokens the chosen tools may add up to.
    pub fn selectable_tokens(&self) -> u32 {
        self.token_budget.saturating_sub(self.reserved_tokens)
    }

    /// Whether the host advertises `name` whatever the classifier returns.
    pub fn is_pinned(&self, name: &str) -> bool {
        self.pinned.iter().any(|pinned| pinned == name)
    }
}

impl fmt::Debug for ToolSelectionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolSelectionRequest")
            .field("context", &self.context)
            .field("called_tools", &self.called_tools.len())
            .field("candidates", &self.candidates.len())
            .field("pinned", &self.pinned.len())
            .field("max_tools", &self.max_tools)
            .field("token_budget", &self.token_budget)
            .field("reserved_tools", &self.reserved_tools)
            .field("reserved_tokens", &self.reserved_tokens)
            .finish()
    }
}

/// One tool a classifier chose, and the score it was chosen on.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenTool {
    /// Provider tool name of a candidate.
    pub name: String,
    /// Finite, non-negative score on the classifier's scale (a ranker score,
    /// or a probability), recorded with the selection and logged at `debug!`.
    pub score: f32,
}

impl ChosenTool {
    pub fn new(name: impl Into<String>, score: f32) -> Self {
        Self {
            name: name.into(),
            score,
        }
    }
}

/// A classifier's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSelection {
    /// The chosen tools, best first. The host keeps them in this order until
    /// `max_tools` or the token budget is reached.
    pub chosen: Vec<ChosenTool>,
    /// Stable identifier of the scale `chosen` scores are on (a ranker
    /// version, or a model id). Recorded with the selection.
    pub scorer: String,
}

/// Why a selection failed.
///
/// Every variant is safe to record: none carries the conversation text, any
/// tool description or any credential, and an implementation must not
/// smuggle them into a `reason`.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ToolSelectionError {
    /// The classifier's backend could not be reached or is not ready.
    #[error("tool selection is unavailable: {reason}")]
    Unavailable { reason: LoopSafeSummary },
    /// The classifier gave up after its own time bound.
    #[error("tool selection timed out after {elapsed:?}")]
    Timeout { elapsed: Duration },
    /// The backend refused the classifier's credential.
    #[error("tool selection credential was refused")]
    Unauthorized,
    /// The backend refused the request because the account behind the
    /// credential cannot pay for it (HTTP `402`). Not retried.
    #[error("tool selection was refused for lack of payment")]
    PaymentRequired,
    /// The backend refused the request itself (for example as invalid).
    #[error("tool selection request was refused with status {status}")]
    Rejected { status: u16 },
    /// The backend kept asking the classifier to back off.
    #[error("tool selection was rate limited")]
    RateLimited,
    /// The backend answered with output the classifier could not use.
    #[error("tool selection returned invalid output: {reason}")]
    InvalidOutput { reason: LoopSafeSummary },
    /// The ranker a ranking classifier uses failed.
    #[error(transparent)]
    Ranking(#[from] ToolRetrievalError),
}

impl ToolSelectionError {
    /// Stable label for the failure kind, for logs and for the selection
    /// history's fallback reason. Lowercase `snake_case`.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unavailable",
            Self::Timeout { .. } => "timeout",
            Self::Unauthorized => "unauthorized",
            Self::PaymentRequired => "payment_required",
            Self::Rejected { .. } => "rejected",
            Self::RateLimited => "rate_limited",
            Self::InvalidOutput { .. } => "invalid_output",
            Self::Ranking(_) => "ranking_failed",
        }
    }
}

/// Chooses which candidates a conversation advertises.
///
/// One classifier is bound per deployment and reused across conversations.
/// It runs inside a turn, before the model is called, so it must bound any
/// network or model I/O it performs.
#[async_trait]
pub trait ToolSelectionClassifier: Send + Sync + fmt::Debug {
    /// Stable, non-identifying name of the classifier for logs (for example
    /// `"local"`).
    fn classifier_name(&self) -> &str;

    /// Choose tools for `request`. See the module docs for what the host
    /// does with the answer.
    async fn classify(
        &self,
        request: &ToolSelectionRequest,
    ) -> Result<ToolSelection, ToolSelectionError>;
}

#[cfg(test)]
mod tests;
