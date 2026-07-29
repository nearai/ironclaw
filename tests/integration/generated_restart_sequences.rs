//! Generated restart sequences over a durable approval-gated run (#6524 WS9).
//!
//! Unlike the independent store-reopen probes, these cases gracefully stop the
//! old scheduler and rebuild the coordinator, executor, scheduler, scope
//! gateway, checkpoint adapters, and process journal over a fresh LibSQL connection.
//! The post-restart action is generated from the supported gate-resolution
//! classes so every recovery arm is exercised reproducibly.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_turns::{TurnRunId, TurnStatus};
use reborn_support::builder::StorageMode;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const GATED_CAPABILITY: &str = "builtin.write_file";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostRestartAction {
    Approve,
    Deny,
    Cancel,
}

impl PostRestartAction {
    const ALL: [Self; 3] = [Self::Approve, Self::Deny, Self::Cancel];

    fn index(self) -> usize {
        match self {
            Self::Approve => 0,
            Self::Deny => 1,
            Self::Cancel => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RestartSequence {
    action: PostRestartAction,
}

fn restart_sequences() -> Vec<RestartSequence> {
    PostRestartAction::ALL
        .into_iter()
        .map(|action| RestartSequence { action })
        .collect()
}

fn validate_restart_sequences(sequences: &[RestartSequence]) -> Result<(), String> {
    let mut seen = [false; PostRestartAction::ALL.len()];
    for sequence in sequences {
        let slot = &mut seen[sequence.action.index()];
        if *slot {
            return Err(format!(
                "duplicate generated restart action: {:?}",
                sequence.action
            ));
        }
        *slot = true;
    }
    let missing: Vec<_> = PostRestartAction::ALL
        .into_iter()
        .filter(|action| !seen[action.index()])
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing generated restart actions: {missing:?}"))
    }
}

async fn build_restarted_harness(
    case_index: usize,
) -> (
    reborn_support::builder::RebornIntegrationHarness,
    TurnRunId,
    ironclaw_turns::GateRef,
) {
    let group = RebornIntegrationGroup::builder()
        .storage(StorageMode::LibSql)
        .live_approvals()
        .await
        .expect("durable live-approvals group builds");
    let conversation_id = format!("generated-restart-{case_index}");
    let before_restart = group
        .thread(&conversation_id)
        .script([
            RebornScriptedReply::tool_call(
                GATED_CAPABILITY,
                json!({
                    "path": format!("/workspace/generated-restart-{case_index}.txt"),
                    "content": "generated across restart"
                }),
            ),
            RebornScriptedReply::text("unreachable on the old runtime"),
        ])
        .build()
        .await
        .expect("pre-restart thread builds");
    let (run_id, gate_ref) = before_restart
        .submit_turn_until_blocked("write after a runtime restart")
        .await
        .expect("run parks before restart");
    before_restart
        .assert_no_orphan_runs_or_reservations(&[run_id])
        .await
        .expect("parked run owns every process and releases capability holds");

    // Dropping this is part of the contract: a live thread harness owns the old
    // coordinator. `restart_planned_runtime` rejects that dishonest shape.
    drop(before_restart);
    let restarted_group = group
        .restart_planned_runtime()
        .await
        .expect("planned runtime restarts over durable state");
    let after_restart = restarted_group
        .thread(conversation_id)
        // The first model reply was consumed before the restart. Resuming an
        // approved run reaches its next model turn through this newly-built
        // scope gateway.
        .script([RebornScriptedReply::text("done after restart")])
        .build()
        .await
        .expect("post-restart thread rebuilds the scope gateway");
    let state = after_restart
        .run_state(run_id)
        .await
        .expect("restarted process journal reads the durable run");
    assert_eq!(
        state.status,
        TurnStatus::BlockedApproval,
        "restart changed the parked lifecycle state"
    );
    assert_eq!(
        state.gate_ref.as_ref(),
        Some(&gate_ref),
        "restart changed the durable gate identity"
    );
    after_restart
        .assert_no_orphan_runs_or_reservations(&[run_id])
        .await
        .expect("restart preserves run and reservation ownership");
    (after_restart, run_id, gate_ref)
}

#[tokio::test]
async fn generated_restart_sequences_preserve_gate_lifecycle_and_effect_count() {
    let sequences = restart_sequences();
    validate_restart_sequences(&sequences)
        .unwrap_or_else(|error| panic!("restart generator is incomplete: {error}"));

    for (case_index, sequence) in sequences.into_iter().enumerate() {
        let (harness, run_id, gate_ref) = build_restarted_harness(case_index).await;
        match sequence.action {
            PostRestartAction::Approve => {
                harness
                    .approve_gate(run_id, &gate_ref)
                    .await
                    .expect("approval resolves after restart");
            }
            PostRestartAction::Deny => {
                harness
                    .deny_gate(run_id, &gate_ref)
                    .await
                    .expect("denial resolves after restart");
            }
            PostRestartAction::Cancel => {
                harness
                    .cancel_run(run_id)
                    .await
                    .expect("cancellation resolves after restart");
            }
        }

        let terminal = harness
            .wait_for_terminal(run_id)
            .await
            .expect("post-restart action reaches a terminal state");
        harness
            .assert_no_orphan_runs_or_reservations(&[run_id])
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{:?} left an orphan after restart: {error}",
                    sequence.action
                )
            });
        match sequence.action {
            PostRestartAction::Approve => {
                assert_eq!(terminal.status, TurnStatus::Completed);
                harness
                    .assert_capability_result_count(GATED_CAPABILITY, 1)
                    .await
                    .expect("approved effect executes exactly once after restart");
            }
            PostRestartAction::Deny => {
                // Denial is delivered back to the loop as a model-visible tool
                // outcome, so the conversation itself may complete normally.
                // Effect evidence, not terminal status, proves the write stayed
                // denied.
                assert_eq!(terminal.status, TurnStatus::Completed);
                harness
                    .assert_capability_result_count(GATED_CAPABILITY, 0)
                    .await
                    .expect("denied effect stays unexecuted after restart");
            }
            PostRestartAction::Cancel => {
                assert_eq!(terminal.status, TurnStatus::Cancelled);
                harness
                    .assert_capability_result_count(GATED_CAPABILITY, 0)
                    .await
                    .expect("cancelled effect stays unexecuted after restart");
            }
        }
    }
}

#[test]
fn restart_generator_sabotage_detects_a_missing_recovery_arm() {
    let mut sabotaged = restart_sequences();
    sabotaged.retain(|sequence| sequence.action != PostRestartAction::Deny);
    let error = validate_restart_sequences(&sabotaged)
        .expect_err("removing a recovery arm must fail the generator denominator");
    assert!(
        error.contains("Deny"),
        "missing-arm diagnostic did not name the sabotaged branch: {error}"
    );
}
