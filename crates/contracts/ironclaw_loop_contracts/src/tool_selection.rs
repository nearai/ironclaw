//! Tool selection port for the agent loop host.
//!
//! Turn-start tool selection decides which deferred tools a conversation
//! advertises in its request's `tools` array, beside the core tools and the
//! discovery bridges that progressive disclosure always advertises. The loop
//! host owns everything around that decision: which tools are candidates (the
//! authorized tools that are not already advertised), the bounds on the
//! conversation text, the durable record of the result, and what happens when
//! a selection fails. The decision itself, "which of these candidates", is
//! this port: [`ToolSelectionClassifier`].
//!
//! # Output is untrusted
//!
//! A classifier cannot grant authority. The host drops every returned name
//! that is not a candidate, duplicates and invalid scores, and leaves out any
//! tool past [`ToolSelectionRequest::max_tools`] or
//! [`ToolSelectionRequest::token_budget`].
//!
//! # When a classifier runs
//!
//! Once per conversation: before the model call of the first turn that has
//! user text and no recorded selection. The result is recorded with the
//! conversation, and every later turn rebuilds the same list from that record
//! without calling the classifier, because the `tools` array is part of the
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

/// The most conversation text a selection may see, in bytes, over all of its
/// messages. The host keeps every [`ConversationContext`] it builds within
/// it. [`ConversationContext::new`] does not check it, so a classifier that
/// forwards the text and is handed a context built anywhere else must bound
/// what it sends itself.
pub const MAX_CONVERSATION_CONTEXT_BYTES: usize = 16 * 1_024;

/// The user messages a selection may see, oldest first.
///
/// The host builds it from the turn's accepted user message, cut to
/// [`MAX_CONVERSATION_CONTEXT_BYTES`]. This type only carries the result. It
/// is never logged: `Debug` prints only message and byte counts.
#[derive(Clone, PartialEq, Eq)]
pub struct ConversationContext {
    user_messages: Vec<String>,
}

impl ConversationContext {
    /// A context holding `user_messages`, oldest first. Unchecked: the
    /// bounds are the builder's (see [`MAX_CONVERSATION_CONTEXT_BYTES`]).
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

/// One tool a classifier may choose: an authorized definition and the host's
/// estimate of the tokens its schema costs in the `tools` array.
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
    /// Every tool that may be chosen, in catalog order (which is the
    /// tie-break order).
    pub candidates: Vec<ToolSelectionCandidate>,
    /// Most tools the classifier may choose.
    pub max_tools: usize,
    /// Most estimated schema tokens the chosen tools may add up to.
    pub token_budget: u32,
}

impl fmt::Debug for ToolSelectionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolSelectionRequest")
            .field("context", &self.context)
            .field("candidates", &self.candidates.len())
            .field("max_tools", &self.max_tools)
            .field("token_budget", &self.token_budget)
            .finish()
    }
}

/// One tool a classifier chose, and the score it was chosen on.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenTool {
    /// Provider tool name of a candidate.
    pub name: String,
    /// Finite, non-negative score on the classifier's scale (a probability,
    /// say), recorded with the selection and logged at `debug!`.
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
    /// Stable identifier of the scale `chosen` scores are on (a model id,
    /// say). Recorded with the selection.
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
    /// credential cannot pay for it (HTTP `402`).
    #[error("tool selection was refused for lack of payment")]
    PaymentRequired,
    /// The backend refused the request itself (for example as invalid).
    #[error("tool selection request was refused with status {status}")]
    Rejected { status: u16 },
    /// The backend asked the classifier to back off.
    #[error("tool selection was rate limited")]
    RateLimited,
    /// The backend answered with output the classifier could not use.
    #[error("tool selection returned invalid output: {reason}")]
    InvalidOutput { reason: LoopSafeSummary },
}

impl ToolSelectionError {
    /// Stable label for the failure kind, for logs and for the recorded
    /// selection's fallback reason. Lowercase `snake_case`.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unavailable",
            Self::Timeout { .. } => "timeout",
            Self::Unauthorized => "unauthorized",
            Self::PaymentRequired => "payment_required",
            Self::Rejected { .. } => "rejected",
            Self::RateLimited => "rate_limited",
            Self::InvalidOutput { .. } => "invalid_output",
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
    /// `"jev"`).
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
