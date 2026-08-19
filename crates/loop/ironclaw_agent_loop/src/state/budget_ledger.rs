//! Per-run resource budget accounting chokepoint.
//!
//! `BudgetLedger` owns the three per-run budget counters that used to be
//! bare public fields on [`super::LoopExecutionState`]: `run_started_at`,
//! `model_calls_made`, and `capability_invocations_made`. Every charge
//! against the run's model-call or capability-invocation budget goes
//! through this type's typed API — production code can no longer increment
//! a counter without checking the remaining allowance, and a reset can no
//! longer forget one of the three fields.
//!
//! Wire compatibility: `LoopExecutionState` embeds this type with
//! `#[serde(flatten)]`, so the JSON checkpoint shape is byte-identical to
//! the previous three top-level fields (same names, same `#[serde(default)]`
//! / `skip_serializing_if` behavior). See the frozen-shape test in
//! `state.rs`.

use chrono::{DateTime, Utc};
use ironclaw_loop_contracts::ResourceBudgetPolicy;

/// Outcome of charging a single model-call dispatch against the run's
/// model-call budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetCharge {
    /// The call was admitted; the counter now reflects it.
    Charged,
    /// The budget has no remaining allowance; nothing was charged.
    Exhausted,
}

/// Outcome of charging a batch of capability invocations against the run's
/// capability-invocation budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationCharge {
    /// The whole batch was admitted; the counter now reflects all of it.
    Charged,
    /// Only `admitted` of the requested invocations fit the remaining
    /// allowance (`admitted` is always `> 0` and `< n`); the counter now
    /// reflects exactly `admitted` more invocations. The caller is
    /// responsible for not dispatching the remainder.
    Partial { admitted: usize },
    /// The budget has no remaining allowance; nothing was charged.
    Exhausted,
}

/// Per-run budget accounting: wall-clock start plus the model-call and
/// capability-invocation counters. Fields are private — all reads go
/// through the accessors below and all writes go through the typed charge
/// API (or `fresh_for_run` / the test-only setters), so a caller can never
/// increment a counter without checking the remaining allowance and can
/// never reset only part of the ledger.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BudgetLedger {
    /// Wall-clock start of budget accounting for this run. `None` until the
    /// budget stage's first pass arms it (initial state stays deterministic;
    /// older checkpoints without the field re-arm from resume time). Read
    /// against `ResourceBudgetPolicy::max_wall_clock_seconds`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_started_at: Option<DateTime<Utc>>,

    /// Provider model calls dispatched this run (every attempt, including
    /// recovery retries). Enforced against
    /// `ResourceBudgetPolicy::max_model_calls`.
    #[serde(default)]
    model_calls_made: u32,

    /// Capability invocations executed this run, summed per completed
    /// batch. Enforced against
    /// `ResourceBudgetPolicy::max_capability_invocations`.
    #[serde(default)]
    capability_invocations_made: u32,
}

impl BudgetLedger {
    /// A zeroed ledger for the start of a fresh run — used both by
    /// `LoopExecutionState::initial_for_run` and by `rebase_for_run`'s
    /// different-run branch, so a fresh run's accounting can never
    /// accidentally carry over a source run's counters.
    pub(crate) fn fresh_for_run() -> Self {
        Self::default()
    }

    pub(crate) fn model_calls_made(&self) -> u32 {
        self.model_calls_made
    }

    pub(crate) fn capability_invocations_made(&self) -> u32 {
        self.capability_invocations_made
    }

    /// Arms the wall-clock start on the first call (returns `now` and
    /// stores it) and returns the previously-armed start on every later
    /// call. Mirrors the pre-refactor `Option::get_or_insert` behavior in
    /// the budget stage exactly.
    pub(crate) fn arm_wall_clock(&mut self, now: DateTime<Utc>) -> DateTime<Utc> {
        *self.run_started_at.get_or_insert(now)
    }

    /// Charges one model-call dispatch against `policy.max_model_calls`.
    /// Returns `Exhausted` (charging nothing) when the run has already hit
    /// its ceiling.
    pub(crate) fn try_charge_model_call(&mut self, policy: &ResourceBudgetPolicy) -> BudgetCharge {
        if self.model_calls_made >= policy.max_model_calls {
            return BudgetCharge::Exhausted;
        }
        self.model_calls_made = self.model_calls_made.saturating_add(1);
        BudgetCharge::Charged
    }

    /// Charges up to `n` capability invocations against
    /// `policy.max_capability_invocations`. When the full batch does not
    /// fit the remaining allowance, admits and charges only the leading
    /// `admitted` of them and reports `Partial`; when nothing remains,
    /// charges nothing and reports `Exhausted`.
    pub(crate) fn try_charge_invocations(
        &mut self,
        n: usize,
        policy: &ResourceBudgetPolicy,
    ) -> InvocationCharge {
        if n == 0 {
            return InvocationCharge::Charged;
        }
        let remaining = policy
            .max_capability_invocations
            .saturating_sub(self.capability_invocations_made) as usize;
        if remaining == 0 {
            return InvocationCharge::Exhausted;
        }
        if n <= remaining {
            self.capability_invocations_made =
                self.capability_invocations_made.saturating_add(n as u32);
            InvocationCharge::Charged
        } else {
            self.capability_invocations_made = self
                .capability_invocations_made
                .saturating_add(remaining as u32);
            InvocationCharge::Partial {
                admitted: remaining,
            }
        }
    }

    /// Settles a batch reservation after the host reports how many admitted
    /// calls it actually launched. Returns `false` when the supplied counts
    /// contradict the reservation or cannot be represented by the ledger.
    pub(crate) fn settle_invocation_reservation(
        &mut self,
        reserved: usize,
        launched: usize,
    ) -> bool {
        let Some(unlaunched) = reserved.checked_sub(launched) else {
            return false;
        };
        let Ok(unlaunched) = u32::try_from(unlaunched) else {
            return false;
        };
        let Some(settled) = self.capability_invocations_made.checked_sub(unlaunched) else {
            return false;
        };
        self.capability_invocations_made = settled;
        true
    }
}

/// Test seams live in a test-gated module (struct-debt ratchet: production
/// struct code carries no test-support members). The impl still attaches to
/// [`BudgetLedger`] crate-wide under `cfg(test)`.
#[cfg(test)]
mod test_seams {
    use super::*;

    impl BudgetLedger {
        pub(crate) fn set_model_calls_made_for_test(&mut self, value: u32) {
            self.model_calls_made = value;
        }

        pub(crate) fn set_capability_invocations_made_for_test(&mut self, value: u32) {
            self.capability_invocations_made = value;
        }

        pub(crate) fn set_run_started_at_for_test(&mut self, value: Option<DateTime<Utc>>) {
            self.run_started_at = value;
        }

        /// Test-only accessor: production code reads the wall-clock start
        /// only through `arm_wall_clock`, which arms and returns it in one
        /// call.
        pub(crate) fn run_started_at(&self) -> Option<DateTime<Utc>> {
            self.run_started_at
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_model_calls: u32, max_capability_invocations: u32) -> ResourceBudgetPolicy {
        ResourceBudgetPolicy {
            tier: ironclaw_loop_contracts::ResourceBudgetTier::new("budget-ledger-test-tier")
                .expect("valid"),
            max_model_calls,
            max_capability_invocations,
            max_wall_clock_seconds: None,
        }
    }

    #[test]
    fn try_charge_model_call_charges_until_the_ceiling_then_exhausts() {
        let mut ledger = BudgetLedger::default();
        let policy = policy(2, 0);

        assert_eq!(ledger.try_charge_model_call(&policy), BudgetCharge::Charged);
        assert_eq!(ledger.model_calls_made(), 1);
        assert_eq!(ledger.try_charge_model_call(&policy), BudgetCharge::Charged);
        assert_eq!(ledger.model_calls_made(), 2);
        assert_eq!(
            ledger.try_charge_model_call(&policy),
            BudgetCharge::Exhausted
        );
        // An exhausted charge does not mutate the counter.
        assert_eq!(ledger.model_calls_made(), 2);
    }

    #[test]
    fn try_charge_invocations_charges_the_whole_batch_when_it_fits() {
        let mut ledger = BudgetLedger::default();
        let policy = policy(0, 10);

        assert_eq!(
            ledger.try_charge_invocations(4, &policy),
            InvocationCharge::Charged
        );
        assert_eq!(ledger.capability_invocations_made(), 4);
    }

    #[test]
    fn try_charge_invocations_admits_a_partial_batch_at_the_boundary() {
        let mut ledger = BudgetLedger::default();
        let policy = policy(0, 10);
        ledger.set_capability_invocations_made_for_test(8);

        assert_eq!(
            ledger.try_charge_invocations(4, &policy),
            InvocationCharge::Partial { admitted: 2 }
        );
        // Charged exactly the admitted count, landing exactly at the cap.
        assert_eq!(ledger.capability_invocations_made(), 10);
    }

    #[test]
    fn try_charge_invocations_exhausted_charges_nothing() {
        let mut ledger = BudgetLedger::default();
        let policy = policy(0, 10);
        ledger.set_capability_invocations_made_for_test(10);

        assert_eq!(
            ledger.try_charge_invocations(3, &policy),
            InvocationCharge::Exhausted
        );
        assert_eq!(ledger.capability_invocations_made(), 10);
    }

    #[test]
    fn try_charge_invocations_of_zero_is_a_charged_no_op() {
        let mut ledger = BudgetLedger::default();
        let policy = policy(0, 0);

        assert_eq!(
            ledger.try_charge_invocations(0, &policy),
            InvocationCharge::Charged
        );
        assert_eq!(ledger.capability_invocations_made(), 0);
    }

    #[test]
    fn settle_invocation_reservation_refunds_only_unlaunched_calls() {
        let mut ledger = BudgetLedger::default();
        let policy = policy(0, 10);
        assert_eq!(
            ledger.try_charge_invocations(4, &policy),
            InvocationCharge::Charged
        );

        assert!(ledger.settle_invocation_reservation(4, 1));
        assert_eq!(ledger.capability_invocations_made(), 1);
    }

    #[test]
    fn settle_invocation_reservation_rejects_impossible_counts() {
        let mut ledger = BudgetLedger::default();
        let policy = policy(0, 10);
        assert_eq!(
            ledger.try_charge_invocations(2, &policy),
            InvocationCharge::Charged
        );

        assert!(!ledger.settle_invocation_reservation(2, 3));
        assert_eq!(ledger.capability_invocations_made(), 2);
    }

    #[test]
    fn arm_wall_clock_arms_once_then_returns_the_same_start() {
        let mut ledger = BudgetLedger::default();
        let first = Utc::now();
        assert_eq!(ledger.arm_wall_clock(first), first);
        assert_eq!(ledger.run_started_at(), Some(first));

        let later = first + chrono::Duration::seconds(30);
        assert_eq!(ledger.arm_wall_clock(later), first);
        assert_eq!(ledger.run_started_at(), Some(first));
    }

    #[test]
    fn fresh_for_run_is_a_zeroed_ledger() {
        let mut ledger = BudgetLedger::default();
        ledger.set_model_calls_made_for_test(5);
        ledger.set_capability_invocations_made_for_test(7);
        ledger.set_run_started_at_for_test(Some(Utc::now()));

        let fresh = BudgetLedger::fresh_for_run();
        assert_eq!(fresh.model_calls_made(), 0);
        assert_eq!(fresh.capability_invocations_made(), 0);
        assert_eq!(fresh.run_started_at(), None);
        // The prior ledger is untouched by constructing a fresh one.
        assert_eq!(ledger.model_calls_made(), 5);
    }
}
