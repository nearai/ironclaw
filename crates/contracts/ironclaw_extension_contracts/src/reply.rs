//! The progressive-reply vocabulary: what crosses the reply-sink seam
//! (`docs/internal/design/2026-08-31-progressive-reply-publication.md` §3–§4).
//!
//! A run's answer is published as a **desired-state document** that evolves
//! by typed [`ReplyChange`]s. Every channel that declares `[channel.reply]`
//! binds one [`ReplySink`]; the declared [`ReplyTransport`] decides the
//! **cadence** at which the host asks it to reconcile: `stream` sees every
//! revision, `message` sees only the terminal materialization. One seam, two
//! cadences.
//!
//! **What this vocabulary cannot represent is the point.** There is no field
//! for raw chain-of-thought, hidden model reasoning, unrestricted tool
//! arguments or results, credentials, secrets, host paths, or provider
//! payloads: every text field is a validating newtype with a byte bound and a
//! control-character ban, every collection is capped by construction, and the
//! only reasoning that can appear is a product-approved summary segment.
//! Provider verbs (`startStream`, session status, Block Kit, SSE frames)
//! never appear here — a sink owns those behind its checkpoint.
//!
//! The reducer ([`ReplyDocument::apply`]) is deterministic, side-effect free,
//! and total: the same change sequence always yields the same document.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::attachment::WorkspaceFile;
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

use crate::channel::ReplyTransport;
use crate::channel_adapter::ChannelError;
use crate::external::ExternalConversationRef;
use crate::tool_adapter::RestrictedEgress;

/// Bound on the cumulative answer text of one reply.
pub const REPLY_ANSWER_MAX_BYTES: usize = 128 * 1024;
/// Bound on titles, headlines, status lines, and other one-line display text.
pub const REPLY_DISPLAY_TEXT_MAX_BYTES: usize = 2 * 1024;
/// Bound on sanitized activity input/output previews and attention bodies.
pub const REPLY_DISPLAY_PREVIEW_MAX_BYTES: usize = 4 * 1024;
/// Bound on one product-approved reasoning summary segment.
pub const REPLY_REASONING_SEGMENT_MAX_BYTES: usize = 8 * 1024;
/// Bound on a reply/item identifier.
pub const REPLY_ITEM_ID_MAX_BYTES: usize = 128;
/// Bound on activity rows retained per document (older rows stay; later
/// starts are dropped and the document says so).
pub const REPLY_MAX_ACTIVITIES: usize = 256;
/// Bound on retained reasoning summary segments.
pub const REPLY_MAX_REASONING_SEGMENTS: usize = 128;
/// Bound on final attachments.
pub const REPLY_MAX_ATTACHMENTS: usize = 16;
/// Bound on the adapter-owned checkpoint the host persists between
/// reconciliations.
pub const REPLY_SINK_CHECKPOINT_MAX_BYTES: usize = 16 * 1024;
/// Bound on one provider reference in sink evidence.
pub const REPLY_PROVIDER_REF_MAX_BYTES: usize = 256;
/// Bound on provider references retained per report.
pub const REPLY_MAX_PROVIDER_REFS: usize = 32;
/// Bound on a vendor threading anchor.
pub const REPLY_THREAD_ANCHOR_MAX_BYTES: usize = 256;
/// Bound on the adapter's stored ingress context handed back at reply time
/// (the same 4 KiB `NormalizedInboundMessage::reply_context` bound).
pub const REPLY_CONTEXT_MAX_BYTES: usize = 4 * 1024;
/// Bound on a diagnostic outcome reason.
pub const REPLY_OUTCOME_REASON_MAX_BYTES: usize = 512;

/// Failures constructing reply vocabulary from untrusted or unbounded input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplyContractError {
    #[error("{field} exceeds {max} bytes")]
    TextTooLong { field: &'static str, max: usize },
    #[error("{field} must not be empty")]
    EmptyText { field: &'static str },
    #[error("{field} contains a control character")]
    ControlCharacter { field: &'static str },
    #[error("{field} is not a valid identifier: {reason}")]
    InvalidId {
        field: &'static str,
        reason: &'static str,
    },
    #[error("{field} holds more than {max} items")]
    TooManyItems { field: &'static str, max: usize },
    #[error("reply sink checkpoint exceeds {max} bytes")]
    CheckpointTooLarge { max: usize },
    #[error("reply context exceeds {max} bytes")]
    ReplyContextTooLarge { max: usize },
}

fn validate_text(
    field: &'static str,
    value: &str,
    max: usize,
    allow_empty: bool,
) -> Result<(), ReplyContractError> {
    if value.len() > max {
        return Err(ReplyContractError::TextTooLong { field, max });
    }
    if !allow_empty && value.trim().is_empty() {
        return Err(ReplyContractError::EmptyText { field });
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t' | '\r'))
    {
        return Err(ReplyContractError::ControlCharacter { field });
    }
    Ok(())
}

/// Fold arbitrary text into the bound: strip control characters (keeping
/// line structure) and cut at the last character boundary under `max`.
/// Used by the constructors that must never fail because their input is
/// diagnostic (an adapter's reason string) rather than product content.
fn fold_text(value: &str, max: usize) -> String {
    let stripped: String = value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect();
    if stripped.len() <= max {
        return stripped;
    }
    let mut end = max;
    while end > 0 && !stripped.is_char_boundary(end) {
        end -= 1;
    }
    stripped[..end].to_string() // safety: `end` walked back to a char boundary above.
}

macro_rules! bounded_text {
    ($(#[$doc:meta])* $name:ident, $field:literal, $max:expr, allow_empty = $allow_empty:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ReplyContractError> {
                let value = value.into();
                validate_text($field, &value, $max, $allow_empty)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ReplyContractError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

bounded_text!(
    /// One-line display text: titles, headlines, status lines, failure
    /// summaries, gate refs, URLs shown as text. Never empty.
    ReplyDisplayText,
    "reply display text",
    REPLY_DISPLAY_TEXT_MAX_BYTES,
    allow_empty = false
);

bounded_text!(
    /// A sanitized, bounded preview a product surface may show beside an
    /// activity (an input summary, an output excerpt) or under an attention
    /// headline. Producers build it only from already-sanitized display
    /// sources — never from raw tool arguments or results.
    ReplyDisplayPreview,
    "reply display preview",
    REPLY_DISPLAY_PREVIEW_MAX_BYTES,
    allow_empty = false
);

bounded_text!(
    /// Cumulative answer text. May be empty (a run can finish with only
    /// attachments, or fail before producing text).
    ReplyAnswerText,
    "reply answer text",
    REPLY_ANSWER_MAX_BYTES,
    allow_empty = true
);

bounded_text!(
    /// One product-approved reasoning summary segment. This is the only
    /// reasoning the seam can carry; disclosure policy decides which
    /// audiences see it at all.
    ReplyReasoningText,
    "reply reasoning summary",
    REPLY_REASONING_SEGMENT_MAX_BYTES,
    allow_empty = false
);

bounded_text!(
    /// A vendor threading anchor within the target conversation (a thread
    /// timestamp, a reply-to message id).
    ReplyThreadAnchor,
    "reply thread anchor",
    REPLY_THREAD_ANCHOR_MAX_BYTES,
    allow_empty = false
);

bounded_text!(
    /// One provider-issued reference (message id, stream id) a sink reports
    /// as evidence.
    ReplyProviderRef,
    "reply provider ref",
    REPLY_PROVIDER_REF_MAX_BYTES,
    allow_empty = false
);

/// A diagnostic reason attached to a non-applied sink outcome. Always
/// constructible: adapter-supplied text is folded into the bound rather than
/// rejected, because a reason exists to explain a failure, never to gate it.
/// The host treats it as diagnostic text — it is never rendered to a user
/// and never parsed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ReplyOutcomeReason(String);

impl ReplyOutcomeReason {
    pub fn new(value: impl AsRef<str>) -> Self {
        let folded = fold_text(value.as_ref(), REPLY_OUTCOME_REASON_MAX_BYTES);
        if folded.trim().is_empty() {
            return Self("unspecified".to_string());
        }
        Self(folded)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ReplyOutcomeReason {
    type Error = ReplyContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self::new(value))
    }
}

impl fmt::Display for ReplyOutcomeReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ReplyContractError> {
    if value.is_empty() {
        return Err(ReplyContractError::InvalidId {
            field,
            reason: "must not be empty",
        });
    }
    if value.len() > REPLY_ITEM_ID_MAX_BYTES {
        return Err(ReplyContractError::InvalidId {
            field,
            reason: "exceeds the identifier byte bound",
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ReplyContractError::InvalidId {
            field,
            reason: "may contain only ASCII alphanumerics, '.', '_', ':' and '-'",
        });
    }
    Ok(())
}

macro_rules! bounded_identifier {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ReplyContractError> {
                let value = value.into();
                validate_identifier($field, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ReplyContractError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

bounded_identifier!(
    /// Stable identity of one reply: the run it answers.
    ReplyId,
    "reply id"
);

bounded_identifier!(
    /// Stable identity of one item inside a document (an activity row, an
    /// attachment). Sinks key provider-side state on it.
    ReplyItemId,
    "reply item id"
);

impl ReplyId {
    /// The canonical reply identity for a run. A run has exactly one reply.
    pub fn for_run(run_id: &TurnRunId) -> Self {
        Self(run_id.to_string())
    }
}

/// Where a reply stands. `WaitingForInput`, `Completed`, `Failed`, and
/// `Cancelled` are derived by the reducer from control-critical changes; the
/// others are set by explicit [`ReplyChange::PhaseChanged`] or by activity
/// starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyPhase {
    Preparing,
    Thinking,
    Working,
    WaitingForInput,
    Completed,
    Failed,
    Cancelled,
}

impl ReplyPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Thinking => "thinking",
            Self::Working => "working",
            Self::WaitingForInput => "waiting_for_input",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The answer as it currently stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyAnswer {
    /// Cumulative text (progressive appends, then the canonical finalized
    /// transcript text once the run finalizes it).
    pub text: ReplyAnswerText,
    /// True once [`ReplyChange::AnswerFinalized`] replaced the progressive
    /// text with the canonical transcript row. Progressive appends after that
    /// are ignored.
    pub finalized: bool,
    /// True when appends were dropped at the answer byte bound. The finalized
    /// text is authoritative regardless.
    #[serde(default)]
    pub truncated: bool,
}

impl Default for ReplyAnswer {
    fn default() -> Self {
        Self {
            text: ReplyAnswerText(String::new()),
            finalized: false,
            truncated: false,
        }
    }
}

/// Lifecycle state of one capability/tool activity row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyActivityState {
    Started,
    Running,
    Completed,
    Failed { kind: ReplyDisplayText },
    Killed,
}

impl ReplyActivityState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed { .. } | Self::Killed)
    }
}

/// Where a finished activity ran, as neutral display facts: the extension
/// that served it, the runtime lane, and how much output it produced. Never
/// the output itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyActivityProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ReplyDisplayText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<ReplyDisplayText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
}

/// One activity row: a capability invocation as a product surface may show
/// it. `detail` and `output_preview` are sanitized display previews, never
/// raw arguments or results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyActivity {
    pub id: ReplyItemId,
    pub title: ReplyDisplayText,
    pub state: ReplyActivityState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<ReplyDisplayPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<ReplyDisplayPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ReplyActivityProvenance>,
    /// The change ordinal that created this row; rows render in this order.
    pub started_ordinal: u64,
    /// The change ordinal of the last update to this row.
    pub updated_ordinal: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyAttentionKind {
    Approval,
    Auth,
    Resource,
}

/// The run is parked on the user. `action_url` is present only when the
/// disclosure policy allowed it for the target audience (a connect URL must
/// never land in a shared conversation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyAttention {
    pub kind: ReplyAttentionKind,
    pub headline: ReplyDisplayText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<ReplyDisplayPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_url: Option<ReplyDisplayText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<ReplyDisplayText>,
}

/// Metadata of one final attachment. Bytes never enter the document; a
/// terminal reconciliation carries the materialized files separately
/// ([`ReplyReconcileRequest::materialized_attachments`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyAttachmentRef {
    pub id: ReplyItemId,
    pub filename: ReplyDisplayText,
    pub mime_type: ReplyDisplayText,
    pub size_bytes: u64,
}

/// The terminal fact. Once set it is never replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyOutcome {
    Completed,
    Failed { summary: ReplyDisplayText },
    Cancelled,
}

/// The desired state of one reply at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyDocument {
    pub phase: ReplyPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ReplyDisplayText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_kind: Option<ReplyStatusKind>,
    #[serde(default)]
    pub answer: ReplyAnswer,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning: Vec<ReplyReasoningText>,
    /// True while the last reasoning segment is still being produced
    /// ([`ReplyChange::ReasoningAppended`] grows it; a
    /// [`ReplyChange::ReasoningSummary`] closes it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning_open: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activities: Vec<ReplyActivity>,
    /// True when activity starts were dropped at [`REPLY_MAX_ACTIVITIES`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub activities_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<ReplyAttention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ReplyAttachmentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ReplyOutcome>,
    /// Number of changes folded so far; the source of activity ordinals.
    #[serde(default)]
    pub applied_changes: u64,
}

impl Default for ReplyDocument {
    fn default() -> Self {
        Self {
            phase: ReplyPhase::Preparing,
            status: None,
            status_kind: None,
            answer: ReplyAnswer::default(),
            reasoning: Vec::new(),
            reasoning_open: false,
            activities: Vec::new(),
            activities_truncated: false,
            attention: None,
            attachments: Vec::new(),
            outcome: None,
            applied_changes: 0,
        }
    }
}

impl ReplyDocument {
    /// Whether a terminal outcome has been recorded.
    pub fn is_terminal(&self) -> bool {
        self.outcome.is_some()
    }

    /// Fold one change into the document. Total and deterministic: bound
    /// overflow truncates or drops with a flag, ordering surprises land as
    /// rows rather than errors, and the first terminal outcome is durable.
    pub fn apply(&mut self, change: &ReplyChange) {
        self.applied_changes = self.applied_changes.saturating_add(1);
        let ordinal = self.applied_changes;
        match change {
            ReplyChange::PhaseChanged { phase } => {
                if !self.is_terminal() && self.attention.is_none() && !phase.is_terminal() {
                    self.phase = *phase;
                }
            }
            ReplyChange::StatusSummary { text, work } => {
                if !self.is_terminal() {
                    self.status = Some(text.clone());
                    self.status_kind = *work;
                }
            }
            ReplyChange::AnswerAppended { text } => {
                if self.is_terminal() || self.answer.finalized {
                    return;
                }
                self.append_answer(text.as_str());
            }
            ReplyChange::AnswerRewritten { text } => {
                if self.is_terminal() || self.answer.finalized {
                    return;
                }
                self.answer.text = ReplyAnswerText(String::new());
                self.answer.truncated = false;
                self.append_answer(text.as_str());
            }
            ReplyChange::AnswerFinalized { text, attachments } => {
                self.answer.text = text.clone();
                self.answer.finalized = true;
                self.answer.truncated = false;
                self.attachments = attachments
                    .iter()
                    .take(REPLY_MAX_ATTACHMENTS)
                    .cloned()
                    .collect();
            }
            ReplyChange::ReasoningSummary { text } => {
                if self.is_terminal() {
                    return;
                }
                if self.reasoning_open {
                    // The summary is the open segment's final text: replace,
                    // never duplicate.
                    if let Some(open) = self.reasoning.last_mut() {
                        *open = text.clone();
                    }
                    self.reasoning_open = false;
                } else if self.reasoning.len() < REPLY_MAX_REASONING_SEGMENTS {
                    self.reasoning.push(text.clone());
                }
                if self.attention.is_none() && matches!(self.phase, ReplyPhase::Preparing) {
                    self.phase = ReplyPhase::Thinking;
                }
            }
            ReplyChange::ReasoningAppended { text } => {
                if self.is_terminal() {
                    return;
                }
                if self.reasoning_open
                    && let Some(open) = self.reasoning.last_mut()
                {
                    let remaining = REPLY_REASONING_SEGMENT_MAX_BYTES.saturating_sub(open.0.len());
                    let fit = char_boundary_prefix(text.as_str(), remaining);
                    open.0.push_str(fit);
                } else if self.reasoning.len() < REPLY_MAX_REASONING_SEGMENTS {
                    self.reasoning.push(text.clone());
                    self.reasoning_open = true;
                }
                if self.attention.is_none() && matches!(self.phase, ReplyPhase::Preparing) {
                    self.phase = ReplyPhase::Thinking;
                }
            }
            ReplyChange::ActivityStarted { id, title, detail } => {
                if self.is_terminal() {
                    return;
                }
                if let Some(existing) = self.activities.iter_mut().find(|row| &row.id == id) {
                    existing.title = title.clone();
                    if detail.is_some() {
                        existing.detail = detail.clone();
                    }
                    existing.updated_ordinal = ordinal;
                } else if self.activities.len() >= REPLY_MAX_ACTIVITIES {
                    self.activities_truncated = true;
                } else {
                    self.activities.push(ReplyActivity {
                        id: id.clone(),
                        title: title.clone(),
                        state: ReplyActivityState::Started,
                        detail: detail.clone(),
                        output_preview: None,
                        provenance: None,
                        started_ordinal: ordinal,
                        updated_ordinal: ordinal,
                    });
                }
                if self.attention.is_none() {
                    self.phase = ReplyPhase::Working;
                }
            }
            ReplyChange::ActivityProgress { id, detail } => {
                if let Some(existing) = self.activities.iter_mut().find(|row| &row.id == id) {
                    if !existing.state.is_terminal() {
                        existing.state = ReplyActivityState::Running;
                    }
                    if detail.is_some() {
                        existing.detail = detail.clone();
                    }
                    existing.updated_ordinal = ordinal;
                }
            }
            ReplyChange::ActivityFinished {
                id,
                state,
                output_preview,
                provenance,
            } => {
                if let Some(existing) = self.activities.iter_mut().find(|row| &row.id == id) {
                    existing.state = state.clone();
                    if output_preview.is_some() {
                        existing.output_preview = output_preview.clone();
                    }
                    if provenance.is_some() {
                        existing.provenance = provenance.clone();
                    }
                    existing.updated_ordinal = ordinal;
                } else if self.activities.len() >= REPLY_MAX_ACTIVITIES {
                    self.activities_truncated = true;
                } else {
                    // A finish for a row this document never saw start (a
                    // producer that only observed the terminal milestone)
                    // still lands as a row: dropping it would hide a failure.
                    self.activities.push(ReplyActivity {
                        id: id.clone(),
                        title: ReplyDisplayText(id.as_str().to_string()),
                        state: state.clone(),
                        detail: None,
                        output_preview: output_preview.clone(),
                        provenance: provenance.clone(),
                        started_ordinal: ordinal,
                        updated_ordinal: ordinal,
                    });
                }
            }
            ReplyChange::AttentionRequired { attention } => {
                if self.is_terminal() {
                    return;
                }
                self.attention = Some(attention.clone());
                self.phase = ReplyPhase::WaitingForInput;
            }
            ReplyChange::AttentionCleared => {
                if self.is_terminal() {
                    return;
                }
                if self.attention.take().is_some() {
                    self.phase = ReplyPhase::Working;
                }
            }
            ReplyChange::Completed => self.settle(ReplyOutcome::Completed, ReplyPhase::Completed),
            ReplyChange::Failed { summary } => self.settle(
                ReplyOutcome::Failed {
                    summary: summary.clone(),
                },
                ReplyPhase::Failed,
            ),
            ReplyChange::Cancelled => self.settle(ReplyOutcome::Cancelled, ReplyPhase::Cancelled),
        }
    }

    fn append_answer(&mut self, delta: &str) {
        let current = &self.answer.text.0;
        let remaining = REPLY_ANSWER_MAX_BYTES.saturating_sub(current.len());
        if remaining == 0 {
            self.answer.truncated = true;
            return;
        }
        if delta.len() <= remaining {
            self.answer.text.0.push_str(delta);
            return;
        }
        let mut end = remaining;
        while end > 0 && !delta.is_char_boundary(end) {
            end -= 1;
        }
        // safety: `end` walked back to a char boundary above.
        self.answer.text.0.push_str(&delta[..end]);
        self.answer.truncated = true;
    }

    fn settle(&mut self, outcome: ReplyOutcome, phase: ReplyPhase) {
        if self.is_terminal() {
            return;
        }
        self.outcome = Some(outcome);
        self.phase = phase;
        self.attention = None;
    }
}

/// The longest prefix of `text` that fits in `max_bytes` without splitting a
/// character.
fn char_boundary_prefix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end] // safety: `end` walked back to a char boundary above.
}

/// The kind of work a status line describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyStatusKind {
    Planning,
    Waiting,
    Retrying,
    Context,
}

/// One semantic change. The `kind` tag is the persisted/wire name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplyChange {
    PhaseChanged {
        phase: ReplyPhase,
    },
    StatusSummary {
        text: ReplyDisplayText,
        /// What kind of work the line describes, when the producer knows —
        /// a surface may pick an icon or grouping from it. (Named `work`
        /// because `kind` is the change's own wire tag.)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work: Option<ReplyStatusKind>,
    },
    AnswerAppended {
        text: ReplyAnswerText,
    },
    /// The progressive answer was replaced wholesale (a model call restarted
    /// its text, a moderation rewrite). Ignored once the answer is finalized:
    /// the transcript row is authoritative from then on.
    AnswerRewritten {
        text: ReplyAnswerText,
    },
    /// The canonical finalized transcript text (and its attachments) — the
    /// authoritative answer, replacing progressive appends.
    AnswerFinalized {
        text: ReplyAnswerText,
        #[serde(default)]
        attachments: Vec<ReplyAttachmentRef>,
    },
    /// Closes the open reasoning segment (if any) and records `text` as a
    /// finished one.
    ReasoningSummary {
        text: ReplyReasoningText,
    },
    /// Grows the open reasoning segment — the one the model is still
    /// producing — so a progressive surface can show thinking as it happens.
    /// Bounded like every segment: growth past the segment bound is dropped
    /// until a `ReasoningSummary` closes it.
    ReasoningAppended {
        text: ReplyReasoningText,
    },
    ActivityStarted {
        id: ReplyItemId,
        title: ReplyDisplayText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ReplyDisplayPreview>,
    },
    ActivityProgress {
        id: ReplyItemId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ReplyDisplayPreview>,
    },
    ActivityFinished {
        id: ReplyItemId,
        state: ReplyActivityState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_preview: Option<ReplyDisplayPreview>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance: Option<ReplyActivityProvenance>,
    },
    AttentionRequired {
        attention: ReplyAttention,
    },
    AttentionCleared,
    Completed,
    Failed {
        summary: ReplyDisplayText,
    },
    Cancelled,
}

/// How a change matters to a publisher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplyChangeClass {
    /// Superseded intermediate state a publisher may coalesce under
    /// backpressure (text growth, reasoning, status, activity progress).
    Replaceable,
    /// An input-required transition or the canonical answer: reconciled as
    /// its own revision, never coalesced across.
    ControlCritical,
    /// The reply is over.
    Terminal,
}

impl ReplyChange {
    pub fn class(&self) -> ReplyChangeClass {
        match self {
            Self::Completed | Self::Failed { .. } | Self::Cancelled => ReplyChangeClass::Terminal,
            Self::AttentionRequired { .. }
            | Self::AttentionCleared
            | Self::AnswerFinalized { .. } => ReplyChangeClass::ControlCritical,
            Self::PhaseChanged { .. }
            | Self::StatusSummary { .. }
            | Self::AnswerAppended { .. }
            | Self::AnswerRewritten { .. }
            | Self::ReasoningSummary { .. }
            | Self::ReasoningAppended { .. }
            | Self::ActivityStarted { .. }
            | Self::ActivityProgress { .. }
            | Self::ActivityFinished { .. } => ReplyChangeClass::Replaceable,
        }
    }

    /// Control-critical changes are the ones a publisher may never coalesce
    /// *across*: input-required transitions and terminal facts must each be
    /// reconciled as their own revision, however far behind the publisher is.
    pub fn is_control_critical(&self) -> bool {
        !matches!(self.class(), ReplyChangeClass::Replaceable)
    }

    /// Whether this change ends the reply.
    pub fn is_terminal(&self) -> bool {
        matches!(self.class(), ReplyChangeClass::Terminal)
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::PhaseChanged { .. } => "phase_changed",
            Self::StatusSummary { .. } => "status_summary",
            Self::AnswerAppended { .. } => "answer_appended",
            Self::AnswerRewritten { .. } => "answer_rewritten",
            Self::AnswerFinalized { .. } => "answer_finalized",
            Self::ReasoningSummary { .. } => "reasoning_summary",
            Self::ReasoningAppended { .. } => "reasoning_appended",
            Self::ActivityStarted { .. } => "activity_started",
            Self::ActivityProgress { .. } => "activity_progress",
            Self::ActivityFinished { .. } => "activity_finished",
            Self::AttentionRequired { .. } => "attention_required",
            Self::AttentionCleared => "attention_cleared",
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Why the host is reconciling now — the cadence point this revision sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyReconcilePoint {
    /// The first revision of the reply (the run has started answering).
    Opened,
    /// Intermediate desired state; only `stream` sinks see these.
    Progress,
    /// An input-required transition (attention required or cleared) or the
    /// canonical answer arriving before the terminal fact.
    ControlCritical,
    /// The terminal materialization.
    Terminal,
    /// No new change: the host is re-reconciling the same desired state
    /// (a retry, a lease takeover, or a periodic heartbeat while the reply is
    /// open) so a sink may refresh a provider-side liveness signal.
    Heartbeat,
}

impl ReplyTransport {
    /// Whether a sink with this transport is asked to reconcile at `point`.
    /// `stream` sees every point; `message` sees only the terminal
    /// materialization — a one-shot channel answers once, and progress,
    /// attention, and liveness on such a channel stay with the host's
    /// source-routed notices.
    pub fn reconciles_at(self, point: ReplyReconcilePoint) -> bool {
        match self {
            Self::Stream => true,
            Self::Message => matches!(point, ReplyReconcilePoint::Terminal),
        }
    }
}

/// One desired state at one monotonic revision of one reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyRevision {
    pub reply_id: ReplyId,
    /// Monotonic within one publisher's ownership of the reply. A sink keys
    /// idempotency on its checkpoint, never on this number alone.
    pub revision: u64,
    pub document: ReplyDocument,
}

/// Who can see the target conversation. Decided by the host from facts it
/// owns (trigger class, conversation model); drives disclosure policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyAudience {
    /// An authenticated session or a direct conversation with one actor.
    Private,
    /// A channel, group, or any conversation with other members.
    Shared,
}

/// Where a revision is being reconciled to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTarget {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    /// The vendor conversation the run's input came from. `None` for the
    /// product projection target (the browser subscribes by thread scope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ExternalConversationRef>,
    /// Optional vendor threading anchor within that conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_anchor: Option<ReplyThreadAnchor>,
    pub audience: ReplyAudience,
}

/// The adapter's own stored ingress context (`NormalizedInboundMessage::
/// reply_context`) handed back at reply time. Bounded by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyContextBytes(Vec<u8>);

impl ReplyContextBytes {
    pub fn new(bytes: Vec<u8>) -> Result<Self, ReplyContractError> {
        if bytes.len() > REPLY_CONTEXT_MAX_BYTES {
            return Err(ReplyContractError::ReplyContextTooLarge {
                max: REPLY_CONTEXT_MAX_BYTES,
            });
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Adapter-owned reconciliation state the host persists between calls
/// (provider session/message refs, appended offsets, task hashes). Opaque to
/// the host beyond its bound; `version` lets a sink refuse a checkpoint it no
/// longer understands after an upgrade instead of misapplying it. Fields are
/// private so the bound holds by construction, including across
/// deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ReplySinkCheckpointWire")]
pub struct ReplySinkCheckpoint {
    version: u32,
    payload: String,
}

#[derive(Deserialize)]
struct ReplySinkCheckpointWire {
    version: u32,
    payload: String,
}

impl TryFrom<ReplySinkCheckpointWire> for ReplySinkCheckpoint {
    type Error = ReplyContractError;

    fn try_from(wire: ReplySinkCheckpointWire) -> Result<Self, Self::Error> {
        Self::new(wire.version, wire.payload)
    }
}

impl ReplySinkCheckpoint {
    pub fn new(version: u32, payload: impl Into<String>) -> Result<Self, ReplyContractError> {
        let payload = payload.into();
        if payload.len() > REPLY_SINK_CHECKPOINT_MAX_BYTES {
            return Err(ReplyContractError::CheckpointTooLarge {
                max: REPLY_SINK_CHECKPOINT_MAX_BYTES,
            });
        }
        Ok(Self { version, payload })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn payload(&self) -> &str {
        &self.payload
    }
}

/// One reconciliation request: the desired revision, the target, the
/// adapter's stored ingress context, its last checkpoint, and — on the
/// terminal point only — the materialized final attachments.
#[derive(Debug, Clone)]
pub struct ReplyReconcileRequest {
    pub revision: ReplyRevision,
    pub point: ReplyReconcilePoint,
    pub target: ReplyTarget,
    /// The opaque `reply_context` the adapter attached to the originating
    /// inbound message, when the target has one.
    pub reply_context: Option<ReplyContextBytes>,
    /// The checkpoint the sink returned from its previous applied
    /// reconciliation for this `(reply, target)`, if any.
    pub checkpoint: Option<ReplySinkCheckpoint>,
    /// The extension generation the host resolved this sink from. A sink
    /// compares it with the generation its checkpoint was minted under.
    pub extension_generation: u64,
    /// The final attachments, read by the host under policy immediately
    /// before this call. Non-empty only at [`ReplyReconcilePoint::Terminal`]
    /// (and only when the document lists attachments); transient — never
    /// persisted in checkpoints, attempts, events, or projections.
    pub materialized_attachments: Vec<WorkspaceFile>,
}

/// What the provider did with one reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplySinkOutcome {
    /// The provider now reflects the requested revision.
    Applied,
    /// Nothing was changed provider-side; try the same revision again after
    /// `retry_after` (the provider's hint) or the host's backoff.
    Retryable {
        reason: ReplyOutcomeReason,
        retry_after: Option<Duration>,
    },
    /// The request crossed into transport and the provider may or may not
    /// have applied it. The host records the uncertainty; the sink's returned
    /// checkpoint should reflect what it could read back.
    Ambiguous { reason: ReplyOutcomeReason },
    /// This target can no longer take this reply.
    Permanent { reason: ReplyOutcomeReason },
    /// The provider rejected the credential; the host raises re-auth.
    Unauthorized { reason: ReplyOutcomeReason },
    /// The provider reports the user stopped this reply. The host records a
    /// cancellation for the reply.
    StoppedByUser,
}

impl ReplySinkOutcome {
    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied)
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Retryable { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Retryable { .. } => "retryable",
            Self::Ambiguous { .. } => "ambiguous",
            Self::Permanent { .. } => "permanent",
            Self::Unauthorized { .. } => "unauthorized",
            Self::StoppedByUser => "stopped_by_user",
        }
    }
}

/// Provider-issued references, bounded by construction: at most
/// [`REPLY_MAX_PROVIDER_REFS`], each within [`REPLY_PROVIDER_REF_MAX_BYTES`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<ReplyProviderRef>")]
pub struct ReplyProviderRefs(Vec<ReplyProviderRef>);

impl ReplyProviderRefs {
    pub fn new(refs: Vec<ReplyProviderRef>) -> Result<Self, ReplyContractError> {
        if refs.len() > REPLY_MAX_PROVIDER_REFS {
            return Err(ReplyContractError::TooManyItems {
                field: "reply sink provider refs",
                max: REPLY_MAX_PROVIDER_REFS,
            });
        }
        Ok(Self(refs))
    }

    /// Append one reference; a reference past the bound is refused rather
    /// than silently dropped so a sink notices it is over-reporting.
    pub fn push(&mut self, reference: ReplyProviderRef) -> Result<(), ReplyContractError> {
        if self.0.len() >= REPLY_MAX_PROVIDER_REFS {
            return Err(ReplyContractError::TooManyItems {
                field: "reply sink provider refs",
                max: REPLY_MAX_PROVIDER_REFS,
            });
        }
        self.0.push(reference);
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &ReplyProviderRef> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<Vec<ReplyProviderRef>> for ReplyProviderRefs {
    type Error = ReplyContractError;

    fn try_from(refs: Vec<ReplyProviderRef>) -> Result<Self, Self::Error> {
        Self::new(refs)
    }
}

/// Provider evidence a sink reports. `read_back_verified` is true only when
/// the sink re-read provider state and found the revision reflected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplySinkEvidence {
    #[serde(default)]
    pub provider_refs: ReplyProviderRefs,
    #[serde(default)]
    pub read_back_verified: bool,
}

/// One reconciliation's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplySinkReport {
    pub outcome: ReplySinkOutcome,
    /// The next checkpoint to persist for this `(reply, target)`; `None`
    /// keeps the previous one.
    pub checkpoint: Option<ReplySinkCheckpoint>,
    pub evidence: ReplySinkEvidence,
}

impl ReplySinkReport {
    pub fn applied(checkpoint: Option<ReplySinkCheckpoint>, evidence: ReplySinkEvidence) -> Self {
        Self {
            outcome: ReplySinkOutcome::Applied,
            checkpoint,
            evidence,
        }
    }
}

/// **The reply half** — every channel declaring `[channel.reply]` binds one.
/// The host asks it to reconcile its provider toward successive desired
/// revisions of one reply at the cadence its declared transport admits
/// ([`ReplyTransport::reconciles_at`]); the sink owns every provider
/// mechanic behind its checkpoint and never writes delivery state.
///
/// Contract:
/// - Idempotent for a repeated `(reply, target, revision)`: the checkpoint
///   tells the sink what it already applied.
/// - Must never claim `Applied` for a request the provider may not have
///   accepted; report `Ambiguous` and, where the provider allows, read back.
/// - Must never trust a checkpoint whose `version` it does not understand.
/// - Reasons are diagnostic text the host never renders; never put provider
///   payloads, tokens, or user content in them.
#[async_trait]
pub trait ReplySink: Send + Sync {
    async fn reconcile(
        &self,
        request: ReplyReconcileRequest,
        egress: &dyn RestrictedEgress,
    ) -> Result<ReplySinkReport, ChannelError>;
}
