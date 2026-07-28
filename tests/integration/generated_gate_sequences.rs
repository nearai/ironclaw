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

use ironclaw_turns::TurnStatus;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// The capability the gate guards. Named once so the effect-count assertion
/// and the scripted call cannot drift apart.
const GATED_CAPABILITY: &str = "builtin.write_file";

// Where each dimension workstream 9 names is actually exercised.
//
// Listing them as a comment rather than inventing an enum per axis: a typed
// `Ingress` with one variant that nothing matches on is decoration, and it
// would read as coverage on a scan.
//
// | axis | exercised | where |
// |---|---|---|
// | lifecycle state | yes | `GateAction` below, enumerated |
// | policy state | yes | auto-approve on/off, per owner, in the cross-actor case |
// | auth state | partly | `auth/auth_gate.rs` + the expired-credential resume; not enumerated with the lifecycle axis |
// | provider outcome | partly | `with_github_network_status` drives 401/5xx in the auth slice; not crossed with gate actions |
// | operation class | no | read vs idempotent vs non-idempotent write is an E2E fault-matrix axis (`provider_fault_cases.py`) |
// | ingress | no | one ingress at this tier; WebUI/Slack/Telegram are covered whole-path in E2E |
// | delivery target | no | same — a delivery target needs a channel, which this tier does not run |
//
// The three "no" rows are not oversights: crossing them with the lifecycle
// axis needs a tier that runs channels and providers, which is the E2E
// journey suite, not this one. Recorded so the epic is not read as claiming
// (table above documents the axes; the enum's own doc follows)

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

/// How long a sequence to enumerate. Pull requests get depth 2 (20 sequences,
/// ~20s); the nightly deep lane raises it.
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
    fn accepts_an_ordinary_settling_sequence() {
        observed_with(&[
            TurnStatus::BlockedApproval,
            TurnStatus::Running,
            TurnStatus::Completed,
        ]);
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
