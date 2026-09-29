//! Append-only history of the tools a conversation advertises to the model.
//!
//! When turn-start tool selection is on, the loop host picks the tools to
//! advertise from the conversation's opening request and keeps that list
//! byte-identical for as long as the provider's prompt cache could be warm:
//! the `tools` array is part of the cached prompt prefix, so any change to it
//! re-bills the whole prompt. Once the cache has gone cold, or the model
//! changed, the host may choose again from the conversation so far. This
//! module owns the durable record of those lists. It knows nothing about
//! ranking or prompt caches; it stores entries, enforces the append-only
//! invariants, and answers "which entry was in force at this turn".
//!
//! Beside the history it keeps the conversation's [`ToolSelectionActivity`]:
//! when (and on which model) the conversation last called the model, and
//! which tools it has called successfully. The host reads it to decide
//! whether the cache could still be warm, and which tools a new choice keeps.
//!
//! Each change appends an entry. Nothing is overwritten or deleted: an entry
//! is LLM-facing data (it records exactly what the model was shown), and the
//! history is kept outside the thread root so deleting a thread does not erase
//! it, while a recreated thread id starts a fresh history.
//!
//! New selection reasons are added as new [`ToolSelectionReason`] variants;
//! the entry shape does not change, so stored entries never need migrating.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{ids::ThreadId, turn::TurnId};
use serde::{Deserialize, Serialize};

use crate::{SessionThreadError, ThreadScope};

/// Version of the stored [`ToolSelectionHistory`] shape.
pub const TOOL_SELECTION_HISTORY_SCHEMA_VERSION: u32 = 1;

/// Most entries one conversation's history may hold. A conversation appends
/// one `initial` entry, then an entry per revocation and per re-selection
/// after an idle gap or a model change, so this is far above any real
/// history; it bounds the record against a runaway writer.
pub const MAX_TOOL_SELECTION_ENTRIES: usize = 256;

/// Most tool names one entry may advertise.
pub const MAX_TOOL_SELECTION_ADVERTISED: usize = 1_024;

const MAX_TOOL_NAME_BYTES: usize = 256;
const MAX_RANKER_VERSION_BYTES: usize = 256;
const MAX_FALLBACK_REASON_BYTES: usize = 64;
/// The `tool_search` description is a model-safe summary capped at 4 KiB by
/// the capability layer; leave generous headroom without admitting unbounded
/// text.
const MAX_TOOL_SEARCH_DESCRIPTION_BYTES: usize = 16 * 1024;

/// Why an entry was appended.
///
/// Serialized in `snake_case`. New reasons are added as new variants;
/// existing stored entries keep deserializing unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSelectionReason {
    /// The conversation's first selection, made from its opening request.
    /// Always the first entry, and only ever the first.
    Initial,
    /// Tools were removed because their authorization or availability was
    /// revoked. Always drops at least one tool the previous entry advertised;
    /// when the host re-selects at the same moment it may also add tools.
    Revoked,
    /// Re-selected from the conversation so far because the provider's
    /// prompt cache had expired since the conversation's last model call.
    CacheCold,
    /// Re-selected from the conversation so far because the model or
    /// provider changed since the conversation's last model call.
    ModelChange,
}

impl ToolSelectionReason {
    /// Stable label for logs and metrics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Revoked => "revoked",
            Self::CacheCold => "cache_cold",
            Self::ModelChange => "model_change",
        }
    }
}

/// The turn an entry takes effect from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSelectionEffectiveFrom {
    pub turn_id: TurnId,
    /// Thread sequence of that turn's accepted user message, when the turn
    /// had one. Orders the entry against turns that have no entry of their
    /// own (see [`ToolSelectionHistory::in_force_at`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_sequence: Option<u64>,
}

/// One ranked tool's score, on the scale named by the entry's
/// `ranker_version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSelectionScore {
    pub name: String,
    pub score: f32,
}

/// One advertised tool list and why it took effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSelectionEntry {
    pub effective_from: ToolSelectionEffectiveFrom,
    pub reason: ToolSelectionReason,
    /// Provider tool names in the exact order the request's `tools` array
    /// carries them.
    pub advertised: Vec<String>,
    /// The score of each tool the ranker selected. Always-on tools that were
    /// advertised without being ranked in have no score.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scores: Vec<ToolSelectionScore>,
    /// The ranker whose scale `scores` is on (its `ranker_version`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranker_version: Option<String>,
    /// The `tool_search` description advertised with this list. It indexes
    /// the tools that are *not* advertised, so it is frozen with the list:
    /// recomputing it later would change the cached prompt prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_search_description: Option<String>,
    /// Set when this list is a fallback rather than a classifier's choice:
    /// the classifier failed and the host froze a default list instead. The
    /// value is the failure's stable label (for example `timeout`), never
    /// its detail. Absent on every classifier-made entry, and on entries
    /// written before the field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

impl ToolSelectionEntry {
    /// Validate the entry's own shape (not its place in a history).
    pub fn validate(&self) -> Result<(), SessionThreadError> {
        if self.advertised.len() > MAX_TOOL_SELECTION_ADVERTISED {
            return Err(invalid("tool selection advertises too many tools"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.advertised {
            if name.is_empty() || name.len() > MAX_TOOL_NAME_BYTES {
                return Err(invalid("tool selection names an invalid tool"));
            }
            if !seen.insert(name.as_str()) {
                return Err(invalid("tool selection advertises a tool twice"));
            }
        }
        let mut scored = std::collections::BTreeSet::new();
        for score in &self.scores {
            if !seen.contains(score.name.as_str()) {
                return Err(invalid(
                    "tool selection scores a tool it does not advertise",
                ));
            }
            if !scored.insert(score.name.as_str()) {
                return Err(invalid("tool selection scores a tool twice"));
            }
            if !score.score.is_finite() || score.score < 0.0 {
                return Err(invalid("tool selection carries an invalid score"));
            }
        }
        if self
            .ranker_version
            .as_ref()
            .is_some_and(|version| version.is_empty() || version.len() > MAX_RANKER_VERSION_BYTES)
        {
            return Err(invalid("tool selection ranker version is invalid"));
        }
        if self.fallback_reason.as_ref().is_some_and(|reason| {
            reason.is_empty()
                || reason.len() > MAX_FALLBACK_REASON_BYTES
                || !reason
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }) {
            return Err(invalid("tool selection fallback reason is invalid"));
        }
        if self
            .tool_search_description
            .as_ref()
            .is_some_and(|description| description.len() > MAX_TOOL_SEARCH_DESCRIPTION_BYTES)
        {
            return Err(invalid(
                "tool selection tool_search description is too large",
            ));
        }
        Ok(())
    }
}

/// The whole append-only history of one conversation (one thread incarnation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSelectionHistory {
    pub schema_version: u32,
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    /// Oldest first. Never empty in a stored history.
    pub entries: Vec<ToolSelectionEntry>,
}

impl ToolSelectionHistory {
    /// The most recent entry: the one in force for the current and every
    /// later turn.
    pub fn latest(&self) -> Option<&ToolSelectionEntry> {
        self.entries.last()
    }

    /// The entry in force at a given turn, for resume and replay.
    ///
    /// An entry that took effect at `turn_id` itself wins (the last one, if
    /// that turn appended several). Otherwise, when the turn's accepted
    /// message sequence is known, the in-force entry is the last one that took
    /// effect at an earlier message; `None` means the turn predates the
    /// history. With no sequence to order by, the latest entry is in force.
    pub fn in_force_at(
        &self,
        turn_id: TurnId,
        message_sequence: Option<u64>,
    ) -> Option<&ToolSelectionEntry> {
        if let Some(entry) = self
            .entries
            .iter()
            .rev()
            .find(|entry| entry.effective_from.turn_id == turn_id)
        {
            return Some(entry);
        }
        let Some(sequence) = message_sequence else {
            return self.latest();
        };
        self.entries.iter().rev().find(|entry| {
            entry
                .effective_from
                .message_sequence
                .is_some_and(|entry_sequence| entry_sequence <= sequence)
        })
    }
}

/// Append one entry to a conversation's history.
#[derive(Debug, Clone, PartialEq)]
pub struct AppendToolSelectionEntryRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    /// How many entries the caller saw when it built `entry`. The append is
    /// refused with [`SessionThreadError::ToolSelectionHistoryConflict`] when
    /// another writer appended first; the caller re-reads and uses the entry
    /// now in force instead of racing it.
    pub expected_entries: usize,
    pub entry: ToolSelectionEntry,
}

/// Apply `request` to the stored history (`None` when nothing is stored yet).
/// Shared by every backend so the append rules cannot drift between them.
pub(crate) fn append_tool_selection_entry(
    current: Option<ToolSelectionHistory>,
    request: &AppendToolSelectionEntryRequest,
) -> Result<ToolSelectionHistory, SessionThreadError> {
    request.entry.validate()?;
    let mut history = current.unwrap_or_else(|| ToolSelectionHistory {
        schema_version: TOOL_SELECTION_HISTORY_SCHEMA_VERSION,
        scope: request.scope.clone(),
        thread_id: request.thread_id.clone(),
        entries: Vec::new(),
    });
    if history.scope != request.scope || history.thread_id != request.thread_id {
        return Err(SessionThreadError::Backend(
            "tool selection history key does not match the requested thread".to_string(),
        ));
    }
    if history.entries.len() != request.expected_entries {
        return Err(SessionThreadError::ToolSelectionHistoryConflict {
            thread_id: request.thread_id.clone(),
            expected_entries: request.expected_entries,
            actual_entries: history.entries.len(),
        });
    }
    if history.entries.len() >= MAX_TOOL_SELECTION_ENTRIES {
        return Err(invalid("tool selection history is full"));
    }
    match (history.entries.last(), request.entry.reason) {
        (None, ToolSelectionReason::Initial) => {}
        (None, _) => {
            return Err(invalid(
                "a tool selection history must start with an initial entry",
            ));
        }
        (Some(_), ToolSelectionReason::Initial) => {
            return Err(invalid(
                "a tool selection history has only one initial entry",
            ));
        }
        (Some(previous), ToolSelectionReason::Revoked) => {
            if previous
                .advertised
                .iter()
                .all(|name| request.entry.advertised.contains(name))
            {
                return Err(invalid("a revoked tool selection entry must remove a tool"));
            }
        }
        (Some(previous), ToolSelectionReason::CacheCold | ToolSelectionReason::ModelChange) => {
            if previous.advertised == request.entry.advertised {
                return Err(invalid(
                    "a re-selection entry must change the advertised tools",
                ));
            }
        }
    }
    history.entries.push(request.entry.clone());
    Ok(history)
}

/// Validate a history read back from storage.
pub(crate) fn validate_stored_history(
    history: &ToolSelectionHistory,
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<(), SessionThreadError> {
    if &history.scope != scope || &history.thread_id != thread_id {
        return Err(SessionThreadError::Backend(
            "tool selection history key does not match the requested thread".to_string(),
        ));
    }
    if history.schema_version != TOOL_SELECTION_HISTORY_SCHEMA_VERSION {
        return Err(invalid(
            "tool selection history has an unknown schema version",
        ));
    }
    if history.entries.is_empty() {
        return Err(invalid("stored tool selection history is empty"));
    }
    for entry in &history.entries {
        entry.validate()?;
    }
    Ok(())
}

/// Version of the stored [`ToolSelectionActivity`] shape.
pub const TOOL_SELECTION_ACTIVITY_SCHEMA_VERSION: u32 = 1;

/// Most called tools one conversation's activity keeps. Tools called after
/// the list is full are not recorded; the first ones stay.
pub const MAX_TOOL_SELECTION_CALLED_TOOLS: usize = 256;

const MAX_MODEL_IDENTITY_BYTES: usize = 512;

/// The conversation's latest model call.
///
/// Durable so that the idle time since that call survives a restart: a
/// restarted host must neither treat a warm prompt cache as cold nor the
/// reverse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallMark {
    /// The turn that made the call.
    pub turn_id: TurnId,
    /// When the call returned.
    pub called_at: DateTime<Utc>,
    /// Opaque identity of the provider and model the call went to, when the
    /// host knows it. Compared for equality only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// What a conversation has done that turn-start tool selection reads back:
/// its latest model call and the tools it called successfully.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSelectionActivity {
    pub schema_version: u32,
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_model_call: Option<ModelCallMark>,
    /// Provider names of the tools the conversation has called
    /// successfully, in first-call order, each once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub called_tools: Vec<String>,
}

/// One change to a conversation's [`ToolSelectionActivity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSelectionActivityUpdate {
    /// A model call returned. The stored mark only moves forward in time: a
    /// mark older than the stored one is ignored.
    ModelCall(ModelCallMark),
    /// A tool call succeeded. Recorded once per tool.
    CalledTool(String),
}

/// Record one change to a conversation's [`ToolSelectionActivity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordToolSelectionActivityRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub update: ToolSelectionActivityUpdate,
}

/// Apply `request` to the stored activity (`None` when nothing is stored
/// yet). Shared by every backend so the rules cannot drift between them.
/// Returns the stored value unchanged when the update changes nothing.
pub(crate) fn record_tool_selection_activity(
    current: Option<ToolSelectionActivity>,
    request: &RecordToolSelectionActivityRequest,
) -> Result<ToolSelectionActivity, SessionThreadError> {
    let mut activity = current.unwrap_or_else(|| ToolSelectionActivity {
        schema_version: TOOL_SELECTION_ACTIVITY_SCHEMA_VERSION,
        scope: request.scope.clone(),
        thread_id: request.thread_id.clone(),
        last_model_call: None,
        called_tools: Vec::new(),
    });
    if activity.scope != request.scope || activity.thread_id != request.thread_id {
        return Err(SessionThreadError::Backend(
            "tool selection activity key does not match the requested thread".to_string(),
        ));
    }
    match &request.update {
        ToolSelectionActivityUpdate::ModelCall(mark) => {
            validate_model_call_mark(mark)?;
            let newer = activity
                .last_model_call
                .as_ref()
                .is_none_or(|stored| stored.called_at <= mark.called_at);
            if newer {
                activity.last_model_call = Some(mark.clone());
            }
        }
        ToolSelectionActivityUpdate::CalledTool(name) => {
            validate_tool_name(name)?;
            if !activity.called_tools.contains(name)
                && activity.called_tools.len() < MAX_TOOL_SELECTION_CALLED_TOOLS
            {
                activity.called_tools.push(name.clone());
            }
        }
    }
    Ok(activity)
}

/// Validate an activity record read back from storage.
pub(crate) fn validate_stored_activity(
    activity: &ToolSelectionActivity,
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<(), SessionThreadError> {
    if &activity.scope != scope || &activity.thread_id != thread_id {
        return Err(SessionThreadError::Backend(
            "tool selection activity key does not match the requested thread".to_string(),
        ));
    }
    if activity.schema_version != TOOL_SELECTION_ACTIVITY_SCHEMA_VERSION {
        return Err(invalid(
            "tool selection activity has an unknown schema version",
        ));
    }
    if activity.called_tools.len() > MAX_TOOL_SELECTION_CALLED_TOOLS {
        return Err(invalid("tool selection activity lists too many tools"));
    }
    for name in &activity.called_tools {
        validate_tool_name(name)?;
    }
    if let Some(mark) = &activity.last_model_call {
        validate_model_call_mark(mark)?;
    }
    Ok(())
}

fn validate_tool_name(name: &str) -> Result<(), SessionThreadError> {
    if name.is_empty() || name.len() > MAX_TOOL_NAME_BYTES {
        return Err(invalid("tool selection activity names an invalid tool"));
    }
    Ok(())
}

fn validate_model_call_mark(mark: &ModelCallMark) -> Result<(), SessionThreadError> {
    if mark
        .model
        .as_ref()
        .is_some_and(|model| model.is_empty() || model.len() > MAX_MODEL_IDENTITY_BYTES)
    {
        return Err(invalid("tool selection activity model identity is invalid"));
    }
    Ok(())
}

fn invalid(reason: &str) -> SessionThreadError {
    SessionThreadError::InvalidToolSelection {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{AgentId, TenantId};

    fn scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            agent_id: AgentId::new("agent").expect("agent"),
            project_id: None,
            owner_user_id: None,
            mission_id: None,
        }
    }

    fn entry(
        reason: ToolSelectionReason,
        turn_id: TurnId,
        message_sequence: Option<u64>,
        advertised: &[&str],
    ) -> ToolSelectionEntry {
        ToolSelectionEntry {
            effective_from: ToolSelectionEffectiveFrom {
                turn_id,
                message_sequence,
            },
            reason,
            advertised: advertised.iter().map(|name| name.to_string()).collect(),
            scores: Vec::new(),
            ranker_version: None,
            tool_search_description: None,
            fallback_reason: None,
            recorded_at: Utc::now(),
        }
    }

    fn request(
        expected_entries: usize,
        entry: ToolSelectionEntry,
    ) -> AppendToolSelectionEntryRequest {
        AppendToolSelectionEntryRequest {
            scope: scope(),
            thread_id: ThreadId::new("thread").expect("thread"),
            expected_entries,
            entry,
        }
    }

    #[test]
    fn history_starts_with_one_initial_entry_and_later_entries_must_change_the_list() {
        let first = TurnId::new();
        let history = append_tool_selection_entry(
            None,
            &request(
                0,
                entry(ToolSelectionReason::Initial, first, Some(1), &["a", "b"]),
            ),
        )
        .expect("initial entry");

        let second_initial = append_tool_selection_entry(
            Some(history.clone()),
            &request(
                1,
                entry(ToolSelectionReason::Initial, TurnId::new(), Some(3), &["a"]),
            ),
        )
        .expect_err("second initial entry");
        assert_eq!(second_initial.kind_name(), "invalid_tool_selection");

        let widening_only = append_tool_selection_entry(
            Some(history.clone()),
            &request(
                1,
                entry(
                    ToolSelectionReason::Revoked,
                    TurnId::new(),
                    Some(3),
                    &["a", "b", "c"],
                ),
            ),
        )
        .expect_err("a revocation that removes nothing");
        assert_eq!(widening_only.kind_name(), "invalid_tool_selection");

        // A revocation that re-selects at the same moment drops the revoked
        // tool and may add others.
        let reselected = append_tool_selection_entry(
            Some(history.clone()),
            &request(
                1,
                entry(
                    ToolSelectionReason::Revoked,
                    TurnId::new(),
                    Some(3),
                    &["a", "c"],
                ),
            ),
        )
        .expect("a revocation that removes a tool and re-selects");
        assert_eq!(reselected.entries[1].advertised, ["a", "c"]);

        for reason in [
            ToolSelectionReason::CacheCold,
            ToolSelectionReason::ModelChange,
        ] {
            let unchanged = append_tool_selection_entry(
                Some(history.clone()),
                &request(1, entry(reason, TurnId::new(), Some(5), &["a", "b"])),
            )
            .expect_err("a re-selection that changes nothing");
            assert_eq!(unchanged.kind_name(), "invalid_tool_selection");
            let changed = append_tool_selection_entry(
                Some(history.clone()),
                &request(1, entry(reason, TurnId::new(), Some(5), &["b", "d"])),
            )
            .expect("a re-selection may add and remove tools");
            assert_eq!(changed.entries[1].reason, reason);
        }

        let narrowed = append_tool_selection_entry(
            Some(history),
            &request(
                1,
                entry(ToolSelectionReason::Revoked, TurnId::new(), Some(3), &["b"]),
            ),
        )
        .expect("a revocation that removes a tool");
        assert_eq!(narrowed.entries.len(), 2);
        assert_eq!(narrowed.entries[0].advertised, ["a", "b"]);
    }

    #[test]
    fn history_must_start_with_initial_and_rejects_stale_writers() {
        let revoked_first = append_tool_selection_entry(
            None,
            &request(
                0,
                entry(ToolSelectionReason::Revoked, TurnId::new(), None, &[]),
            ),
        )
        .expect_err("revoked first");
        assert_eq!(revoked_first.kind_name(), "invalid_tool_selection");

        let history = append_tool_selection_entry(
            None,
            &request(
                0,
                entry(ToolSelectionReason::Initial, TurnId::new(), None, &["a"]),
            ),
        )
        .expect("initial");
        let stale = append_tool_selection_entry(
            Some(history),
            &request(
                0,
                entry(ToolSelectionReason::Initial, TurnId::new(), None, &["b"]),
            ),
        )
        .expect_err("stale writer");
        assert!(matches!(
            stale,
            SessionThreadError::ToolSelectionHistoryConflict {
                expected_entries: 0,
                actual_entries: 1,
                ..
            }
        ));
    }

    #[test]
    fn entries_reject_duplicates_unscored_names_and_invalid_scores() {
        let mut duplicate = entry(
            ToolSelectionReason::Initial,
            TurnId::new(),
            None,
            &["a", "a"],
        );
        assert!(duplicate.validate().is_err());
        duplicate.advertised = vec!["a".to_string()];
        duplicate.scores = vec![ToolSelectionScore {
            name: "b".to_string(),
            score: 1.0,
        }];
        assert!(
            duplicate.validate().is_err(),
            "scored tool is not advertised"
        );
        duplicate.scores = vec![ToolSelectionScore {
            name: "a".to_string(),
            score: f32::NAN,
        }];
        assert!(duplicate.validate().is_err(), "non-finite score");
        duplicate.scores = vec![ToolSelectionScore {
            name: "a".to_string(),
            score: 0.5,
        }];
        assert!(duplicate.validate().is_ok());
        for bad in ["", "Timeout", "has space", &"x".repeat(65)] {
            duplicate.fallback_reason = Some(bad.to_string());
            assert!(duplicate.validate().is_err(), "fallback reason {bad:?}");
        }
        duplicate.fallback_reason = Some("rate_limited".to_string());
        assert!(duplicate.validate().is_ok());
    }

    #[test]
    fn entries_written_before_the_fallback_field_still_read() {
        let mut current = entry(ToolSelectionReason::Initial, TurnId::new(), None, &["a"]);
        let mut stored = serde_json::to_value(&current).expect("encode");
        assert!(
            stored.get("fallback_reason").is_none(),
            "an unset reason is not written"
        );
        stored
            .as_object_mut()
            .expect("object")
            .remove("fallback_reason");
        let read: ToolSelectionEntry = serde_json::from_value(stored).expect("decode");
        assert_eq!(read, current);

        current.fallback_reason = Some("timeout".to_string());
        let round_trip: ToolSelectionEntry =
            serde_json::from_value(serde_json::to_value(&current).expect("encode"))
                .expect("decode");
        assert_eq!(round_trip.fallback_reason.as_deref(), Some("timeout"));
    }

    #[test]
    fn in_force_entry_is_resolved_by_turn_then_by_message_order() {
        let opening = TurnId::new();
        let later = TurnId::new();
        let revoking = TurnId::new();
        let history = ToolSelectionHistory {
            schema_version: TOOL_SELECTION_HISTORY_SCHEMA_VERSION,
            scope: scope(),
            thread_id: ThreadId::new("thread").expect("thread"),
            entries: vec![
                entry(ToolSelectionReason::Initial, opening, Some(1), &["a", "b"]),
                entry(ToolSelectionReason::Revoked, revoking, Some(7), &["a"]),
            ],
        };

        assert_eq!(
            history.in_force_at(opening, Some(1)).map(|e| e.reason),
            Some(ToolSelectionReason::Initial)
        );
        // A turn between the two entries replays the initial list.
        assert_eq!(
            history
                .in_force_at(later, Some(4))
                .map(|e| e.advertised.len()),
            Some(2)
        );
        // A turn after the revocation sees the narrowed list.
        assert_eq!(
            history
                .in_force_at(TurnId::new(), Some(9))
                .map(|e| e.advertised.len()),
            Some(1)
        );
        // The revoking turn itself resumes with its own entry.
        assert_eq!(
            history.in_force_at(revoking, None).map(|e| e.reason),
            Some(ToolSelectionReason::Revoked)
        );
        // A turn that predates the history has no entry in force.
        assert!(history.in_force_at(TurnId::new(), Some(0)).is_none());
        // Without a sequence to order by, the latest entry is in force.
        assert_eq!(
            history.in_force_at(TurnId::new(), None).map(|e| e.reason),
            Some(ToolSelectionReason::Revoked)
        );
    }

    #[test]
    fn stored_reasons_use_stable_snake_case_labels() {
        for reason in [
            ToolSelectionReason::Initial,
            ToolSelectionReason::Revoked,
            ToolSelectionReason::CacheCold,
            ToolSelectionReason::ModelChange,
        ] {
            let encoded = serde_json::to_string(&reason).expect("encode");
            assert_eq!(encoded, format!("\"{}\"", reason.as_str()));
        }
    }

    #[test]
    fn entries_stored_before_the_new_reasons_still_read() {
        // A history written when only `initial` and `revoked` existed.
        let stored = serde_json::json!({
            "effective_from": {"turn_id": TurnId::new(), "message_sequence": 1},
            "reason": "revoked",
            "advertised": ["a"],
            "recorded_at": "2026-01-01T00:00:00Z"
        });
        let read: ToolSelectionEntry = serde_json::from_value(stored).expect("decode");
        assert_eq!(read.reason, ToolSelectionReason::Revoked);
        assert!(read.fallback_reason.is_none());
    }

    fn activity_request(update: ToolSelectionActivityUpdate) -> RecordToolSelectionActivityRequest {
        RecordToolSelectionActivityRequest {
            scope: scope(),
            thread_id: ThreadId::new("thread").expect("thread"),
            update,
        }
    }

    fn mark(seconds: i64, model: Option<&str>) -> ModelCallMark {
        ModelCallMark {
            turn_id: TurnId::new(),
            called_at: DateTime::<Utc>::from_timestamp(seconds, 0).expect("time"),
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn activity_keeps_the_latest_model_call_and_each_called_tool_once() {
        let later = mark(2_000, Some("anthropic/claude"));
        let activity = record_tool_selection_activity(
            None,
            &activity_request(ToolSelectionActivityUpdate::ModelCall(later.clone())),
        )
        .expect("first mark");
        // A stale writer never moves the clock back.
        let activity = record_tool_selection_activity(
            Some(activity),
            &activity_request(ToolSelectionActivityUpdate::ModelCall(mark(1_000, None))),
        )
        .expect("stale mark");
        assert_eq!(activity.last_model_call.as_ref(), Some(&later));

        let mut activity = activity;
        for name in [
            "github__create_issue",
            "calendar__list",
            "github__create_issue",
        ] {
            activity = record_tool_selection_activity(
                Some(activity),
                &activity_request(ToolSelectionActivityUpdate::CalledTool(name.to_string())),
            )
            .expect("called tool");
        }
        assert_eq!(
            activity.called_tools,
            ["github__create_issue", "calendar__list"]
        );
        let invalid = record_tool_selection_activity(
            Some(activity.clone()),
            &activity_request(ToolSelectionActivityUpdate::CalledTool(String::new())),
        )
        .expect_err("empty tool name");
        assert_eq!(invalid.kind_name(), "invalid_tool_selection");
        validate_stored_activity(
            &activity,
            &scope(),
            &ThreadId::new("thread").expect("thread"),
        )
        .expect("valid stored activity");
    }
}
