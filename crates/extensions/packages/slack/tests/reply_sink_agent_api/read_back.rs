//! Ambiguity and read-back: the pending/checkpoint machinery that keeps a
//! lost provider response from ever duplicating visible text. Split from the
//! parent suite by theme; it drives the same harness.

use super::*;

// ── Ambiguity and read-back ──────────────────────────────────────────────

#[tokio::test]
async fn a_transport_failure_after_an_append_is_ambiguous_and_read_back_decides_the_continuation() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    // The append reached Slack; only the answer was lost.
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the checkpoint remembers the unanswered request"
    );

    harness.append("!");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(
        report.evidence.read_back_verified,
        "the read-back proved the ambiguous append landed"
    );
    let calls = harness.fake.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        ["conversations.replies", "chat.appendStream"],
        "read back BEFORE appending more"
    );
    let read_back = harness
        .fake
        .requests()
        .into_iter()
        .find(|request| request.url.contains("conversations.replies"))
        .expect("read-back request");
    assert!(
        read_back.url.contains(&format!("channel={DM}"))
            && read_back.url.contains(&format!("ts={ts}")),
        "read-back addresses the streaming message: {}",
        read_back.url
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .last(),
        Some(
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "!" }] })
        ),
        "only the NEW delta is appended; the landed one is not repeated"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello world!"
    );

    // The append never reached Slack.
    harness.fake.inject(Fault::TransportBeforeAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" Bye");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));

    harness.append(".");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(
        !report.evidence.read_back_verified,
        "a read-back that shows the append missing verifies nothing"
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .last(),
        Some(
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": " Bye." }] })
        ),
        "the lost delta is re-sent together with the new one"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello world! Bye."
    );
}

/// A read-back that FINDS the streaming message but gets no `text` field
/// (Slack's shape for a blocks-only rendering) proves nothing about a
/// text-carrying pending: the sink must stay ambiguous and never re-send
/// the fragment the user may already see.
#[tokio::test]
async fn a_found_but_textless_read_back_stays_ambiguous_without_resending() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    let appends_before = harness
        .fake
        .bodies(SlackWebApiMethod::ChatAppendStream)
        .len();

    harness.fake.omit_read_back_text();
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "a textless read-back cannot verify a text-carrying pending, got {:?}",
        report.outcome
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .len(),
        appends_before,
        "nothing is re-sent while the pending append is unverifiable"
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the pending stays on the checkpoint for the next verification"
    );
}

/// An attention block streamed between answer fragments must not defeat the
/// read-back: the pending append's own delta is contiguous in the message
/// even though the FULL answer prefix is not. A false "did not land" here
/// re-sends text the user already sees.
#[tokio::test]
async fn read_back_verifies_a_pending_append_across_an_earlier_attention_block() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    harness.document.require_attention(ReplyAttention {
        kind: ReplyAttentionKind::Approval,
        headline: text("Approve the write"),
        body: None,
        action_url: None,
        gate_ref: Some(text("gate:x")),
    });
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );
    harness.document.clear_attention();
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );

    // The next answer fragment reaches Slack but the response is lost.
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    let appends_before = harness
        .fake
        .bodies(SlackWebApiMethod::ChatAppendStream)
        .len();

    // Read-back sees "Hello" + the attention markdown + " world": the full
    // answer prefix is NOT contiguous, the pending delta is.
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(
        report.evidence.read_back_verified,
        "the delta landed and read-back proves it"
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .len(),
        appends_before,
        "a landed append is never re-sent: the user would see the text twice"
    );
    assert_eq!(
        harness.checkpoint_json()["stream"]["appended_chars"],
        11,
        "the checkpoint advances over the verified delta"
    );
}
