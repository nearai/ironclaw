//! Delivery outcome records for trigger-fired runs.
//!
//! When a trigger fires and submits a run, the delivery driver attempts to
//! resolve the creator's personal communication preference and send the run
//! result to their configured personal delivery target. This module holds the
//! outcome record and its in-memory store.
//!
//! Design constraints:
//! - Resolution-stage failures (e.g. no default configured) must NOT produce
//!   delivery-attempt rows — those rows are for attempts that reach the
//!   transport layer. Instead we write a lightweight outcome record here.
//! - Authoritative terminal state: owners propagate record failures into their
//!   managed task lifecycle instead of reporting an unpersisted completion.
//! - Personal scope only: non-personal triggers fail closed with `Denied`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::UserId;
use ironclaw_host_api::turn::{TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

/// Terminal outcome of a triggered-run delivery attempt.
///
/// One record is written per run, after delivery reaches a terminal state
/// (success or any failure). The outcome is coarse and sanitized: no target
/// addresses, no payloads, no reasons that would expose PII.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggeredRunDeliveryOutcomeKind {
    /// The final reply (or gate prompt) was delivered successfully.
    Delivered,
    /// No default communication target is configured for this creator.
    NoDefaultConfigured,
    /// The resolved target is unavailable or rejected the delivery.
    TargetUnavailable,
    /// The trigger creator's scope could not be confirmed (personal-scope
    /// check failed). Delivery is always denied for non-personal triggers.
    Denied,
    /// Delivery failed due to a transport or egress error.
    Failed,
    /// Delivery was skipped (e.g. run was already delivered, empty result).
    Skipped,
}

/// Sanitized delivery outcome keyed by run id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggeredRunDeliveryRecord {
    pub run_id: TurnRunId,
    pub outcome: TriggeredRunDeliveryOutcomeKind,
    pub recorded_at: DateTime<Utc>,
}

/// Store for [`TriggeredRunDeliveryRecord`]s.
///
/// Intentionally minimal: one write per run and one read per run. The channel
/// delivery owner treats write failures as authoritative lifecycle failures.
#[async_trait::async_trait]
pub trait TriggeredRunDeliveryStore: Send + Sync {
    async fn record_triggered_run_delivery(
        &self,
        record: TriggeredRunDeliveryRecord,
    ) -> Result<(), String>;

    async fn load_triggered_run_delivery(
        &self,
        run_id: TurnRunId,
    ) -> Result<Option<TriggeredRunDeliveryRecord>, String>;
}

/// One trigger-submitted run to watch and deliver, in generic vocabulary.
/// The generic post-submit hook translates its trigger-fire type into this.
#[derive(Debug, Clone)]
pub struct TriggeredRunDeliveryRequest {
    pub run_id: TurnRunId,
    pub scope: TurnScope,
    /// The trigger creator; notifications go to THEIR notification channels.
    pub creator_user_id: UserId,
    /// Fail closed for non-personal triggers: a project-scoped trigger never
    /// notifies a personal channel.
    pub project_scoped: bool,
    /// The trigger prompt; its first line becomes the short footer label.
    pub prompt: String,
}

/// The proactive delivery driver for one channel extension, as its caller
/// consumes it.
///
/// **Declared here rather than beside the implementation** (§12.11 D-A, the
/// same move WS2.5 made for the auth ports): every type in the signature is
/// either this crate's own triggered-delivery vocabulary or
/// `ironclaw_host_api` turn vocabulary, and the caller — the generic
/// post-submit hook in `ironclaw_extension_host` — sits below the crate that
/// implements it. Declaring it in the vocabulary's own home costs zero type
/// weakening; narrowing it into a contracts crate would have cost the request
/// its typed turn vocabulary.
#[async_trait::async_trait]
pub trait TriggeredRunDelivery: Send + Sync {
    /// Watch the submitted run and deliver its outputs to the creator's
    /// resolved target, recording the terminal outcome in the store.
    async fn on_trigger_submitted(&self, request: TriggeredRunDeliveryRequest);
}

#[async_trait::async_trait]
impl<T> TriggeredRunDelivery for Arc<T>
where
    T: TriggeredRunDelivery + ?Sized,
{
    async fn on_trigger_submitted(&self, request: TriggeredRunDeliveryRequest) {
        self.as_ref().on_trigger_submitted(request).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_store_round_trips_outcome_record() {
        let store = crate::test_support::in_memory_backed_outbound_state_store();
        let run_id = TurnRunId::new();
        let record = TriggeredRunDeliveryRecord {
            run_id,
            outcome: TriggeredRunDeliveryOutcomeKind::Delivered,
            recorded_at: Utc::now(),
        };

        store
            .record_triggered_run_delivery(record.clone())
            .await
            .expect("record write succeeds");

        let loaded = store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("record load succeeds");

        assert_eq!(loaded, Some(record));
    }

    #[tokio::test]
    async fn in_memory_store_returns_none_for_unknown_run() {
        let store = crate::test_support::in_memory_backed_outbound_state_store();

        let loaded = store
            .load_triggered_run_delivery(TurnRunId::new())
            .await
            .expect("missing load succeeds");

        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn in_memory_store_overwrites_on_second_write() {
        let store = crate::test_support::in_memory_backed_outbound_state_store();
        let run_id = TurnRunId::new();

        store
            .record_triggered_run_delivery(TriggeredRunDeliveryRecord {
                run_id,
                outcome: TriggeredRunDeliveryOutcomeKind::Failed,
                recorded_at: Utc::now(),
            })
            .await
            .expect("first write");

        store
            .record_triggered_run_delivery(TriggeredRunDeliveryRecord {
                run_id,
                outcome: TriggeredRunDeliveryOutcomeKind::Delivered,
                recorded_at: Utc::now(),
            })
            .await
            .expect("second write");

        let loaded = store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load");
        assert_eq!(
            loaded.map(|r| r.outcome),
            Some(TriggeredRunDeliveryOutcomeKind::Delivered)
        );
    }

    #[test]
    fn all_outcome_kinds_serialize_as_snake_case() {
        let cases = [
            (TriggeredRunDeliveryOutcomeKind::Delivered, "delivered"),
            (
                TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured,
                "no_default_configured",
            ),
            (
                TriggeredRunDeliveryOutcomeKind::TargetUnavailable,
                "target_unavailable",
            ),
            (TriggeredRunDeliveryOutcomeKind::Denied, "denied"),
            (TriggeredRunDeliveryOutcomeKind::Failed, "failed"),
            (TriggeredRunDeliveryOutcomeKind::Skipped, "skipped"),
        ];
        for (outcome, expected) in cases {
            let serialized = serde_json::to_string(&outcome).expect("serialize outcome");
            assert_eq!(
                serialized,
                format!("\"{expected}\""),
                "outcome {outcome:?} should serialize as {expected:?}"
            );
        }
    }
}
