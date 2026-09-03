//! The Telegram reply half: a `message`-cadence [`ReplySink`] that
//! materializes the terminal document through the same Bot API send path
//! `deliver` uses, keyed for idempotency on its checkpoint.

use std::time::Duration;

use ironclaw_extension_contracts::channel_adapter::ChannelError;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::reply::{
    ReplyAnswerText, ReplyAudience, ReplyDisplayText, ReplyDocument, ReplyProviderRef,
    ReplyReconcilePoint, ReplyReconcileRequest, ReplyRevision, ReplySink, ReplySinkCheckpoint,
    ReplySinkOutcome, ReplyTarget, ReplyThreadAnchor,
};
use ironclaw_extension_contracts::tool_adapter::{RestrictedEgressError, RestrictedEgressResponse};
use ironclaw_host_api::attachment::WorkspaceFile;
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope};

use super::*;
use crate::TelegramChannelAdapter;
use crate::channel::deliver_tests::{ScriptedEgress, workspace_file};

enum Terminal {
    Completed,
    Failed(&'static str),
    Cancelled,
}

fn terminal_document(answer: &str, terminal: Terminal) -> ReplyDocument {
    let mut document = ReplyDocument::default();
    document.finalize_answer(
        ReplyAnswerText::new(answer).expect("answer text"),
        Vec::new(),
    );
    match terminal {
        Terminal::Completed => document.complete(),
        Terminal::Failed(summary) => {
            document.fail(ReplyDisplayText::new(summary).expect("summary"))
        }
        Terminal::Cancelled => document.cancel(),
    };
    document
}

fn request(
    document: ReplyDocument,
    point: ReplyReconcilePoint,
    checkpoint: Option<ReplySinkCheckpoint>,
    attachments: Vec<WorkspaceFile>,
) -> ReplyReconcileRequest {
    let run_id = TurnRunId::new();
    let user = || UserId::new("reply-user").expect("user id");
    ReplyReconcileRequest {
        revision: ReplyRevision {
            revision: 2,
            document,
        },
        point,
        target: ReplyTarget {
            scope: TurnScope::new_with_owner(
                TenantId::new("reply-tenant").expect("tenant id"),
                None,
                None,
                ThreadId::new("reply-thread").expect("thread id"),
                Some(user()),
            ),
            actor: TurnActor::new(user()),
            run_id,
            conversation: Some(
                ExternalConversationRef::new(None, "8675309", None, None).expect("conversation"),
            ),
            thread_anchor: Some(ReplyThreadAnchor::new("77").expect("anchor")),
            audience: ReplyAudience::Private,
        },
        reply_context: None,
        checkpoint,
        extension_generation: 1,
        materialized_attachments: attachments,
    }
}

fn terminal_request(document: ReplyDocument) -> ReplyReconcileRequest {
    request(document, ReplyReconcilePoint::Terminal, None, Vec::new())
}

fn sent(message_id: u64) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
    ScriptedEgress::ok(&format!(
        r#"{{"ok":true,"result":{{"message_id":{message_id}}}}}"#
    ))
}

fn rate_limited(
    retry_after: Option<Duration>,
) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
    Ok(RestrictedEgressResponse {
        retry_after,
        status: 429,
        body: br#"{"ok":false,"error_code":429,"description":"Too Many Requests"}"#.to_vec(),
    })
}

fn checkpoint_json(checkpoint: &ReplySinkCheckpoint) -> serde_json::Value {
    assert_eq!(checkpoint.version(), TELEGRAM_REPLY_CHECKPOINT_VERSION);
    serde_json::from_str(checkpoint.payload()).expect("checkpoint payload is JSON")
}

fn provider_refs(report: &ReplySinkReport) -> Vec<&str> {
    report
        .evidence
        .provider_refs
        .iter()
        .map(ReplyProviderRef::as_str)
        .collect()
}

fn sent_texts(egress: &ScriptedEgress) -> Vec<String> {
    egress
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .map(|request| {
            let body: serde_json::Value =
                serde_json::from_slice(request.body.as_deref().unwrap_or_default())
                    .expect("sendMessage body is JSON");
            body["text"].as_str().expect("sendMessage text").to_string()
        })
        .collect()
}

#[tokio::test]
async fn terminal_completed_renders_answer_then_attachments_and_checkpoints_message_refs() {
    let egress = ScriptedEgress::new(vec![sent(41), sent(42)]);
    let report = TelegramChannelAdapter::default()
        .reconcile(
            request(
                terminal_document("final answer", Terminal::Completed),
                ReplyReconcilePoint::Terminal,
                None,
                vec![workspace_file(
                    Some("report.pdf"),
                    "application/pdf",
                    b"pdf bytes",
                )],
            ),
            &egress,
        )
        .await
        .expect("a terminal reconcile drives the vendor");

    assert!(report.outcome.is_applied(), "{:?}", report.outcome);
    assert_eq!(provider_refs(&report), ["41", "42"]);
    assert!(
        !report.evidence.read_back_verified,
        "the Bot API offers no read-back; the sink must not claim one"
    );
    let checkpoint = report
        .checkpoint
        .expect("a fully applied terminal render mints a checkpoint");
    assert_eq!(
        checkpoint_json(&checkpoint),
        serde_json::json!({ "terminal_applied": true, "message_refs": ["41", "42"] })
    );

    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "one sendMessage, then one sendDocument");
    assert!(requests[0].url.ends_with("/sendMessage"));
    let body: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_deref().unwrap_or_default()).unwrap();
    assert_eq!(body["chat_id"], "8675309");
    assert_eq!(body["text"], "final answer");
    assert_eq!(
        body["message_thread_id"], 77,
        "the reply target's thread anchor threads the answer"
    );
    assert!(requests[1].url.ends_with("/sendDocument"));
    let multipart = String::from_utf8_lossy(requests[1].body.as_deref().unwrap_or_default());
    assert!(multipart.contains("name=\"document\"; filename=\"report.pdf\""));
    assert!(multipart.contains("name=\"message_thread_id\"\r\n\r\n77"));
}

#[tokio::test]
async fn terminal_failed_renders_the_failure_summary_only() {
    let egress = ScriptedEgress::new(vec![sent(43)]);
    let report = TelegramChannelAdapter::default()
        .reconcile(
            terminal_request(terminal_document(
                "partial answer that must not ship",
                Terminal::Failed("That run didn't finish."),
            )),
            &egress,
        )
        .await
        .expect("a failed terminal reconcile drives the vendor");

    assert!(report.outcome.is_applied(), "{:?}", report.outcome);
    assert_eq!(provider_refs(&report), ["43"]);
    assert_eq!(sent_texts(&egress), ["That run didn't finish."]);
}

#[tokio::test]
async fn terminal_cancelled_renders_a_stopped_line() {
    let egress = ScriptedEgress::new(vec![sent(44)]);
    let report = TelegramChannelAdapter::default()
        .reconcile(
            terminal_request(terminal_document("half an answer", Terminal::Cancelled)),
            &egress,
        )
        .await
        .expect("a cancelled terminal reconcile drives the vendor");

    assert!(report.outcome.is_applied(), "{:?}", report.outcome);
    assert_eq!(sent_texts(&egress), [TELEGRAM_REPLY_CANCELLED_TEXT]);
    assert_eq!(
        checkpoint_json(&report.checkpoint.expect("checkpoint")),
        serde_json::json!({ "terminal_applied": true, "message_refs": ["44"] })
    );
}

#[tokio::test]
async fn repeated_terminal_reconcile_with_the_applied_checkpoint_does_not_resend() {
    let applied = ReplySinkCheckpoint::new(
        TELEGRAM_REPLY_CHECKPOINT_VERSION,
        r#"{"terminal_applied":true,"message_refs":["41","42"]}"#,
    )
    .expect("checkpoint");
    let egress = ScriptedEgress::new(Vec::new());
    let report = TelegramChannelAdapter::default()
        .reconcile(
            request(
                terminal_document("final answer", Terminal::Completed),
                ReplyReconcilePoint::Terminal,
                Some(applied.clone()),
                Vec::new(),
            ),
            &egress,
        )
        .await
        .expect("a repeated terminal reconcile is idempotent");

    assert!(report.outcome.is_applied(), "{:?}", report.outcome);
    assert!(
        egress.requests.lock().unwrap().is_empty(),
        "an applied checkpoint must short-circuit every provider call"
    );
    assert_eq!(
        provider_refs(&report),
        ["41", "42"],
        "the recorded refs are re-reported as evidence"
    );
    assert_eq!(report.checkpoint, Some(applied));
}

#[tokio::test]
async fn non_terminal_points_are_no_ops_that_keep_the_checkpoint() {
    let prior = ReplySinkCheckpoint::new(TELEGRAM_REPLY_CHECKPOINT_VERSION, r#"{"unused":true}"#)
        .expect("checkpoint");
    for point in [
        ReplyReconcilePoint::Opened,
        ReplyReconcilePoint::Progress,
        ReplyReconcilePoint::ControlCritical,
        ReplyReconcilePoint::Heartbeat,
    ] {
        let egress = ScriptedEgress::new(Vec::new());
        let report = TelegramChannelAdapter::default()
            .reconcile(
                request(
                    terminal_document("progress", Terminal::Completed),
                    point,
                    Some(prior.clone()),
                    Vec::new(),
                ),
                &egress,
            )
            .await
            .expect("a non-terminal point never errors");
        assert!(
            report.outcome.is_applied(),
            "{point:?}: {:?}",
            report.outcome
        );
        assert!(
            egress.requests.lock().unwrap().is_empty(),
            "{point:?} must not reach the vendor"
        );
        assert_eq!(
            report.checkpoint,
            Some(prior.clone()),
            "{point:?} must hand the incoming checkpoint back unchanged"
        );
        assert!(report.evidence.provider_refs.is_empty());
    }
}

#[tokio::test]
async fn a_rate_limited_first_part_is_retryable_with_the_provider_retry_after_hint() {
    let egress = ScriptedEgress::new(vec![rate_limited(Some(Duration::from_secs(7)))]);
    let report = TelegramChannelAdapter::default()
        .reconcile(
            terminal_request(terminal_document("final answer", Terminal::Completed)),
            &egress,
        )
        .await
        .expect("a vendor rejection is a report, not an error");

    assert!(
        matches!(
            &report.outcome,
            ReplySinkOutcome::Retryable { retry_after, .. }
                if *retry_after == Some(Duration::from_secs(7))
        ),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        report.checkpoint, None,
        "nothing was accepted, so there is nothing to checkpoint"
    );
    assert!(report.evidence.provider_refs.is_empty());

    // Without a hint the outcome is still retryable; pacing falls to the host.
    let egress = ScriptedEgress::new(vec![rate_limited(None)]);
    let report = TelegramChannelAdapter::default()
        .reconcile(
            terminal_request(terminal_document("final answer", Terminal::Completed)),
            &egress,
        )
        .await
        .expect("reconcile");
    assert!(matches!(
        report.outcome,
        ReplySinkOutcome::Retryable {
            retry_after: None,
            ..
        }
    ));
}

#[tokio::test]
async fn a_partially_sent_answer_is_permanent_and_never_resent() {
    // 5000 chars split at the 4096-unit limit: chunk one lands, chunk two
    // is rate-limited. OUT-7: the accepted chunk must never be re-posted.
    let long_answer = "line\n".repeat(1_000);
    let egress = ScriptedEgress::new(vec![sent(1), rate_limited(Some(Duration::from_secs(3)))]);
    let report = TelegramChannelAdapter::default()
        .reconcile(
            terminal_request(terminal_document(&long_answer, Terminal::Completed)),
            &egress,
        )
        .await
        .expect("a partial send is a report, not an error");

    let ReplySinkOutcome::Permanent { reason } = &report.outcome else {
        panic!("a retryable failure after an accepted part must be Permanent: {report:?}");
    };
    assert!(
        reason.as_str().contains("already accepted"),
        "the reason must say why the failure is terminal: {reason}"
    );
    assert_eq!(egress.requests.lock().unwrap().len(), 2);
    assert_eq!(provider_refs(&report), ["1"]);
    let partial = report
        .checkpoint
        .expect("a partial render checkpoints what the provider accepted");
    assert_eq!(
        checkpoint_json(&partial),
        serde_json::json!({ "terminal_applied": false, "message_refs": ["1"] })
    );

    // A later reconcile carrying the partial checkpoint stays Permanent and
    // never reaches the vendor: the sink cannot resume without duplicating.
    let egress = ScriptedEgress::new(vec![sent(2), sent(3)]);
    let repeat = TelegramChannelAdapter::default()
        .reconcile(
            request(
                terminal_document(&long_answer, Terminal::Completed),
                ReplyReconcilePoint::Terminal,
                Some(partial.clone()),
                Vec::new(),
            ),
            &egress,
        )
        .await
        .expect("reconcile");
    assert!(
        matches!(repeat.outcome, ReplySinkOutcome::Permanent { .. }),
        "{:?}",
        repeat.outcome
    );
    assert!(egress.requests.lock().unwrap().is_empty());
    assert_eq!(provider_refs(&repeat), ["1"]);
    assert_eq!(repeat.checkpoint, Some(partial));
}

#[tokio::test]
async fn a_checkpoint_of_an_unknown_version_is_treated_as_not_applied() {
    let foreign = ReplySinkCheckpoint::new(
        TELEGRAM_REPLY_CHECKPOINT_VERSION + 1,
        r#"{"terminal_applied":true,"message_refs":["41"]}"#,
    )
    .expect("checkpoint");
    let egress = ScriptedEgress::new(vec![sent(45)]);
    let report = TelegramChannelAdapter::default()
        .reconcile(
            request(
                terminal_document("final answer", Terminal::Completed),
                ReplyReconcilePoint::Terminal,
                Some(foreign),
                Vec::new(),
            ),
            &egress,
        )
        .await
        .expect("reconcile");

    assert!(report.outcome.is_applied(), "{:?}", report.outcome);
    assert_eq!(
        sent_texts(&egress),
        ["final answer"],
        "an unreadable checkpoint carries no evidence, so the answer is rendered"
    );
    assert_eq!(
        checkpoint_json(&report.checkpoint.expect("checkpoint")),
        serde_json::json!({ "terminal_applied": true, "message_refs": ["45"] }),
        "the fresh render mints a checkpoint of this sink's own version"
    );
}

#[tokio::test]
async fn an_empty_completed_answer_applies_without_a_provider_call() {
    let egress = ScriptedEgress::new(Vec::new());
    let report = TelegramChannelAdapter::default()
        .reconcile(
            terminal_request(terminal_document("  \n", Terminal::Completed)),
            &egress,
        )
        .await
        .expect("nothing to render is not an error");

    assert!(report.outcome.is_applied(), "{:?}", report.outcome);
    assert!(
        egress.requests.lock().unwrap().is_empty(),
        "Telegram rejects empty text; an empty answer must not be posted"
    );
    assert_eq!(
        checkpoint_json(&report.checkpoint.expect("checkpoint")),
        serde_json::json!({ "terminal_applied": true, "message_refs": [] })
    );
}

#[tokio::test]
async fn a_target_without_a_vendor_conversation_is_permanent() {
    let egress = ScriptedEgress::new(vec![sent(46)]);
    let mut request = terminal_request(terminal_document("final answer", Terminal::Completed));
    request.target.conversation = None;
    let report = TelegramChannelAdapter::default()
        .reconcile(request, &egress)
        .await
        .expect("reconcile");

    assert!(
        matches!(report.outcome, ReplySinkOutcome::Permanent { .. }),
        "{:?}",
        report.outcome
    );
    assert!(egress.requests.lock().unwrap().is_empty());
    assert_eq!(report.checkpoint, None);
}

#[tokio::test]
async fn a_terminal_point_without_a_terminal_outcome_is_a_render_error() {
    let mut document = ReplyDocument::default();
    document.append_answer("still going");
    let egress = ScriptedEgress::new(vec![sent(47)]);
    let error = TelegramChannelAdapter::default()
        .reconcile(terminal_request(document), &egress)
        .await
        .expect_err("a terminal point must carry a terminal outcome");
    assert!(matches!(error, ChannelError::Render { .. }), "{error:?}");
    assert!(egress.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn first_part_failures_map_one_to_one_onto_sink_outcomes() {
    for (response, expected) in [
        (
            ScriptedEgress::ok(r#"{"ok":false,"error_code":403,"description":"Forbidden"}"#),
            "unauthorized",
        ),
        (
            Err(RestrictedEgressError::AuthRequired {
                required_secrets: Vec::new(),
                credential_requirements: Vec::new(),
            }),
            "unauthorized",
        ),
        (ScriptedEgress::ok("not-json"), "ambiguous"),
        (
            Err(RestrictedEgressError::Transport {
                reason: "connection reset".to_string(),
            }),
            "ambiguous",
        ),
        (
            ScriptedEgress::ok(r#"{"ok":false,"error_code":400,"description":"chat not found"}"#),
            "permanent",
        ),
    ] {
        let egress = ScriptedEgress::new(vec![response]);
        let report = TelegramChannelAdapter::default()
            .reconcile(
                terminal_request(terminal_document("final answer", Terminal::Completed)),
                &egress,
            )
            .await
            .expect("reconcile");
        assert_eq!(
            report.outcome.kind_name(),
            expected,
            "unexpected outcome: {:?}",
            report.outcome
        );
        assert_eq!(report.checkpoint, None);
        assert!(report.evidence.provider_refs.is_empty());
    }
}
