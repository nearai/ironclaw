//! Generated lifecycle sequences over a gated run (#6524 workstream 9).
//!
//! The hand-written gate tests each drive one path: approve, or deny, or
//! cancel. The transitions nobody writes by hand are the ones that go wrong —
//! resolving a gate twice, cancelling something already finished, approving a
//! run that was cancelled a moment earlier. Those are exactly the races a real
//! client produces by double-clicking or retrying.
//!
//! This enumerates every ordering of a small action alphabet rather than
//! sampling randomly. Enumeration is reproducible, and at this size it is also
//! complete, so a failure names the exact sequence instead of a seed. That is
//! the "representative equivalence classes rather than the full Cartesian
//! product" the workstream asks for: the alphabet is the equivalence class,
//! and short orderings of it cover the interesting adjacencies.
//!
//! Invariants asserted after EVERY transition, not just at the end:
//!   1. a terminal run never becomes non-terminal again — which covers
//!      re-parking a finished run, since every gate status is non-terminal;
//!   2. a cancelled run never later reports Completed;
//!   3. every sequence lands terminal.
//!
//! Actions are applied unconditionally rather than only when "legal": refusing
//! an action that no longer applies is the behaviour under test, so the run
//! must survive being approved after it was cancelled.
//!
//! An earlier draft carried a fourth assertion for "a finished run returned to
//! a gate". Its self-test showed invariant 1 always fires first, because a
//! gate status is never terminal — so it could not fail on its own and was
//! removed rather than left as reassuring dead weight.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use ironclaw_assistant::ProductInboundAck;
use ironclaw_turns::{TurnRunId, TurnStatus};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::ParkingModelGate;
use serde_json::json;

/// The capability the gate guards. Named once so the effect-count assertion
/// and the scripted call cannot drift apart.
const GATED_CAPABILITY: &str = "builtin.write_file";

// The complete seven-axis denominator and its selected pairwise crossings live
// in `tests/e2e/state_machine_coverage.py`. This target supplies the executable
// lifecycle, actor-isolation, double-submit, and orphan-invariant evidence;
// channel/provider axes remain mapped to their owning whole-path suites.

/// One dimension of workstream 9's lifecycle axis: what a client can do to a
/// run parked on an approval gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateAction {
    Approve,
    Deny,
    Cancel,
}

// Deliberately three actions, not four. An earlier `ApproveAgain` variant
// meant to model a double-clicked approve button, but it dispatched the
// identical call with the identical gate ref -- so `[Approve, Approve]`
// already produces that exact shape, and the variant only multiplied the
// sequence space. Depth 2 drops from 20 sequences to 12.
const ALPHABET: [GateAction; 3] = [GateAction::Approve, GateAction::Deny, GateAction::Cancel];

/// Every ordering of `ALPHABET` up to `max_len`, shortest first.
fn sequences(max_len: usize) -> Vec<Vec<GateAction>> {
    let mut out: Vec<Vec<GateAction>> = Vec::new();
    let mut frontier: Vec<Vec<GateAction>> = vec![Vec::new()];
    for _ in 0..max_len {
        let mut next = Vec::new();
        for prefix in &frontier {
            for action in ALPHABET {
                let mut candidate = prefix.clone();
                candidate.push(action);
                next.push(candidate);
            }
        }
        out.extend(next.iter().cloned());
        frontier = next;
    }
    out
}

struct Observed {
    statuses: Vec<TurnStatus>,
}

impl Observed {
    fn record(&mut self, status: TurnStatus, sequence: &[GateAction], step: usize) {
        // (2) is checked BEFORE (1) on purpose. Equality below subsumes it,
        // so leaving (1) first made this assertion unreachable and its
        // self-test failed on the other rule's message -- an assertion that
        // can never fire is worse than no assertion, because it reads as
        // coverage.
        //
        // (2) cancellation is not silently overridden by a later completion.
        if self.statuses.contains(&TurnStatus::Cancelled) {
            assert_ne!(
                status,
                TurnStatus::Completed,
                "{sequence:?} step {step}: a cancelled run reported Completed"
            );
        }
        if let Some(previous) = self.statuses.last().copied() {
            // (1) terminal is absorbing -- meaning the status stops changing,
            // not merely that it stays somewhere in the terminal set. The
            // weaker `status.is_terminal()` form accepted `Completed -> Failed`
            // and `Failed -> Cancelled`, which are as illegal as returning to
            // an active state: all four statuses are terminal, and a run that
            // finished does not finish differently later.
            assert!(
                !previous.is_terminal() || status == previous,
                "{sequence:?} step {step}: {previous:?} -> {status:?} changed \
                 after reaching a terminal state"
            );
        }
        self.statuses.push(status);
    }
}

async fn run_sequence(sequence: &[GateAction]) -> usize {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let thread = format!(
        "gen-gate-{}",
        sequence
            .iter()
            .map(|action| format!("{action:?}"))
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase()
    );
    let h = group
        .thread(thread)
        .script([
            RebornScriptedReply::tool_call(
                GATED_CAPABILITY,
                json!({"path": "/workspace/generated.txt", "content": "generated"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("thread builds");

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the generated file")
        .await
        .expect("turn parks on the approval gate");

    let mut observed = Observed {
        statuses: Vec::new(),
    };
    let parked = h
        .run_state(run_id)
        .await
        .expect("parked run is readable")
        .status;
    observed.record(parked, sequence, 0);
    h.assert_no_orphan_runs_or_reservations(&[run_id])
        .await
        .unwrap_or_else(|err| panic!("{sequence:?} step 0: orphan invariant failed: {err}"));

    for (step, action) in sequence.iter().enumerate() {
        // Deliberately unconditional: refusing an action that no longer
        // applies is the contract being tested, so the harness result is
        // allowed to be an error. What must never happen is the run coming
        // back to life, which `record` checks below.
        match action {
            GateAction::Approve => {
                let _ = h.approve_gate(run_id, &gate_ref).await;
            }
            GateAction::Deny => {
                let _ = h.deny_gate(run_id, &gate_ref).await;
            }
            GateAction::Cancel => {
                let _ = h.cancel_run(run_id).await;
            }
        }
        let status = h
            .run_state(run_id)
            .await
            .expect("run stays readable after every action")
            .status;
        observed.record(status, sequence, step + 1);
        h.assert_process_ownership(&[run_id])
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "{sequence:?} step {}: process ownership invariant failed: {err}",
                    step + 1
                )
            });
        if status.is_terminal()
            || matches!(
                status,
                TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
            )
        {
            h.assert_no_capability_resource_reservations()
                .unwrap_or_else(|err| {
                    panic!(
                        "{sequence:?} step {}: quiescent reservation invariant failed: {err}",
                        step + 1
                    )
                });
        }
    }

    // (3) every sequence settles. Read through `wait_for_terminal` so a run
    // still converging is given the same grace the product gives it.
    let final_state = h
        .wait_for_terminal(run_id)
        .await
        .unwrap_or_else(|err| panic!("{sequence:?} never reached a terminal state: {err:?}"));
    // Through `record`, not asserted directly. This is the status the
    // invariants most need to see: a run cancelled mid-sequence that settles
    // `Completed` afterwards -- the late-arriving resume -- is exactly what
    // invariant (2) exists for, and it used to reach this point untouched
    // because the checks live in `record`.
    //
    // Asserting `is_terminal()` here would be tautological: `wait_for_terminal`
    // only returns on a terminal status.
    observed.record(final_state.status, sequence, sequence.len());
    if sequence == [GateAction::Deny] {
        assert_eq!(
            final_state.status,
            TurnStatus::Completed,
            "a denied capability outcome is model-visible; the conversation should complete"
        );
    }
    h.assert_no_orphan_runs_or_reservations(&[run_id])
        .await
        .unwrap_or_else(|err| {
            panic!("{sequence:?} terminal observation: orphan invariant failed: {err}")
        });

    // (4) no duplicate confirmed effect. The gated capability writes a file;
    // resolving the same gate twice must not write it twice. This is the
    // invariant a repeated approve attacks, and status alone
    // cannot express it — a double dispatch still ends Completed.
    // Counted by RESULTS, not invocations: the gated attempt that raised the
    // gate is itself recorded as an invocation, so a single approve shows two
    // invocations and one effect. Counting invocations here reported a
    // duplicate that never happened — the first version of this assertion did
    // exactly that, and the run said so.
    let effects = h
        .capability_result_count(GATED_CAPABILITY)
        .await
        .unwrap_or_else(|err| panic!("{sequence:?}: capability results unreadable: {err:?}"));
    assert!(
        effects <= 1,
        "{sequence:?}: gated capability produced {effects} effects; a \
         resolved-twice gate must not perform the effect twice"
    );

    // A sequence that never approves must not perform the effect at all.
    let approved = sequence
        .iter()
        .any(|action| matches!(action, GateAction::Approve));
    if !approved {
        assert_eq!(
            effects, 0,
            "{sequence:?}: capability performed its effect without any approval"
        );
    }
    effects
}

/// How long a sequence to enumerate. Pull requests get depth 2 (12 sequences);
/// the nightly deep lane raises it to depth 3 (39 sequences).
///
/// Parsed strictly rather than with `unwrap_or(2)`: a typo in the workflow
/// would otherwise silently run the shallow lane while the job name still
/// claimed a deep one, which is the failure this epic keeps finding.
const SEQUENCE_DEPTH_ENV: &str = "IRONCLAW_GENERATED_SEQUENCE_DEPTH";
const DEFAULT_SEQUENCE_DEPTH: usize = 2;

fn sequence_depth() -> usize {
    match std::env::var(SEQUENCE_DEPTH_ENV) {
        Err(std::env::VarError::NotPresent) => DEFAULT_SEQUENCE_DEPTH,
        Err(err) => panic!("{SEQUENCE_DEPTH_ENV} is not readable: {err}"),
        Ok(raw) => {
            let depth: usize = raw.trim().parse().unwrap_or_else(|err| {
                panic!("{SEQUENCE_DEPTH_ENV}={raw:?} is not a number: {err}")
            });
            // `sequences` accumulates every level, so depth n is
            // sum(len^1..=len^n), not len^n. The old message named a count the
            // enumerator never produces.
            let sequences_at_depth =
                |n: u32| -> usize { (1..=n).map(|level| ALPHABET.len().pow(level)).sum() };
            assert!(
                (1..=4).contains(&depth),
                "{SEQUENCE_DEPTH_ENV}={depth} is out of range; depth 0 would \
                 test nothing and depth 5 is already {} sequences",
                sequences_at_depth(5)
            );
            depth
        }
    }
}

#[tokio::test]
async fn generated_gate_sequences_preserve_lifecycle_invariants() {
    let depth = sequence_depth();
    let sequences = sequences(depth);
    assert!(
        sequences.len() >= ALPHABET.len(),
        "enumeration produced {} sequences at depth {depth}; an empty or \
         truncated list would pass this test while checking nothing",
        sequences.len()
    );
    eprintln!(
        "generated-gate-sequences: depth {depth}, {} sequences",
        sequences.len()
    );
    let mut performed = 0usize;
    for sequence in sequences {
        performed += run_sequence(&sequence).await;
    }

    // Anti-vacuity: `effects <= 1` is also satisfied by an effect that never
    // happens, which is what a harness misconfiguration would produce. At
    // least one ordering must actually approve and perform the write, or the
    // bound above proves nothing.
    assert!(
        performed > 0,
        "no sequence performed the gated effect; the <=1 bound above would \
         hold vacuously and this suite would be checking nothing"
    );
}

/// Representative scheduler interleavings for two submissions racing on the
/// same durable thread. Both ordered edges matter in addition to a true
/// simultaneous join: only checking one order can hide a payload-correlated
/// winner or an admission check performed outside the atomic store boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DoubleSubmitInterleaving {
    FirstThenSecond,
    SecondThenFirst,
    Simultaneous,
}

const DOUBLE_SUBMIT_INTERLEAVINGS: [DoubleSubmitInterleaving; 3] = [
    DoubleSubmitInterleaving::FirstThenSecond,
    DoubleSubmitInterleaving::SecondThenFirst,
    DoubleSubmitInterleaving::Simultaneous,
];

/// Exactly one submission is admitted as a run; the other is a BUSY
/// acknowledgement naming the winner's run. With queued-message steering
/// wired (the production default), a message hitting the busy thread is
/// ACCEPTED AND QUEUED for the active run — `DeferredBusy` — rather than
/// bounced. An ordered second submit must therefore be `DeferredBusy`; only
/// a true simultaneous race may also settle `RejectedBusy` (the loser can
/// observe the winner mid-admission, before its run is steerable).
fn accepted_and_busy_run(
    left: ProductInboundAck,
    right: ProductInboundAck,
    interleaving: DoubleSubmitInterleaving,
) -> TurnRunId {
    let busy_run_id = |ack: &ProductInboundAck| match ack {
        ProductInboundAck::DeferredBusy { active_run_id, .. } => Some(*active_run_id),
        ProductInboundAck::RejectedBusy {
            active_run_id: Some(active_run_id),
            ..
        } if matches!(interleaving, DoubleSubmitInterleaving::Simultaneous) => Some(*active_run_id),
        _ => None,
    };
    match (&left, &right) {
        (
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            },
            busy,
        )
        | (
            busy,
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            },
        ) if busy_run_id(busy).is_some() => {
            let active_run_id = busy_run_id(busy).expect("guard checked");
            assert_eq!(
                active_run_id, *submitted_run_id,
                "{interleaving:?}: busy acknowledgement named a different active run"
            );
            *submitted_run_id
        }
        _ => panic!(
            "{interleaving:?}: expected exactly one Accepted and one busy \
             (DeferredBusy; RejectedBusy only under a simultaneous race) \
             acknowledgement, got {left:?} and {right:?}"
        ),
    }
}

#[tokio::test]
async fn generated_same_thread_double_submit_interleavings_admit_one_run() {
    for interleaving in DOUBLE_SUBMIT_INTERLEAVINGS {
        let group = RebornIntegrationGroup::live_approvals()
            .await
            .expect("live-approvals group builds");
        let gate = ParkingModelGate::new();
        let workspace_file = format!("double-submit-{interleaving:?}.txt").to_ascii_lowercase();
        let h = group
            .thread(format!("double-submit-{interleaving:?}").to_lowercase())
            .park_model(gate.clone())
            .script([
                RebornScriptedReply::tool_call(
                    GATED_CAPABILITY,
                    json!({
                        "path": format!("/workspace/{workspace_file}"),
                        "content": "one admitted effect"
                    }),
                ),
                RebornScriptedReply::text("winner completes"),
            ])
            .build()
            .await
            .expect("double-submit thread builds");

        let (left, right) = match interleaving {
            DoubleSubmitInterleaving::FirstThenSecond => {
                let left = h
                    .submit_turn_ack("first payload")
                    .await
                    .expect("first submit returns an acknowledgement");
                tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
                    .await
                    .expect("accepted first submit reaches the parked provider");
                let right = h
                    .submit_turn_ack("second payload")
                    .await
                    .expect("second submit returns an acknowledgement");
                (left, right)
            }
            DoubleSubmitInterleaving::SecondThenFirst => {
                let right = h
                    .submit_turn_ack("second payload")
                    .await
                    .expect("second-labeled submit returns an acknowledgement");
                tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
                    .await
                    .expect("accepted second-labeled submit reaches the parked provider");
                let left = h
                    .submit_turn_ack("first payload")
                    .await
                    .expect("first-labeled submit returns an acknowledgement");
                (left, right)
            }
            DoubleSubmitInterleaving::Simultaneous => {
                let acknowledgements = tokio::join!(
                    h.submit_turn_ack("first payload"),
                    h.submit_turn_ack("second payload")
                );
                (
                    acknowledgements
                        .0
                        .expect("simultaneous first submit returns an acknowledgement"),
                    acknowledgements
                        .1
                        .expect("simultaneous second submit returns an acknowledgement"),
                )
            }
        };

        let admitted_run = accepted_and_busy_run(left, right, interleaving);
        tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
            .await
            .expect("admitted run reaches the parked provider");
        h.assert_no_orphan_runs_or_reservations(&[admitted_run])
            .await
            .unwrap_or_else(|err| panic!("{interleaving:?}: active invariant failed: {err}"));

        gate.release();
        let blocked = h
            .wait_for_status(admitted_run, TurnStatus::BlockedApproval)
            .await
            .unwrap_or_else(|err| {
                panic!("{interleaving:?}: admitted run did not reach its effect gate: {err}")
            });
        let gate_ref = blocked
            .gate_ref
            .expect("blocked double-submit run names its approval gate");
        h.assert_no_orphan_runs_or_reservations(&[admitted_run])
            .await
            .unwrap_or_else(|err| {
                panic!("{interleaving:?}: blocked-effect invariant failed: {err}")
            });
        h.approve_gate(admitted_run, &gate_ref)
            .await
            .unwrap_or_else(|err| {
                panic!("{interleaving:?}: admitted effect approval failed: {err}")
            });
        h.wait_for_status(admitted_run, TurnStatus::Completed)
            .await
            .unwrap_or_else(|err| panic!("{interleaving:?}: admitted run did not complete: {err}"));
        h.assert_no_orphan_runs_or_reservations(&[admitted_run])
            .await
            .unwrap_or_else(|err| panic!("{interleaving:?}: terminal invariant failed: {err}"));
        h.assert_capability_result_count(GATED_CAPABILITY, 1)
            .await
            .unwrap_or_else(|err| {
                panic!("{interleaving:?}: admitted run did not emit exactly one effect: {err}")
            });
        h.assert_workspace_file_contains(&workspace_file, "one admitted effect")
            .await
            .unwrap_or_else(|err| {
                panic!("{interleaving:?}: admitted effect read-back failed: {err}")
            });
    }
}

/// The invariant checker is itself checked.
///
/// The sequences above pass, which on its own is also what a checker that
/// asserts nothing would produce. These feed `Observed` the transitions the
/// product must never make and require it to reject them, so a future edit
/// that loosens `record` fails here rather than going quiet.
#[cfg(test)]
mod invariant_checker {
    use super::*;

    fn observed_with(statuses: &[TurnStatus]) -> Observed {
        let mut observed = Observed {
            statuses: Vec::new(),
        };
        for (index, status) in statuses.iter().enumerate() {
            observed.record(*status, &[GateAction::Cancel], index);
        }
        observed
    }

    #[test]
    #[should_panic(expected = "changed after reaching a terminal state")]
    fn rejects_a_terminal_run_becoming_active_again() {
        observed_with(&[TurnStatus::Cancelled, TurnStatus::Running]);
    }

    /// Re-parking a finished run is rejected by invariant 1, not by a rule of
    /// its own: a gate status is never terminal, so leaving the terminal state
    /// is the thing that fires. Pinned so a future split into a separate
    /// assertion does not quietly create an unreachable one.
    #[test]
    #[should_panic(expected = "changed after reaching a terminal state")]
    fn rejects_a_finished_run_returning_to_a_gate() {
        observed_with(&[TurnStatus::Completed, TurnStatus::BlockedApproval]);
    }

    /// The case the earlier `status.is_terminal()` form accepted.
    ///
    /// `Completed -> Failed` keeps the run inside the terminal set, so the
    /// weaker invariant passed it. A run that finished does not finish
    /// differently later, and this is the regression test for that.
    #[test]
    #[should_panic(expected = "changed after reaching a terminal state")]
    fn rejects_one_terminal_state_becoming_another() {
        observed_with(&[TurnStatus::Completed, TurnStatus::Failed]);
    }

    #[test]
    #[should_panic(expected = "a cancelled run reported Completed")]
    fn rejects_a_cancelled_run_completing() {
        observed_with(&[TurnStatus::Cancelled, TurnStatus::Completed]);
    }

    #[test]
    fn default_depth_enumerates_more_than_the_alphabet() {
        // Depth 1 alone would only ever exercise single actions, never an
        // ordering — which is the entire point of this suite.
        assert!(sequences(DEFAULT_SEQUENCE_DEPTH).len() > ALPHABET.len());
    }

    #[test]
    fn double_submit_generator_contains_both_orders_and_a_true_race() {
        assert_eq!(
            DOUBLE_SUBMIT_INTERLEAVINGS,
            [
                DoubleSubmitInterleaving::FirstThenSecond,
                DoubleSubmitInterleaving::SecondThenFirst,
                DoubleSubmitInterleaving::Simultaneous,
            ],
            "removing an ordering would silently shrink the interleaving generator"
        );
    }

    #[test]
    #[should_panic(expected = "expected exactly one Accepted and one busy")]
    fn double_submit_checker_rejects_two_accepted_runs() {
        use ironclaw_host_api::turn::AcceptedMessageRef;

        let ack = |message: &str| ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new(message)
                .expect("sabotage message ref is valid"),
            submitted_run_id: TurnRunId::new(),
        };
        accepted_and_busy_run(
            ack("message:sabotage-a"),
            ack("message:sabotage-b"),
            DoubleSubmitInterleaving::Simultaneous,
        );
    }

    #[test]
    fn accepts_an_ordinary_settling_sequence() {
        observed_with(&[
            TurnStatus::BlockedApproval,
            TurnStatus::Running,
            TurnStatus::Completed,
        ]);
    }

    #[tokio::test]
    async fn orphan_checker_rejects_a_live_run_omitted_from_the_expected_set() {
        let group = RebornIntegrationGroup::live_approvals()
            .await
            .expect("live-approvals group builds");
        let h = group
            .thread("orphan-checker-sabotage")
            .script([RebornScriptedReply::tool_call(
                GATED_CAPABILITY,
                json!({"path": "/workspace/sabotage.txt", "content": "sabotage"}),
            )])
            .build()
            .await
            .expect("sabotage thread builds");
        let (run_id, _) = h
            .submit_turn_until_blocked("park for orphan sabotage")
            .await
            .expect("sabotage run parks");

        let error = h
            .assert_no_orphan_runs_or_reservations(&[])
            .await
            .expect_err("omitting the live run must trip the orphan invariant");
        assert!(
            error.to_string().contains("orphan agent-turn process"),
            "wrong invariant branch fired: {error}"
        );

        h.cancel_run(run_id)
            .await
            .expect("sabotage run cancellation is accepted");
        h.wait_for_status(run_id, TurnStatus::Cancelled)
            .await
            .expect("sabotage run settles after cancellation");
    }

    #[tokio::test]
    async fn orphan_checker_rejects_a_capability_resource_hold() {
        use ironclaw_host_api::{
            ids::InvocationId,
            resource::{ResourceEstimate, ResourceScope},
        };

        let group = RebornIntegrationGroup::live_approvals()
            .await
            .expect("live-approvals group builds");
        let h = group
            .thread("resource-orphan-checker-sabotage")
            .build()
            .await
            .expect("resource sabotage thread builds");
        let governor = h
            .capability_resource_governor_for_test()
            .expect("production-composed governor is exposed");
        let reservation = governor
            .reserve(
                ResourceScope {
                    tenant_id: h.binding.tenant_id.clone(),
                    user_id: h.binding.actor_user_id.clone(),
                    agent_id: h.binding.agent_id.clone(),
                    project_id: h.binding.project_id.clone(),
                    mission_id: None,
                    thread_id: Some(h.binding.thread_id.clone()),
                    invocation_id: InvocationId::new(),
                },
                ResourceEstimate::default().set_concurrency_slots(1),
            )
            .expect("sabotage resource reservation succeeds");

        let error = h
            .assert_no_orphan_runs_or_reservations(&[])
            .await
            .expect_err("a live capability hold must trip the orphan invariant");
        assert!(
            error
                .to_string()
                .contains("orphan capability resource reservation"),
            "wrong invariant branch fired: {error}"
        );

        governor
            .release(reservation.id)
            .expect("sabotage reservation releases");
    }

    #[test]
    fn orphan_checker_rejects_a_process_with_a_missing_parent() {
        use ironclaw_host_api::ids::ProcessId;
        use ironclaw_processes::ProcessKind;

        let process_id = ProcessId::new();
        let missing_parent = ProcessId::new();
        let error = reborn_support::assertions::validate_process_ownership(
            &[(
                process_id,
                ProcessKind::CapabilityInvocation,
                Some(missing_parent),
            )],
            &[],
        )
        .expect_err("a process whose parent is absent must trip the orphan invariant");
        assert!(
            error.to_string().contains("names missing parent"),
            "wrong invariant branch fired: {error}"
        );
    }

    #[test]
    fn orphan_checker_rejects_an_expected_id_with_the_wrong_process_kind() {
        use ironclaw_host_api::ids::ProcessId;
        use ironclaw_processes::ProcessKind;

        let expected_process_id = ProcessId::new();
        let error = reborn_support::assertions::validate_process_ownership(
            &[(expected_process_id, ProcessKind::CapabilityInvocation, None)],
            &[expected_process_id],
        )
        .expect_err("the expected agent-turn id under another kind must fail ownership");
        assert!(
            error.to_string().contains("not AgentTurn"),
            "wrong invariant branch fired: {error}"
        );
    }
}

/// Cross-actor isolation under the same generated action alphabet.
///
/// The lifecycle sequences above drive one actor. This drives two over one
/// shared coordinator and asserts the invariant the workstream names as "no
/// cross-user leakage": whatever happens to A's run, B's is untouched until B
/// acts on it.
///
/// `scenario_multi_actor_gate_isolation` already pins the approve arm. What it
/// cannot show is that the OTHER actions are equally scoped — a cancel or a
/// deny that reached across owners would pass every existing test, because no
/// existing test applies those actions while a second actor is parked.
#[tokio::test]
async fn generated_actions_on_one_actor_never_disturb_another() {
    for action in ALPHABET {
        let group = RebornIntegrationGroup::multiuser_approvals()
            .await
            .expect("multiuser-approvals group builds");

        let a = group
            .thread(format!("gen-xuser-a-{action:?}").to_lowercase())
            .script([
                RebornScriptedReply::tool_call(
                    GATED_CAPABILITY,
                    json!({"path": "/workspace/gen-xuser-a.txt", "content": "a"}),
                ),
                RebornScriptedReply::text("a done"),
            ])
            .build()
            .await
            .expect("actor A thread builds");
        let b = group
            .thread(format!("gen-xuser-b-{action:?}").to_lowercase())
            .with_actor_id("reborn-generated-actor-b")
            .script([
                RebornScriptedReply::tool_call(
                    GATED_CAPABILITY,
                    json!({"path": "/workspace/gen-xuser-b.txt", "content": "b"}),
                ),
                RebornScriptedReply::text("b done"),
            ])
            .build()
            .await
            .expect("actor B thread builds");

        // Each actor's gate is scoped to its own owner, so auto-approve has to
        // be disabled per owner rather than globally — otherwise the run
        // dispatches straight through and never parks.
        let owner_a = a
            .binding
            .subject_user_id
            .as_ref()
            .expect("actor A binding has a subject user id");
        let owner_b = b
            .binding
            .subject_user_id
            .as_ref()
            .expect("actor B binding has a subject user id");
        assert_ne!(owner_a, owner_b, "the two actors must be distinct owners");
        group
            .disable_auto_approve_for_owner(owner_a)
            .await
            .expect("auto-approve disabled for A");
        group
            .disable_auto_approve_for_owner(owner_b)
            .await
            .expect("auto-approve disabled for B");

        let (run_a, gate_a) = a
            .submit_turn_until_blocked("actor a writes")
            .await
            .expect("actor A parks");
        let (run_b, gate_b) = b
            .submit_turn_until_blocked("actor b writes")
            .await
            .expect("actor B parks");
        assert_ne!(
            gate_a, gate_b,
            "the two actors must raise distinct gates, or this proves nothing"
        );

        // Act on A only.
        match action {
            GateAction::Approve => {
                let _ = a.approve_gate(run_a, &gate_a).await;
            }
            GateAction::Deny => {
                let _ = a.deny_gate(run_a, &gate_a).await;
            }
            GateAction::Cancel => {
                let _ = a.cancel_run(run_a).await;
            }
        }

        // B is untouched: still parked on its own gate, and its effect has not
        // been performed. Status alone is not enough — a leak that resolved B's
        // gate and ran its write would be invisible if only A were inspected.
        let b_state = b.run_state(run_b).await.expect("actor B state readable");
        assert_eq!(
            b_state.status,
            TurnStatus::BlockedApproval,
            "{action:?} on actor A moved actor B to {:?}",
            b_state.status
        );
        // By FILE, not by the result counter: the recorder is group-scoped, so
        // B's count also sees A's effect. Distinct paths attribute exactly.
        b.assert_workspace_file_absent("gen-xuser-b.txt")
            .await
            .unwrap_or_else(|err| {
                panic!("{action:?} on actor A performed actor B's gated effect: {err}")
            });

        // ...and B can still resolve its own gate afterwards, so the isolation
        // is not simply "B was wedged".
        b.approve_gate(run_b, &gate_b)
            .await
            .expect("actor B resolves its own gate after A acted");
        b.wait_for_terminal(run_b)
            .await
            .expect("actor B settles independently");
        b.assert_workspace_file_contains("gen-xuser-b.txt", "b")
            .await
            .expect("actor B's own approval performs its own effect");
    }
}

/// Concurrency dimension: two runs in flight at once over one coordinator.
///
/// The sequences above act on a single parked run. Nothing there can show what
/// happens when a second run is live at the same moment — which is the
/// ordinary case in production and the one where a shared coordinator, a
/// shared gate store, or a shared lease would cross wires.
#[tokio::test]
async fn concurrent_runs_resolve_independently() {
    for action in [GateAction::Approve, GateAction::Deny, GateAction::Cancel] {
        let group = RebornIntegrationGroup::live_approvals()
            .await
            .expect("live-approvals group builds");
        let first = group
            .thread(format!("gen-conc-1-{action:?}").to_lowercase())
            .script([
                RebornScriptedReply::tool_call(
                    GATED_CAPABILITY,
                    json!({"path": "/workspace/gen-conc-1.txt", "content": "one"}),
                ),
                RebornScriptedReply::text("one done"),
            ])
            .build()
            .await
            .expect("first thread builds");
        let second = group
            .thread(format!("gen-conc-2-{action:?}").to_lowercase())
            .script([
                RebornScriptedReply::tool_call(
                    GATED_CAPABILITY,
                    json!({"path": "/workspace/gen-conc-2.txt", "content": "two"}),
                ),
                RebornScriptedReply::text("two done"),
            ])
            .build()
            .await
            .expect("second thread builds");

        // Both parked before either is resolved: this is the overlap the
        // single-run sequences cannot produce.
        let (run_one, gate_one) = first
            .submit_turn_until_blocked("first writes")
            .await
            .expect("first parks");
        let (run_two, gate_two) = second
            .submit_turn_until_blocked("second writes")
            .await
            .expect("second parks");
        assert_ne!(
            gate_one, gate_two,
            "concurrent runs must raise distinct gates"
        );

        match action {
            GateAction::Approve => {
                let _ = first.approve_gate(run_one, &gate_one).await;
            }
            GateAction::Deny => {
                let _ = first.deny_gate(run_one, &gate_one).await;
            }
            GateAction::Cancel => {
                let _ = first.cancel_run(run_one).await;
            }
        }

        // Resolving one must not move the other, and must not perform its
        // effect. A shared-gate bug would show up exactly here.
        let two_state = second
            .run_state(run_two)
            .await
            .expect("second run readable");
        assert_eq!(
            two_state.status,
            TurnStatus::BlockedApproval,
            "{action:?} on the first run moved the concurrent run to {:?}",
            two_state.status
        );
        // Attributed by FILE, not by the result counter: the recorder is
        // group-scoped, so `second`'s count also sees the first run's effect
        // and cannot tell whose it was. Each run writes a distinct path, which
        // can. The first version of this used the counter and failed with
        // left: 2 — the other run's write, counted as this one's.
        second
            .assert_workspace_file_absent("gen-conc-2.txt")
            .await
            .unwrap_or_else(|err| {
                panic!("{action:?} on the first run performed the concurrent run's effect: {err}")
            });

        // The second run still resolves on its own terms afterwards.
        second
            .approve_gate(run_two, &gate_two)
            .await
            .expect("concurrent run resolves independently");
        second
            .wait_for_terminal(run_two)
            .await
            .expect("concurrent run settles");
        second
            .assert_workspace_file_contains("gen-conc-2.txt", "two")
            .await
            .expect("the concurrent run's own approval performs its own effect");
    }
}
