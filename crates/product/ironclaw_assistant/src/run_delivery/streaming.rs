//! Live-text streaming forwarder for working-indicator streams.
//!
//! When a run's working indicator is a vendor-native stream (Slack
//! `chat.startStream`), the observer spawns one forwarder task per run that
//! subscribes to the same live projection feed the WebUI drains
//! ([`ProjectionStream`]) and converts the cumulative model text into append
//! deltas for [`RunDeliveryServices::append_to_stream`].
//!
//! Two invariants keep the streamed answer correct:
//!
//! - **`appended` only advances on success.** The suffix is retained (and
//!   re-flushed) when the append fails, and `appended` tracks what Slack
//!   actually has — never what the model produced. The completion stop
//!   computes its LCP-tail against `appended`, so a missed or failed append
//!   widens the tail instead of losing text.
//! - **Cumulative-body prefix check.** Live text is replaceable state (a
//!   regeneration replaces a phase's body). A body that no longer extends
//!   what we already sent holds appends until it realigns; the tail carries
//!   the corrected remainder at completion.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironclaw_extension_contracts::channel_adapter::OutboundPart;
use ironclaw_product_contracts::outbound::{
    ProductOutboundEnvelope, ProductOutboundPayload, ProductProjectionItem,
};
use ironclaw_product_contracts::projection::{
    ProjectionStreamSubscription, ProjectionSubscriptionRequest,
};
use ironclaw_turns::TurnActor;
use ironclaw_turns::{TurnRunId, TurnScope};
use tokio::sync::oneshot;

use super::{PendingStreamAppend, PostedWorkingNotice, RunDeliveryServices};

/// Flush a pending suffix when it reaches this many characters, even if the
/// idle window has not elapsed (Slack rate-limit friendliness: the WebUI's
/// 16 ms coalescing cadence is not appropriate for `chat.appendStream`).
const STREAM_APPEND_CHUNK_CHARS: usize = 250;
/// Flush any pending suffix after this idle window without new model text.
const STREAM_APPEND_IDLE: Duration = Duration::from_millis(500);

/// Handle to one run's live-text forwarder task. Shutting it down stops
/// appends (the task flushes pending text first) and AWAITS the task, so
/// `appended_text` read afterwards is stable — the completion stop's LCP-tail
/// is computed only after the final flush, never racing a concurrent append.
pub(crate) struct StreamForwarderHandle {
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<tokio::task::JoinHandle<()>>,
    appended: Arc<Mutex<String>>,
}

impl StreamForwarderHandle {
    /// Stop the forwarder (idempotent) and wait for its final flush.
    pub(crate) async fn shutdown(self) {
        let _ = self.shutdown_and_appended().await;
    }

    /// Stop the forwarder, wait for its final flush, and return the text
    /// Slack actually accepted. The completion stop must use this: reading
    /// the ledger before the flush would race a concurrent append.
    pub(crate) async fn shutdown_and_appended(mut self) -> String {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
        self.appended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Spawn the forwarder for one streamed run. `notice` must be the streamed
/// working notice (its vendor ref is the stream `ts`).
pub(crate) fn spawn_stream_forwarder(
    services: Arc<RunDeliveryServices>,
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    notice: PostedWorkingNotice,
) -> StreamForwarderHandle {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let appended = Arc::new(Mutex::new(String::new()));
    let appended_for_task = Arc::clone(&appended);
    let mut handle = StreamForwarderHandle {
        shutdown: Some(shutdown_tx),
        join: None,
        appended,
    };
    handle.join = Some(tokio::spawn(async move {
        let subscription = match services
            .projection_stream
            .subscribe(ProjectionSubscriptionRequest {
                actor,
                scope: scope.clone(),
                after_cursor: None,
            })
            .await
        {
            Ok(subscription) => subscription,
            Err(error) => {
                // No subscription, no deltas — the LCP-tail stop delivers the
                // full answer; never a hard failure.
                tracing::debug!(
                    target: "ironclaw::reborn::run_delivery",
                    %run_id,
                    %error,
                    "live streaming subscription unavailable; answer will arrive in the stop"
                );
                return;
            }
        };
        forward_loop(
            services,
            scope,
            run_id,
            notice,
            subscription,
            appended_for_task,
            shutdown_rx,
        )
        .await;
    }));
    handle
}

async fn forward_loop(
    services: Arc<RunDeliveryServices>,
    scope: TurnScope,
    run_id: TurnRunId,
    notice: PostedWorkingNotice,
    mut subscription: ProjectionStreamSubscription,
    appended: Arc<Mutex<String>>,
    mut shutdown: oneshot::Receiver<()>,
) {
    // Cumulative text of the run's latest live Text items, keyed by item id
    // (a new phase restarts an id; the coalescer replaces a phase's body).
    let mut bodies: Vec<(String, String)> = Vec::new();
    // The model text already folded in (advances on every update). A failed
    // flush DROPS the pending suffix: the LCP-tail stop recomputes the
    // remainder from the appended ledger, so dropped text is recovered
    // exactly once at completion — while re-sending an ambiguously-failed
    // suffix could duplicate it (appendStream has no idempotency key, and
    // duplication is NOT recoverable by the tail).
    let mut incorporated = String::new();
    // The suffix not yet accepted by the vendor. Cleared on failure (see
    // above) and after success; never grows past one accumulation window.
    let mut pending = String::new();
    let mut flush_timer = tokio::time::interval(STREAM_APPEND_IDLE);
    flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Minimum inter-append pacing: the per-item chunk flush never fires more
    // often than the idle window, so bursty generation cannot exceed Slack
    // per-method rate limits and a sustained vendor failure cannot turn into
    // a per-item retry storm.
    let mut last_flush = std::time::Instant::now();

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                flush(&services, &scope, Some(run_id), &notice, &appended, &mut pending).await;
                break;
            }
            item = subscription.next() => {
                match item {
                    Some(Ok(envelope)) => {
                        if let Some(cumulative) =
                            live_cumulative_text(&envelope, run_id, &mut bodies)
                            && let Some(suffix) = next_suffix(&cumulative, &mut incorporated)
                        {
                            pending.push_str(&suffix);
                        }
                        // A body that no longer extends what we folded in
                        // (regeneration) holds until it realigns; the
                        // LCP-tail stop carries the corrected remainder.
                        if should_chunk_flush(
                            pending.chars().count(),
                            last_flush.elapsed(),
                        ) {
                            last_flush = std::time::Instant::now();
                            flush(&services, &scope, Some(run_id), &notice, &appended, &mut pending).await;
                        }
                    }
                    Some(Err(error)) => {
                        tracing::debug!(
                            target: "ironclaw::reborn::run_delivery",
                            %run_id,
                            %error,
                            "live streaming update failed; waiting for the next one"
                        );
                    }
                    None => {
                        // Subscription ended (e.g. stream manager shutdown).
                        // Stop appending; the completion tail recovers.
                        flush(&services, &scope, Some(run_id), &notice, &appended, &mut pending).await;
                        break;
                    }
                }
            }
            _ = flush_timer.tick() => {
                last_flush = std::time::Instant::now();
                flush(&services, &scope, Some(run_id), &notice, &appended, &mut pending).await;
            }
        }
    }
}

/// The next suffix to append for a cumulative body, or `None` when the body
/// does not extend what we already folded in (regeneration — appends hold
/// until realignment). Advances `incorporated` on extension; the suffix is
/// dropped on append failure (the LCP-tail stop recomputes the remainder),
/// never re-derived from a stale `incorporated`.
fn next_suffix(cumulative: &str, incorporated: &mut String) -> Option<String> {
    if cumulative.len() > incorporated.len() && cumulative.starts_with(incorporated.as_str()) {
        let suffix = cumulative[incorporated.len()..].to_string();
        *incorporated = cumulative.to_string();
        (!suffix.is_empty()).then_some(suffix)
    } else {
        None
    }
}

/// Whether the pending suffix should flush on the item path: at or above the
/// chunk threshold AND past the minimum inter-append window (rate-limit
/// pacing — bursty generation cannot exceed ~1 append per idle window, and a
/// sustained vendor failure cannot become a per-item retry storm).
fn should_chunk_flush(pending_chars: usize, since_last_flush: std::time::Duration) -> bool {
    pending_chars >= STREAM_APPEND_CHUNK_CHARS && since_last_flush >= STREAM_APPEND_IDLE
}

/// The run's cumulative live text: the concatenation of every Text item's
/// latest body in first-seen id order. Returns `None` when the run has no
/// Text items.
fn live_cumulative_text(
    envelope: &ProductOutboundEnvelope,
    run_id: TurnRunId,
    bodies: &mut Vec<(String, String)>,
) -> Option<String> {
    let state = match envelope.payload() {
        ProductOutboundPayload::ProjectionUpdate { state }
        | ProductOutboundPayload::ProjectionSnapshot { state } => state,
        _ => return None,
    };
    let mut changed = false;
    for item in &state.items {
        if let ProductProjectionItem::Text {
            id,
            run_id: Some(item_run),
            body,
        } = item
            && *item_run == run_id
        {
            match bodies.iter_mut().find(|(existing, _)| existing == id) {
                Some((_, existing_body)) => {
                    if existing_body != body {
                        *existing_body = body.clone();
                        changed = true;
                    }
                }
                None => {
                    bodies.push((id.clone(), body.clone()));
                    changed = true;
                }
            }
        }
    }
    if !changed {
        return None;
    }
    let mut cumulative = String::new();
    for (_, body) in bodies.iter() {
        cumulative.push_str(body);
    }
    Some(cumulative)
}

/// Send the pending suffix; on success advance the appended ledger (what
/// Slack actually has). On failure DROP the suffix: re-sending an
/// ambiguously-failed append could duplicate text (appendStream has no
/// idempotency key), and the LCP-tail stop recomputes the remainder from
/// the ledger at completion — dropped text is recovered exactly once.
async fn flush(
    services: &RunDeliveryServices,
    scope: &TurnScope,
    run_id: Option<TurnRunId>,
    notice: &PostedWorkingNotice,
    appended: &Arc<Mutex<String>>,
    pending: &mut String,
) {
    if pending.is_empty() {
        return;
    }
    let suffix = std::mem::take(pending);
    if services
        .append_to_stream(
            scope.clone(),
            run_id,
            PendingStreamAppend {
                conversation: notice.conversation.clone(),
                vendor_message_ref: notice.vendor_message_ref.clone(),
                suffix: suffix.clone(),
            },
        )
        .await
    {
        let mut ledger = appended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ledger.push_str(&suffix);
    } else {
        tracing::debug!(
            target: "ironclaw::reborn::run_delivery",
            "stream append not accepted; dropping the suffix (the completion tail recovers it exactly once)"
        );
    }
}

/// The completion stop parts for a streamed final reply, plus the tail they
/// carry: exactly ONE `StreamStop` whose `markdown_text` is the LCP-tail
/// (`final_text` minus the common prefix with what was already appended).
/// The adapter splits over the vendor's per-call cap internally, so the
/// observer always sees one part; a failed stop therefore means nothing was
/// sent (modulo an already-split >12k tail, where the re-drive may duplicate
/// accepted chunks — documented in the recovery comment). Returning the tail
/// keeps the stop-failure recovery's empty-vs-non-empty decision on the same
/// rule as the happy path.
pub(crate) fn stream_final_parts(
    vendor_message_ref: &str,
    final_text: &str,
    appended: &str,
) -> (Vec<OutboundPart>, String) {
    let common = common_prefix_len(appended, final_text);
    let tail = final_text.get(common..).unwrap_or_default().to_string();
    (
        vec![OutboundPart::StreamStop {
            vendor_message_ref: vendor_message_ref.to_string(),
            markdown_text: tail.clone(),
        }],
        tail,
    )
}

/// Byte length of the longest common prefix of `a` and `b`, measured on
/// character boundaries (never splits a multi-byte codepoint).
pub(crate) fn common_prefix_len(a: &str, b: &str) -> usize {
    let mut len = 0usize;
    for (left, right) in a.chars().zip(b.chars()) {
        if left != right {
            break;
        }
        len += left.len_utf8();
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::external::ExternalConversationRef;
    use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
    use ironclaw_product_contracts::outbound::{
        ProductOutboundEnvelope, ProductOutboundTarget, ProductProjectionState,
    };
    use ironclaw_turns::ReplyTargetBindingRef;

    fn conversation() -> ExternalConversationRef {
        ExternalConversationRef::new(
            None,
            "C123",
            Some("1700000000.000001"),
            Some("1700000000.000001"),
        )
        .expect("conversation")
    }

    fn text_envelope(run_id: TurnRunId, id: &str, body: &str) -> ProductOutboundEnvelope {
        ProductOutboundEnvelope::new(
            ProductAdapterId::new("slack").expect("adapter id"),
            AdapterInstallationId::new("install").expect("installation"),
            ProductOutboundTarget::new(
                ReplyTargetBindingRef::new("binding").expect("binding ref"),
                conversation(),
                None,
            ),
            ironclaw_product_contracts::outbound::ProjectionCursor::new("cursor:test")
                .expect("cursor"),
            ProductOutboundPayload::ProjectionUpdate {
                state: ProductProjectionState {
                    thread_id: "thread-1".to_string(),
                    items: vec![ProductProjectionItem::Text {
                        id: id.to_string(),
                        run_id: Some(run_id),
                        body: body.to_string(),
                    }],
                },
            },
        )
    }

    #[test]
    fn common_prefix_len_is_char_boundary_safe() {
        assert_eq!(common_prefix_len("héllo", "héllo world"), 6);
        assert_eq!(common_prefix_len("abc", "abd"), 2);
        assert_eq!(common_prefix_len("abc", "xyz"), 0);
        assert_eq!(common_prefix_len("abc", "abc"), 3);
        assert_eq!(common_prefix_len("héllo", "h"), 1);
    }

    #[test]
    fn stream_final_parts_carries_only_the_tail() {
        let (parts, tail) = stream_final_parts("ts-1", "Hello world", "Hello ");
        assert_eq!(tail, "world");
        assert!(matches!(
            &parts[..],
            [OutboundPart::StreamStop {
                vendor_message_ref,
                markdown_text,
            }] if vendor_message_ref == "ts-1" && markdown_text == "world"
        ));
    }

    #[test]
    fn stream_final_parts_with_empty_tail_is_an_empty_text_stop() {
        let (parts, tail) = stream_final_parts("ts-1", "Hello world", "Hello world");
        assert!(tail.is_empty());
        assert!(matches!(
            &parts[..],
            [OutboundPart::StreamStop {
                vendor_message_ref,
                markdown_text,
            }] if vendor_message_ref == "ts-1" && markdown_text.is_empty()
        ));
    }

    #[test]
    fn stream_final_parts_recovers_a_diverged_tail() {
        // Regeneration: the final text no longer extends what was appended.
        let (parts, tail) = stream_final_parts("ts-1", "REPLACED answer", "old prefix");
        assert_eq!(tail, "REPLACED answer");
        assert!(matches!(
            &parts[..],
            [OutboundPart::StreamStop {
                vendor_message_ref,
                markdown_text,
            }] if vendor_message_ref == "ts-1" && markdown_text == "REPLACED answer"
        ));
    }

    #[test]
    fn next_suffix_emits_only_the_extension_and_holds_on_divergence() {
        let mut incorporated = String::new();
        assert_eq!(
            next_suffix("Hello ", &mut incorporated).as_deref(),
            Some("Hello ")
        );
        assert_eq!(
            next_suffix("Hello world", &mut incorporated).as_deref(),
            Some("world")
        );
        assert_eq!(incorporated, "Hello world");
        // Regeneration: the new body no longer extends what we folded in —
        // holds (None) until a body realigns with the incorporated prefix.
        assert_eq!(next_suffix("REPLACED", &mut incorporated), None);
        assert_eq!(incorporated, "Hello world");
        assert_eq!(
            next_suffix("Hello world again", &mut incorporated).as_deref(),
            Some(" again")
        );
        // A body identical to what we folded in produces no suffix.
        assert_eq!(next_suffix("Hello world again", &mut incorporated), None);
    }

    #[test]
    fn should_chunk_flush_gates_on_threshold_and_inter_append_window() {
        let window = STREAM_APPEND_IDLE;
        assert!(!should_chunk_flush(STREAM_APPEND_CHUNK_CHARS - 1, window));
        assert!(should_chunk_flush(STREAM_APPEND_CHUNK_CHARS, window));
        // Within the pacing window the chunk flush defers to the idle timer.
        assert!(!should_chunk_flush(
            STREAM_APPEND_CHUNK_CHARS * 2,
            window - std::time::Duration::from_millis(1)
        ));
        assert!(should_chunk_flush(STREAM_APPEND_CHUNK_CHARS, window));
    }

    #[tokio::test]
    async fn live_cumulative_text_concatenates_phases_in_order() {
        let run_id = TurnRunId::new();
        let mut bodies: Vec<(String, String)> = Vec::new();
        let first = text_envelope(run_id, "text:r:0", "Hello ");
        assert_eq!(
            live_cumulative_text(&first, run_id, &mut bodies).as_deref(),
            Some("Hello ")
        );
        let second = text_envelope(run_id, "text:r:1", "world");
        assert_eq!(
            live_cumulative_text(&second, run_id, &mut bodies).as_deref(),
            Some("Hello world")
        );
        // A phase replacement (regeneration) replaces only that phase.
        let replaced = text_envelope(run_id, "text:r:0", "Hi ");
        assert_eq!(
            live_cumulative_text(&replaced, run_id, &mut bodies).as_deref(),
            Some("Hi world")
        );
        // A different run's item does not touch this run's cumulative text.
        let other = text_envelope(TurnRunId::new(), "text:o:0", "other");
        assert_eq!(live_cumulative_text(&other, run_id, &mut bodies), None);
    }
}
