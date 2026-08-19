//! Derived activation-streak caps.
//!
//! Deliberately no stored counter and no new component: each cap is a
//! predicate over a bounded, newest-first window of the thread's own run
//! records, fetched with the complementary provenance excluded.

use crate::{ActivationProvenance, TurnRunRecord};

/// Consecutive `System`-provenance activations allowed on one thread before
/// the reactive wake is refused.
///
/// Independently named on purpose. It coincides numerically with the
/// 16-descendant spawn-tree cap and the SUBAGENT loop family's 16-iteration
/// limit, and those three budgets must never be merged by a refactor — they
/// bound unrelated things and would drift apart the moment one is tuned.
pub const SYSTEM_WAKE_STREAK_CAP: u32 = 16;

/// How many raw run records to fetch per cap-sized window so that, after
/// `ParentAgent` runs are dropped, `SYSTEM_WAKE_STREAK_CAP` records remain.
///
/// The window query is provenance-blind, so the design's "excluded from the
/// fetch" rule is approximated by over-fetching and truncating. A thread whose
/// recent history is more than `(OVERFETCH - 1) / OVERFETCH` `ParentAgent`
/// runs still yields a short window, which admits — a fail-open residual, not
/// a guarantee. Once the extend cap bounds consecutive `ParentAgent`
/// activations at 8, raising this to 9 makes the exclusion exact.
pub const SYSTEM_WAKE_WINDOW_OVERFETCH: u32 = 4;

/// Whether a pending `System` activation may be admitted, given the thread's
/// newest-first window of `Human`/`System` runs (`ParentAgent` runs are
/// excluded by the caller's fetch, so they neither count nor reset).
///
/// Refusing costs nothing durable: a settled await-edge stays settled and
/// drains via the run-start sweep or the boot pass. This gates the reactive
/// wake only, never delivery itself.
///
/// An untagged run (`None`) is an ordinary human-initiated submission and
/// resets the streak, same as an explicit `Human`.
pub fn system_wake_admitted(recent: &[TurnRunRecord]) -> bool {
    if recent.len() < SYSTEM_WAKE_STREAK_CAP as usize {
        return true;
    }
    recent
        .iter()
        .take(SYSTEM_WAKE_STREAK_CAP as usize)
        .any(|record| record.subagent_activation_provenance != Some(ActivationProvenance::System))
}

#[cfg(test)]
mod tests {
    use super::{SYSTEM_WAKE_STREAK_CAP, system_wake_admitted};
    use crate::{ActivationProvenance, TurnRunRecord};

    fn record_with(provenance: Option<ActivationProvenance>) -> TurnRunRecord {
        use crate::{AcceptedMessageRef, EventCursor, TurnRunId, TurnScope, TurnStatus};
        use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId};

        let profile: crate::TurnRunProfile = serde_json::from_value(serde_json::json!({
            "id": "default",
            "version": 1,
            "allow_steering": false,
            "auto_queue_followups": false,
        }))
        .expect("profile deserialization");
        TurnRunRecord {
            run_id: TurnRunId::new(),
            turn_id: crate::TurnId::new(),
            scope: TurnScope::new(
                TenantId::new("tenant-streak-test").expect("tenant"),
                Some(AgentId::new("agent-streak-test").expect("agent")),
                Some(ProjectId::new("project-streak-test").expect("project")),
                ThreadId::new("thread-streak-test").expect("thread"),
            ),
            accepted_message_ref: AcceptedMessageRef::new("accepted-streak-test")
                .expect("accepted"),
            status: TurnStatus::Completed,
            profile,
            output_contract: Default::default(),
            resolved_model_route: None,
            model_usage: None,
            execution_outcome: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(1),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: chrono::Utc::now(),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            subagent_activation_provenance: provenance,
            product_context: None,
            resume_disposition: None,
        }
    }

    /// `provenances[0]` is the newest run.
    fn window(provenances: &[ActivationProvenance]) -> Vec<TurnRunRecord> {
        provenances
            .iter()
            .map(|provenance| record_with(Some(*provenance)))
            .collect()
    }

    #[test]
    fn under_cap_consecutive_system_wakes_are_admitted() {
        let recent = window(&[ActivationProvenance::System; 15]);
        assert!(
            system_wake_admitted(&recent),
            "15 consecutive System wakes is under the cap of {SYSTEM_WAKE_STREAK_CAP}"
        );
    }

    #[test]
    fn a_full_window_of_system_wakes_refuses_the_next_one() {
        let recent = window(&[ActivationProvenance::System; SYSTEM_WAKE_STREAK_CAP as usize]);
        assert!(
            !system_wake_admitted(&recent),
            "a full window of System runs means the pending wake would be the 17th consecutive one"
        );
    }

    #[test]
    fn a_human_activation_anywhere_in_the_window_resets_the_streak() {
        let mut provenances = [ActivationProvenance::System; SYSTEM_WAKE_STREAK_CAP as usize];
        provenances[SYSTEM_WAKE_STREAK_CAP as usize - 1] = ActivationProvenance::Human;
        assert!(
            system_wake_admitted(&window(&provenances)),
            "a Human run anywhere in the window resets the streak"
        );
    }

    #[test]
    fn a_short_history_is_admitted() {
        let recent = window(&[ActivationProvenance::System; 3]);
        assert!(
            system_wake_admitted(&recent),
            "a young thread with fewer than {SYSTEM_WAKE_STREAK_CAP} records must be admitted"
        );
    }

    #[test]
    fn untagged_legacy_runs_count_as_human_and_reset_the_streak() {
        let mut recent =
            window(&[ActivationProvenance::System; SYSTEM_WAKE_STREAK_CAP as usize - 1]);
        recent.push(record_with(None));
        assert!(
            system_wake_admitted(&recent),
            "an untagged run is an ordinary human-initiated run and must reset the streak"
        );
    }

    /// The three 16-valued budgets in this subsystem bound unrelated things.
    /// This pins the wake cap's own identity so a future refactor cannot
    /// quietly alias it to the descendant cap or the iteration limit.
    #[test]
    fn the_wake_streak_cap_is_its_own_named_budget() {
        assert_eq!(SYSTEM_WAKE_STREAK_CAP, 16);
    }
}
