//! Failure-category identifiers — the closed key vocabulary of the
//! category→summary tables in [`super::summary`].
//!
//! These are pure data: stable `&'static str` identifiers that producers stamp
//! onto a failed run and that projections key user-facing copy from. They live
//! here, not in the crate that produces them, because both sides of that
//! relationship are above this crate — the producer (`ironclaw_turn_runner`)
//! and the consumer (the product projection) — and a category constant is
//! kernel vocabulary, not runner implementation (PROPOSAL §6.1.1: "failure-summary
//! data … joins the existing `failure` module so product stops depending on
//! runner").
//!
//! **Classification is not data and did not move.** The functions that decide
//! *which* category a failure gets (`host_stage_unavailable_category`,
//! `failure_lane`, the retry disposition) stay with the producer: they are typed
//! on loop/agent-loop vocabulary that this crate must never depend on.
//!
//! Adding a constant here obliges a matching arm in [`super::summary`]; the
//! product projection's `failure_summary_covers_reborn_failure_category_constants`
//! scans this file and fails when the two drift.

/// Failure category identifier for model provider credit exhaustion.
/// Exposed for cross-crate consumers that project this category to a user-facing message.
pub const MODEL_CREDITS_EXHAUSTED_CATEGORY: &str = "model_credits_exhausted";

/// Failure category identifier for model provider credential or endpoint configuration failures.
/// Exposed for cross-crate consumers that project this category to a user-facing message.
pub const MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY: &str = "model_credentials_unavailable";

/// Failure category for an exhausted host-configured model spend budget.
pub const MODEL_SPEND_BUDGET_EXHAUSTED_CATEGORY: &str = "model_spend_budget_exhausted";

/// Failure category for durable host-side resource accounting outages.
/// This must not be presented as a provider balance or configured-budget outcome.
pub const BUDGET_ACCOUNTING_FAILED_CATEGORY: &str = "budget_accounting_failed";

/// Failure category for assistant output that could not be durably committed.
pub const TRANSCRIPT_WRITE_FAILED_CATEGORY: &str = "transcript_write_failed";

/// A deterministic checkpoint validation rejection.
///
/// The rejected state is never used to run a model or capability. The
/// independent turn lifecycle journal records the terminal explanation.
pub const CHECKPOINT_REJECTED_CATEGORY: &str = "checkpoint_rejected";

/// Model-stage failures that are PERMANENT for an identical retry.
///
/// These four kinds previously returned no category and fell through to
/// `host_stage_unavailable_model`, which `is_auto_retriable_category` treats as
/// a transient outage that re-drives cleanly. None of them can succeed on an
/// identical retry — policy does not change between attempts and a malformed
/// request stays malformed — so the run burned retries on a call that could not
/// work and reported a generic host outage instead of the real cause.
///
/// Deliberately NOT in `is_auto_retriable_category`.
pub const MODEL_STAGE_REQUEST_INVALID_CATEGORY: &str = "model_stage_request_invalid";
/// Model-stage call refused by policy. Permanent; see
/// [`MODEL_STAGE_REQUEST_INVALID_CATEGORY`].
pub const MODEL_STAGE_POLICY_DENIED_CATEGORY: &str = "model_stage_policy_denied";
/// Model-stage call outside the granted scope — configuration-shaped, not
/// transient. See [`MODEL_STAGE_REQUEST_INVALID_CATEGORY`].
pub const MODEL_STAGE_SCOPE_MISMATCH_CATEGORY: &str = "model_stage_scope_mismatch";

pub const HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY: &str = "host_stage_unavailable_prompt";
pub const HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY: &str = "host_stage_unavailable_model";
pub const HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY: &str = "host_stage_unavailable_capability";
pub const HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY: &str = "host_stage_unavailable_transcript";
pub const HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY: &str = "host_stage_unavailable_checkpoint";
pub const HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY: &str = "host_stage_unavailable_input";
pub const HOST_STAGE_UNAVAILABLE_UNKNOWN_CATEGORY: &str = "host_stage_unavailable_unknown";
