//! Wire-stable subagent result and tombstone payloads.

use ironclaw_host_api::ThreadId;
use ironclaw_loop_support::SubagentKindId;
use ironclaw_turns::{EventCursor, TurnRunId, TurnStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpawnedChildRunPayload {
    pub child_run_id: TurnRunId,
    pub child_thread_id: ThreadId,
    #[serde(rename = "flavor")]
    pub subagent_kind: SubagentKindId,
    pub mode: SubagentSpawnMode,
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
pub enum SubagentSpawnMode {
    Blocking,
    Background,
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
            subagent_kind: SubagentKindId::new("researcher").unwrap(),
            mode: SubagentSpawnMode::Background,
            status: SubagentSpawnStatus::Spawned,
            output_available: false,
            final_text: None,
            failure_summary: None,
            terminal_event: None,
        };

        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["flavor"], "researcher");
        assert_eq!(value["mode"], "background");
        assert_eq!(value["status"], "spawned");
        assert!(value.get("final_text").is_none());
        assert_eq!(
            serde_json::from_value::<SpawnedChildRunPayload>(value).unwrap(),
            payload
        );
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
