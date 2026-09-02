//! Tests for `run_completion_observer`; a sibling file so the composition mass gate
//! counts production lines only (`scripts/ci/composition-budget.toml`).

use super::*;
use chrono::Utc;
use ironclaw_host_api::ids::{AgentId, ProcessId, TenantId, ThreadId, UserId};
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_processes::{
    JournaledProcessSnapshot, ProcessJournalCursor, ProcessKind, ProcessLifecycleStatus,
};

fn user(id: &str) -> UserId {
    UserId::new(id).expect("user id")
}

/// A terminal successful top-level agent turn on an owner-visible thread:
/// the one shape that must produce an observation.
fn eligible_commit() -> ProcessJournalCommit {
    let now = Utc::now();
    ProcessJournalCommit {
        state: JournaledProcessSnapshot {
            process_id: ProcessId::from_uuid(uuid::Uuid::new_v4()),
            process_kind: ProcessKind::AgentTurn,
            scope: ResourceScope {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                user_id: user("user-alpha"),
                agent_id: Some(AgentId::new("agent-alpha").expect("agent")),
                project_id: None,
                mission_id: None,
                thread_id: Some(ThreadId::new("thread-alpha").expect("thread")),
                invocation_id: ironclaw_host_api::ids::InvocationId::new(),
            },
            status: ProcessLifecycleStatus::Completed,
            suspension: None,
            checkpoint_ref: None,
            checkpoint_kind: None,
            input_ref: None,
            failure: None,
            journal_cursor: ProcessJournalCursor(1),
            lease: None,
            crash_reclaim_count: 0,
            created_at: now,
            owner_user_id: Some(user("user-alpha")),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            metadata: serde_json::Value::Null,
        },
        kind: ProcessJournalKind::Completed,
        sanitized_reason: None,
        occurred_at: Some(now),
    }
}

#[test]
fn only_completed_top_level_owned_thread_turns_are_observed() {
    let eligible = eligible_commit();
    let observation = RunCompletionJournalObserver::observation(&eligible)
        .expect("a completed top-level owned turn notifies");
    assert_eq!(
        observation.run_id.as_uuid(),
        eligible.state.process_id.as_uuid(),
        "the run identity is the process identity"
    );
    assert_eq!(observation.owner_user_id.as_str(), "user-alpha");
    assert_eq!(observation.scope.thread_id.as_str(), "thread-alpha");
    assert_eq!(
        observation.completed_at,
        eligible.occurred_at.expect("journal instant"),
        "the journal's own occurrence instant is the completion time"
    );

    // Every exclusion the observer screens on, each one alone.
    let mut stopped = eligible_commit();
    stopped.kind = ProcessJournalKind::Stopped;
    let mut not_a_turn = eligible_commit();
    not_a_turn.state.process_kind = ProcessKind::CapabilityInvocation;
    let mut subagent = eligible_commit();
    subagent.state.parent_process_id = Some(ProcessId::from_uuid(uuid::Uuid::new_v4()));
    let mut nested = eligible_commit();
    nested.state.root_process_id = Some(ProcessId::from_uuid(uuid::Uuid::new_v4()));
    let mut ownerless = eligible_commit();
    ownerless.state.owner_user_id = None;
    let mut system_owned = eligible_commit();
    system_owned.state.owner_user_id = Some(ResourceScope::system().user_id);
    let mut system_scope = eligible_commit();
    system_scope.state.scope.user_id = ResourceScope::system().user_id;
    let mut threadless = eligible_commit();
    threadless.state.scope.thread_id = None;
    for (label, commit) in [
        ("stopped turns never notify", stopped),
        ("non-turn processes never notify", not_a_turn),
        ("subagent turns never notify", subagent),
        ("nested turns never notify", nested),
        ("ownerless turns never notify", ownerless),
        ("system-owned turns never notify", system_owned),
        ("system-scoped turns never notify", system_scope),
        ("thread-less turns never notify", threadless),
    ] {
        assert!(
            RunCompletionJournalObserver::observation(&commit).is_none(),
            "{label}"
        );
    }
}

#[test]
fn older_commits_without_an_occurrence_instant_use_snapshot_creation_time() {
    let mut commit = eligible_commit();
    commit.occurred_at = None;
    let observation = RunCompletionJournalObserver::observation(&commit).expect("still eligible");
    assert_eq!(observation.completed_at, commit.state.created_at);
}
