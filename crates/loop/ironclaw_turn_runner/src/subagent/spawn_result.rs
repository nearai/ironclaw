//! Wire-stable subagent result and tombstone payloads.

use ironclaw_host_api::ids::ThreadId;
use ironclaw_loop_host::{SpawnSubagentMode, SubagentKindId};
use ironclaw_turns::{EventCursor, TurnRunId, TurnStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpawnedChildRunPayload {
    pub child_run_id: TurnRunId,
    pub child_thread_id: ThreadId,
    #[serde(rename = "flavor")]
    pub subagent_kind: SubagentKindId,
    pub mode: SpawnSubagentMode,
    pub status: SubagentSpawnStatus,
    pub output_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_event: Option<SubagentTerminalEventPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentSpawnStatus {
    Spawned,
    Completed,
    Failed,
    Cancelled,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubagentTerminalEventPayload {
    pub kind: SubagentTerminalEventKind,
    pub cursor: EventCursor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentTerminalEventKind {
    Submitted,
    Resumed,
    RunnerClaimed,
    RunnerHeartbeat,
    RecoveryRequired,
    Blocked,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubagentResultTombstone {
    pub child_run_id: TurnRunId,
    pub terminal_status: TurnStatus,
    pub disposition: SubagentResultDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentResultDisposition {
    DiscardedByParentCancel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawned_child_run_payload_uses_snake_case_wire_shape() {
        let payload = SpawnedChildRunPayload {
            child_run_id: TurnRunId::new(),
            child_thread_id: ThreadId::new("child-thread-1").unwrap(),
            subagent_kind: SubagentKindId::new("planner").unwrap(),
            mode: SpawnSubagentMode::Background,
            status: SubagentSpawnStatus::Spawned,
            output_available: false,
            final_text: None,
            failure_summary: None,
            terminal_event: None,
        };

        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["flavor"], "planner");
        assert_eq!(value["mode"], "background");
        assert_eq!(value["status"], "spawned");
        assert!(value.get("final_text").is_none());
        assert_eq!(
            serde_json::from_value::<SpawnedChildRunPayload>(value).unwrap(),
            payload
        );
    }

    /// Both spawn-mode wire strings are model-visible in the spawn-result
    /// payload. Pinned as an explicit round-trip so a change of the mode type
    /// cannot silently alter or drop either emitted string.
    #[test]
    fn spawn_mode_wire_strings_round_trip_for_every_variant() {
        for (mode, expected) in [
            (SpawnSubagentMode::Blocking, "blocking"),
            (SpawnSubagentMode::Background, "background"),
        ] {
            let encoded = serde_json::to_value(mode).unwrap();
            assert_eq!(encoded, expected, "emitted wire string for {mode:?}");
            assert_eq!(
                serde_json::from_value::<SpawnSubagentMode>(encoded).unwrap(),
                mode,
                "round-trip for {mode:?}"
            );
        }
    }

    #[test]
    fn subagent_result_tombstone_uses_typed_disposition() {
        let tombstone = SubagentResultTombstone {
            child_run_id: TurnRunId::new(),
            terminal_status: TurnStatus::Completed,
            disposition: SubagentResultDisposition::DiscardedByParentCancel,
        };

        let value = serde_json::to_value(&tombstone).unwrap();
        assert_eq!(value["disposition"], "discarded_by_parent_cancel");
        assert_eq!(
            serde_json::from_value::<SubagentResultTombstone>(value).unwrap(),
            tombstone
        );
    }
}
