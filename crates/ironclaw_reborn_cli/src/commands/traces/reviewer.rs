//! Auto-split per-audience dispatch for the `traces` CLI surface.
//!
//! Audience: reviewer. See `super::run_traces` for the routing.

use super::*;

pub(super) async fn dispatch(cmd: TracesSubcommand) -> anyhow::Result<()> {
    match cmd {
        TracesSubcommand::QuarantineList {
            endpoint,
            lease_filter,
            bearer_token_env,
            json,
        } => trace_commons_quarantine_list(&endpoint, &bearer_token_env, lease_filter, json).await,
        TracesSubcommand::ActiveLearningReviewQueue {
            endpoint,
            limit,
            privacy_risk,
            lease_filter,
            bearer_token_env,
            json,
        } => {
            trace_commons_active_learning_review_queue(
                &endpoint,
                &bearer_token_env,
                limit,
                privacy_risk,
                lease_filter,
                json,
            )
            .await
        }
        TracesSubcommand::ReviewDecision {
            endpoint,
            submission_id,
            decision,
            reason,
            credit_points_pending,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_decision(TraceCommonsReviewDecisionOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                submission_id,
                decision,
                reason,
                credit_points_pending,
                json,
            })
            .await
        }
        TracesSubcommand::ReviewLeaseClaim {
            endpoint,
            submission_id,
            lease_ttl_seconds,
            review_due_at,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_lease_claim(TraceCommonsReviewLeaseClaimOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                submission_id,
                lease_ttl_seconds,
                review_due_at,
                json,
            })
            .await
        }
        TracesSubcommand::ReviewLeaseClaimNext {
            endpoint,
            lease_ttl_seconds,
            review_due_at,
            privacy_risk,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_lease_claim_next(TraceCommonsReviewLeaseClaimNextOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                lease_ttl_seconds,
                review_due_at,
                privacy_risk,
                json,
            })
            .await
        }
        TracesSubcommand::ReviewLeaseClaimBatch {
            endpoint,
            limit,
            lease_ttl_seconds,
            review_due_at,
            privacy_risk,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_lease_claim_batch(TraceCommonsReviewLeaseClaimBatchOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                limit,
                lease_ttl_seconds,
                review_due_at,
                privacy_risk,
                json,
            })
            .await
        }
        TracesSubcommand::ReviewLeaseRelease {
            endpoint,
            submission_id,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_lease_release(&endpoint, &bearer_token_env, submission_id, json)
                .await
        }
        TracesSubcommand::AppendCreditEvent {
            endpoint,
            submission_id,
            event_type,
            credit_points_delta,
            reason,
            external_ref,
            bearer_token_env,
            json,
        } => {
            trace_commons_append_credit_event(TraceCommonsAppendCreditEventOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                submission_id,
                event_type,
                credit_points_delta,
                reason,
                external_ref,
                json,
            })
            .await
        }
        _ => unreachable!("router ensures only audience variants reach this dispatch"),
    }
}
