//! The one impure edge of the timing lane: read the process-local diagnostic
//! store for a run and hand the snapshot to the pure projection.
//!
//! Mirrors `artifact_logs`: best-effort, returns a value on every path, and
//! never fails an artifact export. A user downloading evidence for a bug
//! report must always get a file.

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::ThreadId;
use ironclaw_product_contracts::inspector::DiagnosticScope;
use ironclaw_product_contracts::surface::ProductSurfaceCaller;
use ironclaw_product_contracts::views::RebornViewProvider;
use ironclaw_turns::TurnRunId;

use crate::reborn_services::run_artifact::RunArtifactMessage;
use crate::reborn_services::run_artifact::timings::{
    RunArtifactTimings, project_timings, unavailable,
};
use crate::reborn_services::{ProductCapabilityInvoker, RebornServices};

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub(super) fn artifact_timings<'a, M>(
        &self,
        caller: &ProductSurfaceCaller,
        thread_id: &ThreadId,
        run_id: &TurnRunId,
        run_received_at: DateTime<Utc>,
        messages: M,
    ) -> RunArtifactTimings
    where
        M: IntoIterator<Item = &'a RunArtifactMessage>,
    {
        // Same keying as the operator inspector (`inspector.rs::diagnostic_scope`).
        // On the admin thread-scrape route the caller was already rebound to the
        // scraped user by `thread_scrape_subject`, so this needs no branch.
        let scope = DiagnosticScope::new(
            caller.tenant_id.clone(),
            caller.user_id.clone(),
            thread_id.clone(),
            *run_id,
        );
        let wall_clock_ms = derive_wall_clock_ms(run_received_at, messages);
        match self.diagnostic_store.timing_snapshot(&scope) {
            Ok(Some(snapshot)) => project_timings(snapshot, wall_clock_ms),
            Ok(None) => {
                let mut timings = unavailable("run_not_resident");
                timings.totals.wall_clock_ms = wall_clock_ms;
                timings
            }
            Err(error) => {
                // debug!, not info!/warn!: the operator log buffer captures
                // INFO+ and those entries are embedded into this same
                // artifact's `logs` block (see `build_run_artifact` in
                // `run_artifact.rs`).
                tracing::debug!(
                    ?error,
                    "run artifact exported without optional process-local timings"
                );
                let mut timings = unavailable("diagnostic_store_unavailable");
                timings.totals.wall_clock_ms = wall_clock_ms;
                timings
            }
        }
    }
}

/// run.received_at → newest message `updated_at`, in milliseconds.
///
/// Approximate by design: it folds in queue and persistence latency around
/// the run. `None` when no message carries a timestamp (pre-timestamp
/// records) or when the span is negative, which only happens if clocks
/// disagree — report nothing rather than a nonsense number.
pub(super) fn derive_wall_clock_ms<'a>(
    run_received_at: DateTime<Utc>,
    messages: impl IntoIterator<Item = &'a RunArtifactMessage>,
) -> Option<u64> {
    let newest = messages
        .into_iter()
        .filter_map(|message| message.updated_at)
        .max()?;
    u64::try_from((newest - run_received_at).num_milliseconds()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_threads::{MessageKind, MessageStatus};

    fn message(updated_at: Option<DateTime<Utc>>) -> RunArtifactMessage {
        RunArtifactMessage {
            message_id: "message-a".to_string(),
            sequence: 1,
            run_id: Some("run-a".to_string()),
            kind: MessageKind::Assistant,
            status: MessageStatus::Finalized,
            content: "hello".to_string(),
            tool_call: None,
            created_at: None,
            updated_at,
        }
    }

    #[test]
    fn wall_clock_spans_run_receipt_to_the_newest_message_update() {
        let received = Utc::now();
        let messages = [
            message(Some(received + chrono::Duration::seconds(12))),
            message(Some(received + chrono::Duration::seconds(91))),
            message(Some(received + chrono::Duration::seconds(40))),
        ];

        assert_eq!(
            derive_wall_clock_ms(received, messages.iter()),
            Some(91_000)
        );
    }

    #[test]
    fn wall_clock_is_absent_when_no_message_carries_a_timestamp() {
        let received = Utc::now();
        assert_eq!(
            derive_wall_clock_ms(received, [message(None), message(None)].iter()),
            None
        );
    }

    #[test]
    fn wall_clock_is_absent_rather_than_negative_when_clocks_disagree() {
        let received = Utc::now();
        let messages = [message(Some(received - chrono::Duration::seconds(5)))];
        assert_eq!(derive_wall_clock_ms(received, messages.iter()), None);
    }
}
