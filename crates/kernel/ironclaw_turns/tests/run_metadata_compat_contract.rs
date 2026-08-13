//! Rolling-compatibility pins for the durable agent-turn metadata reader.
//!
//! Detached turns (docs/internal/design/2026-08-12-detached-turns.md §4.1)
//! optionalize the binding refs on the durable `agent_turn` process metadata
//! and introduce `detached_*` run-profile ids. These pins capture TODAY's
//! reader posture, before that change lands, so the change cannot silently
//! alter it:
//!
//! 1. A metadata row missing the binding-ref keys is rejected fail-closed
//!    (typed `TurnError`, never a panic and never a half-populated state).
//!    This is exactly how a pre-change ("old-style") reader treats the rows
//!    a post-change writer will produce — the rolling-compat story for the
//!    optionalization. When the refs become `Option`, this test is expected
//!    to move onto a frozen legacy-shaped struct so the old-reader proof
//!    survives the change.
//! 2. Run-profile ids are opaque strings at read time: a row carrying an id
//!    this build has never heard of (e.g. a future `detached_structured`)
//!    still rehydrates — old readers must not choke on new profile ids.
//! 3. `TurnRunProfile` rows persisted before the `resolved` snapshot existed
//!    deserialize through `ResolvedRunProfile::legacy_compatibility` (the
//!    lenient wire path had no direct pin until now).

use chrono::Utc;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId};
use ironclaw_processes::{JournaledProcessSnapshot, ProcessJournalCursor, ProcessKind};
use ironclaw_turns::{
    TurnError, TurnRunId, TurnRunProfile, TurnScope, TurnStatus,
    process_projection::process_id_from_turn_run_id, turn_run_state_from_process_snapshot,
};
use serde_json::json;

fn scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-metadata-compat").expect("tenant"),
        Some(AgentId::new("agent-metadata-compat").expect("agent")),
        Some(ProjectId::new("project-metadata-compat").expect("project")),
        ThreadId::new("thread-metadata-compat").expect("thread"),
    )
}

fn snapshot_with_agent_turn_metadata(metadata: serde_json::Value) -> JournaledProcessSnapshot {
    JournaledProcessSnapshot {
        process_id: process_id_from_turn_run_id(TurnRunId::new()),
        process_kind: ProcessKind::AgentTurn,
        scope: scope().to_resource_scope(),
        status: ironclaw_processes::ProcessLifecycleStatus::Queued,
        suspension: None,
        checkpoint_ref: None,
        checkpoint_kind: None,
        input_ref: None,
        failure: None,
        journal_cursor: ProcessJournalCursor(1),
        lease: None,
        crash_reclaim_count: 0,
        created_at: Utc::now(),
        owner_user_id: None,
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        metadata: json!({ "agent_turn": metadata }),
    }
}

fn complete_metadata() -> serde_json::Value {
    json!({
        "turn_id": serde_json::to_value(ironclaw_turns::TurnId::new()).expect("turn id json"),
        "accepted_message_ref": "accepted-metadata-compat",
        "source_binding_ref": "source-metadata-compat",
        "reply_target_binding_ref": "reply-metadata-compat",
        "resolved_run_profile_id": "reborn-planned-default",
        "resolved_run_profile_version": 1,
        "allow_steering": true,
        "subagent_depth": 0
    })
}

/// Pin 1: the current reader fails CLOSED — a typed `TurnError`, not a panic
/// and not a partially-populated `TurnRunState` — when the binding-ref keys
/// are absent from a durable `agent_turn` metadata row.
#[test]
fn metadata_rows_missing_binding_refs_fail_closed_in_the_current_reader() {
    for missing_key in ["source_binding_ref", "reply_target_binding_ref"] {
        let mut metadata = complete_metadata();
        metadata
            .as_object_mut()
            .expect("metadata object")
            .remove(missing_key);

        let result = turn_run_state_from_process_snapshot(snapshot_with_agent_turn_metadata(
            metadata.clone(),
        ));
        match result {
            Err(TurnError::InvalidRequest { reason }) => {
                assert!(
                    reason.contains("agent-turn process metadata"),
                    "rejection must identify the metadata reader, got: {reason}"
                );
            }
            other => panic!(
                "row missing {missing_key} must fail closed with InvalidRequest, got {other:?}"
            ),
        }
    }
}

/// Pin 2: run-profile ids are opaque at read time. A row stamped with a
/// profile id this build has never registered (the shape a newer binary's
/// detached submissions will write) still rehydrates into a full run state —
/// unknown profile ids must never brick `get_run_state` for an old reader.
#[test]
fn metadata_rows_with_unknown_profile_ids_still_rehydrate() {
    let mut metadata = complete_metadata();
    metadata.as_object_mut().expect("metadata object").insert(
        "resolved_run_profile_id".into(),
        json!("detached_structured"),
    );

    let state = turn_run_state_from_process_snapshot(snapshot_with_agent_turn_metadata(metadata))
        .expect("unknown profile id must not fail rehydration");
    assert_eq!(
        state.resolved_run_profile_id.as_str(),
        "detached_structured"
    );
    assert_eq!(state.status, TurnStatus::Queued);
    assert_eq!(
        state.source_binding_ref.as_str(),
        "source-metadata-compat",
        "the rest of the row must survive an unknown profile id"
    );
}

/// Pin 3: `TurnRunProfile` rows persisted before the `resolved` snapshot
/// existed deserialize through the legacy-compatibility backfill. The lenient
/// wire path (`Deserialize for TurnRunProfile`) previously had no direct pin.
#[test]
fn turn_run_profile_without_resolved_snapshot_deserializes_via_legacy_compatibility() {
    let profile: TurnRunProfile = serde_json::from_value(json!({
        "id": "default",
        "version": 1,
        "allow_steering": false,
        "auto_queue_followups": false,
    }))
    .expect("legacy profile rows without a resolved snapshot must deserialize");

    assert_eq!(profile.id.as_str(), "default");
    assert!(!profile.allow_steering);
    assert!(
        !profile.resolved.steering_policy.allow_steering,
        "the backfilled resolved snapshot must mirror the persisted allow_steering"
    );
}

/// Pin 3b: an unknown `admission_class` wire value... does not exist — the
/// field defaults when ABSENT. Pin the absence default so a future variant
/// addition cannot silently break old rows.
#[test]
fn turn_run_profile_defaults_admission_class_when_absent() {
    let profile: TurnRunProfile = serde_json::from_value(json!({
        "id": "reborn-planned-default",
        "version": 1,
        "allow_steering": true,
        "auto_queue_followups": false,
    }))
    .expect("rows without admission_class must deserialize");
    assert_eq!(profile.id.as_str(), "reborn-planned-default");
}
