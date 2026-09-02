//! Ambiguity and read-back: the pending/checkpoint machinery that keeps a
//! lost provider response from ever duplicating visible text. Split from the
//! parent suite by theme; it drives the same harness. Text goes out by whole
//! paragraph, so every fragment here ends one.

use super::*;

// ── Ambiguity and read-back ──────────────────────────────────────────────

#[tokio::test]
async fn a_transport_failure_after_an_append_is_ambiguous_and_read_back_decides_the_continuation() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    // The append reached Slack; only the answer was lost.
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world\n\n");
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

    harness.append("!\n\n");
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
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "!\n\n" }] })
        ),
        "only the NEW delta is appended; the landed one is not repeated"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello\n\n world\n\n!\n\n"
    );

    // The append never reached Slack.
    harness.fake.inject(Fault::TransportBeforeAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" Bye\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));

    harness.append(".\n\n");
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
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": " Bye\n\n.\n\n" }] })
        ),
        "the lost delta is re-sent together with the new one"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello\n\n world\n\n!\n\n Bye\n\n.\n\n"
    );
}

/// A read-back that FINDS the streaming message but gets no `text` field
/// (Slack's shape for a blocks-only rendering) proves nothing about a
/// text-carrying pending: the sink must stay ambiguous and never re-send
/// the fragment the user may already see.
#[tokio::test]
async fn a_found_but_textless_read_back_stays_ambiguous_without_resending() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world\n\n");
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
    harness.append("Hello\n\n");
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
    harness.append(" world\n\n");
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
        15,
        "the checkpoint advances over the verified delta"
    );
}

/// A pending delta that repeats text the message already shows ("ok" after
/// "ok") is proven only by an occurrence AFTER the applied text: the old
/// occurrence is not evidence, or the new copy is lost as "landed". And a
/// delta with no comparable text at all (punctuation, whitespace) cannot be
/// proven either way — it stays ambiguous rather than being re-sent.
#[tokio::test]
async fn read_back_proves_a_repeated_delta_only_past_the_applied_text() {
    // Not landed: the earlier "ok" is in the message, the pending one is not.
    let mut harness = Harness::dm();
    harness.append("ok\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();
    harness.fake.inject(Fault::TransportBeforeAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append("ok\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));

    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(
        !report.evidence.read_back_verified,
        "the earlier occurrence of the same text proves nothing about the pending delta"
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .last(),
        Some(
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "ok\n\n" }] })
        ),
        "the lost delta is re-sent"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "ok\n\nok\n\n"
    );

    // Landed: the second "ok" is in the message past the first.
    let mut harness = Harness::dm();
    harness.append("ok\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append("ok\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    let appends_before = harness
        .fake
        .bodies(SlackWebApiMethod::ChatAppendStream)
        .len();

    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(report.evidence.read_back_verified);
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .len(),
        appends_before,
        "a landed delta is never re-sent"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "ok\n\nok\n\n"
    );
    assert_eq!(harness.checkpoint_json()["stream"]["appended_chars"], 8);

    // No comparable text: a punctuation-only delta leaves read-back nothing
    // to find, so the pending stays and nothing is re-sent.
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append("!\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    let appends_before = harness
        .fake
        .bodies(SlackWebApiMethod::ChatAppendStream)
        .len();

    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "a delta with no comparable text cannot be verified, got {:?}",
        report.outcome
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .len(),
        appends_before,
        "nothing is re-sent while the delta's fate is unknown"
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the pending stays on the checkpoint"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello\n\n!\n\n",
        "the text the user sees is never doubled"
    );
}

/// A revoked or invalid token on the read-back itself is the contract's
/// `Unauthorized`, not an ambiguity to retry until the terminal budget
/// lapses: nothing can be verified or sent with that token.
#[tokio::test]
async fn an_unauthorized_read_back_is_reported_as_unauthorized() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    let appends_before = harness
        .fake
        .bodies(SlackWebApiMethod::ChatAppendStream)
        .len();

    harness.fake.inject(Fault::SlackError {
        method: SlackWebApiMethod::ConversationsReplies,
        error: "invalid_auth",
    });
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    let ReplySinkOutcome::Unauthorized { reason } = &report.outcome else {
        panic!("expected Unauthorized, got {:?}", report.outcome);
    };
    assert!(
        reason.as_str().contains("invalid_auth")
            && reason.as_str().contains("conversations.replies"),
        "the reason names the method and the Slack error: {reason}"
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .len(),
        appends_before,
        "nothing is re-sent on a token that cannot read"
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the pending stays on the checkpoint: nothing was resolved"
    );
}

// ── Unreadable 2xx answers ───────────────────────────────────────────────

/// A 2xx answer this sink cannot read crossed transport and was acted on —
/// the same shape as a lost answer. For `chat.startStream` that means a
/// ghost stream with no handle: the checkpoint latches it and the sink never
/// opens a second stream nor posts the terminal beside it.
#[tokio::test]
async fn an_unreadable_stream_open_answer_latches_the_ghost_stream_ambiguity() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    harness.fake.inject(Fault::InvalidBody {
        method: SlackWebApiMethod::ChatStartStream,
    });
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert_eq!(
        harness.checkpoint_json()["stream_open_ambiguous"],
        Value::Bool(true),
        "an unreadable open answer is an unanswered open"
    );

    harness.append(" world\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    harness.document.complete();
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    let calls = harness.fake.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.as_str() == "chat.startStream")
            .count(),
        1,
        "exactly one chat.startStream ever went out: {calls:?}"
    );
    assert_eq!(
        harness.fake.streams().len(),
        1,
        "the ghost stream is the only stream"
    );
    assert!(harness.fake.posted().is_empty());
}

/// For `chat.appendStream` an unreadable 2xx answer arms the pending exactly
/// as a lost answer does, so read-back — not a blind retry — decides whether
/// the delta is appended again.
#[tokio::test]
async fn an_unreadable_append_answer_arms_the_pending_for_read_back() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.fake.inject(Fault::InvalidBody {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the checkpoint remembers the unreadable append"
    );

    harness.append("!\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(
        report.evidence.read_back_verified,
        "read-back proved the append landed"
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .last(),
        Some(
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "!\n\n" }] })
        ),
        "only the NEW delta is appended"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello\n\n world\n\n!\n\n"
    );
}

/// A read-back that answers `ok` without a `messages` array is not "the
/// message is gone" (which would re-send the delta): it proves nothing, so a
/// text-carrying pending stays ambiguous until a read-back that can compare.
#[tokio::test]
async fn a_read_back_without_a_messages_array_proves_nothing_and_re_sends_nothing() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    let appends_before = harness
        .fake
        .bodies(SlackWebApiMethod::ChatAppendStream)
        .len();

    harness.fake.inject(Fault::BareOk {
        method: SlackWebApiMethod::ConversationsReplies,
    });
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "a shapeless read-back cannot verify a text-carrying pending, got {:?}",
        report.outcome
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .len(),
        appends_before,
        "nothing is re-sent on a read-back that proves nothing"
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the pending stays for a read-back that can compare"
    );

    harness.append("!\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(report.evidence.read_back_verified);
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello\n\n world\n\n!\n\n"
    );
}
