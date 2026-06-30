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
//! - Best-effort: a failure to record must never block or abort delivery.
//! - Personal scope only: non-personal triggers fail closed with `Denied`.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use ironclaw_turns::TurnRunId;
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
/// Intentionally minimal: one write per run, one read per run for test
/// assertions. Production callers treat this as best-effort and swallow
/// store errors.
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

/// In-memory [`TriggeredRunDeliveryStore`].
#[derive(Default)]
pub struct InMemoryTriggeredRunDeliveryStore {
    records: Mutex<HashMap<TurnRunId, TriggeredRunDeliveryRecord>>,
}

#[async_trait::async_trait]
impl TriggeredRunDeliveryStore for InMemoryTriggeredRunDeliveryStore {
    async fn record_triggered_run_delivery(
        &self,
        record: TriggeredRunDeliveryRecord,
    ) -> Result<(), String> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(record.run_id, record);
        Ok(())
    }

    async fn load_triggered_run_delivery(
        &self,
        run_id: TurnRunId,
    ) -> Result<Option<TriggeredRunDeliveryRecord>, String> {
        Ok(self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&run_id)
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_store_round_trips_outcome_record() {
        let store = InMemoryTriggeredRunDeliveryStore::default();
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
        let store = InMemoryTriggeredRunDeliveryStore::default();

        let loaded = store
            .load_triggered_run_delivery(TurnRunId::new())
            .await
            .expect("missing load succeeds");

        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn in_memory_store_overwrites_on_second_write() {
        let store = InMemoryTriggeredRunDeliveryStore::default();
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
