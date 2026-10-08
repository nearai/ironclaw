//! A conversation's turn-start tool selection: which deferred tools it
//! advertises to the model beside the always-advertised core tools.
//!
//! The record is written once, at the conversation's first turn with user
//! text, and never rewritten: the request's `tools` array is part of the
//! provider's cached prompt prefix, so every later turn rebuilds the same
//! list from this record instead of choosing again. A second writer is handed
//! the stored record, not an error, so two runs racing on one conversation
//! serve the same list.
//!
//! Like structured-finalization evidence it is stored beside the thread root,
//! partitioned by thread incarnation: deleting a thread keeps it, and a
//! recreated thread id starts afresh.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ids::{CapabilityId, ThreadId},
    turn::TurnId,
};
use serde::{Deserialize, Serialize};

use crate::{SessionThreadError, ThreadScope};

/// Version of the stored [`ToolSelectionRecord`] shape.
pub const TOOL_SELECTION_SCHEMA_VERSION: u32 = 1;

/// Most tools one record may select.
pub const MAX_TOOL_SELECTION_TOOLS: usize = 128;

/// Longest scorer identifier a record may carry, in bytes.
pub const MAX_TOOL_SELECTION_SCORER_BYTES: usize = 256;

/// One selected tool and the score it was chosen on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedTool {
    pub capability_id: CapabilityId,
    /// Finite and non-negative, on the scale `scorer` names.
    pub score: f32,
}

/// Why a conversation recorded an empty selection instead of a classifier's
/// answer. The conversation then advertises the ordinary disclosure surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSelectionFallbackReason {
    Unavailable,
    Timeout,
    Unauthorized,
    PaymentRequired,
    Rejected,
    RateLimited,
    InvalidOutput,
}

impl ToolSelectionFallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Unauthorized => "unauthorized",
            Self::PaymentRequired => "payment_required",
            Self::Rejected => "rejected",
            Self::RateLimited => "rate_limited",
            Self::InvalidOutput => "invalid_output",
        }
    }
}

/// The tools one conversation selected at turn start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSelectionRecord {
    pub schema_version: u32,
    /// The turn that made the selection.
    pub turn_id: TurnId,
    /// The selected tools, best first, each once.
    pub selected: Vec<SelectedTool>,
    /// Identifier of the scale the scores are on (`jev:jev-latest`, say).
    /// `None` on a fallback record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scorer: Option<String>,
    /// Set when the classifier failed; `selected` is then empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<ToolSelectionFallbackReason>,
    pub recorded_at: DateTime<Utc>,
}

impl ToolSelectionRecord {
    /// Check the record's shape. Run on every write and on every read of a
    /// stored record, so a corrupt document is never served.
    pub fn validate(&self) -> Result<(), SessionThreadError> {
        let invalid = |reason: String| SessionThreadError::InvalidToolSelection { reason };
        if self.schema_version != TOOL_SELECTION_SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }
        if self.selected.len() > MAX_TOOL_SELECTION_TOOLS {
            return Err(invalid(format!(
                "{} tools selected, at most {MAX_TOOL_SELECTION_TOOLS} allowed",
                self.selected.len()
            )));
        }
        for (index, tool) in self.selected.iter().enumerate() {
            if !tool.score.is_finite() || tool.score < 0.0 {
                return Err(invalid(format!(
                    "tool {} has an invalid score",
                    tool.capability_id
                )));
            }
            if self
                .selected
                .iter()
                .take(index)
                .any(|earlier| earlier.capability_id == tool.capability_id)
            {
                return Err(invalid(format!(
                    "tool {} is selected twice",
                    tool.capability_id
                )));
            }
        }
        if self.scorer.as_ref().is_some_and(|scorer| {
            scorer.is_empty() || scorer.len() > MAX_TOOL_SELECTION_SCORER_BYTES
        }) {
            return Err(invalid("the scorer identifier is empty or too long".into()));
        }
        if self.fallback_reason.is_some() && !self.selected.is_empty() {
            return Err(invalid("a fallback record selects no tools".into()));
        }
        Ok(())
    }
}

/// Record a conversation's selection, unless one is already stored.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordToolSelectionRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub record: ToolSelectionRecord,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(names: &[&str]) -> ToolSelectionRecord {
        ToolSelectionRecord {
            schema_version: TOOL_SELECTION_SCHEMA_VERSION,
            turn_id: TurnId::new(),
            selected: names
                .iter()
                .map(|name| SelectedTool {
                    capability_id: CapabilityId::new(*name).expect("valid capability id"),
                    score: 0.5,
                })
                .collect(),
            scorer: Some("jev:jev-latest".to_string()),
            fallback_reason: None,
            recorded_at: Utc::now(),
        }
    }

    #[test]
    fn a_well_formed_record_round_trips_and_validates() {
        let record = record(&["gmail.send", "slack.post"]);
        record.validate().expect("valid record");
        let json = serde_json::to_string(&record).expect("serializes");
        let decoded: ToolSelectionRecord = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(decoded, record);
    }

    #[test]
    fn malformed_records_are_refused() {
        let mut duplicate = record(&["gmail.send", "gmail.send"]);
        assert!(duplicate.validate().is_err());
        duplicate.selected.pop();
        duplicate.selected[0].score = f32::NAN;
        assert!(duplicate.validate().is_err());

        let mut fallback = record(&["gmail.send"]);
        fallback.fallback_reason = Some(ToolSelectionFallbackReason::Timeout);
        assert!(fallback.validate().is_err(), "a fallback selects nothing");

        let mut version = record(&[]);
        version.schema_version = TOOL_SELECTION_SCHEMA_VERSION + 1;
        assert!(version.validate().is_err());

        let names: Vec<String> = (0..=MAX_TOOL_SELECTION_TOOLS)
            .map(|index| format!("fixture.tool_{index}"))
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert!(record(&names).validate().is_err());
    }

    #[test]
    fn fallback_reasons_serialize_as_their_labels() {
        let reason = ToolSelectionFallbackReason::PaymentRequired;
        assert_eq!(
            serde_json::to_string(&reason).expect("serializes"),
            format!("\"{}\"", reason.as_str())
        );
    }
}
