use std::sync::Arc;

use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::channel_adapter::*;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::test_support::conformance::{
    ChannelAdapterConformance, ConformanceInbound, run_channel_adapter_conformance,
};
use ironclaw_extension_contracts::tool_adapter::RestrictedEgressResponse;
use ironclaw_sendblue_extension::SendblueChannelAdapter;

#[tokio::test]
async fn sendblue_satisfies_channel_conformance() {
    run_channel_adapter_conformance(ChannelAdapterConformance {
        surfaces: ironclaw_sendblue_extension::sendblue_surfaces(),
        reply_transport: Some(ReplyTransport::Message),
        extension_id: "sendblue".into(), installation_id: "fixture".into(),
        message_inbound: Some(ConformanceInbound {
            body: br#"{"from_number":"+15555550101","to_number":"+15555550100","message_handle":"fixture-1","content":"hello","status":"RECEIVED","is_outbound":false}"#.to_vec(),
            headers: Vec::new(),
        }),
        challenge_inbound: None,
        outbound_envelope: OutboundEnvelope {
            target: OutboundTarget { conversation: ExternalConversationRef::new(Some("+15555550100"), "+15555550101", None, None).unwrap(), thread_anchor: None },
            parts: vec![OutboundPart::Text("hello back".into())], reply_context: None,
            registrations: Vec::new(), visibility: OutboundVisibility::Public,
        },
        vendor_responses: Arc::new(|_| RestrictedEgressResponse {
            status: 200, body: br#"{"message_handle":"fixture-out","status":"SENT"}"#.to_vec(), retry_after: None,
        }),
        config: vec![("sendblue_from_number".into(), "+15555550100".into()), ("sendblue_allow_from".into(), "+15555550101".into())],
    }).await;
}

fn outbound(text: &str) -> OutboundEnvelope {
    OutboundEnvelope {
        target: OutboundTarget {
            conversation: ExternalConversationRef::new(
                Some("+15555550100"),
                "+15555550101",
                None,
                None,
            )
            .unwrap(),
            thread_anchor: None,
        },
        parts: vec![OutboundPart::Text(text.into())],
        reply_context: None,
        registrations: Vec::new(),
        visibility: OutboundVisibility::Public,
    }
}

#[tokio::test]
async fn text_chunks_preserve_unicode_and_no_raw_credentials_reach_egress() {
    use ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer;
    let server = ScriptedVendorServer::new(Arc::new(|_| RestrictedEgressResponse {
        status: 200,
        body: br#"{"status":"SENT","message_handle":"out-1"}"#.to_vec(),
        retry_after: None,
    }));
    let text = format!("{}🦀tail", "a".repeat(1999));
    let report = SendblueChannelAdapter
        .deliver(outbound(&text), &server)
        .await
        .unwrap();
    assert_eq!(report.parts.len(), 2);
    let calls = server.requests();
    let bodies: Vec<serde_json::Value> = calls
        .iter()
        .map(|r| serde_json::from_slice(r.body.as_ref().unwrap()).unwrap())
        .collect();
    assert_eq!(
        format!(
            "{}{}",
            bodies[0]["content"].as_str().unwrap(),
            bodies[1]["content"].as_str().unwrap()
        ),
        text
    );
    for request in calls {
        assert_eq!(request.url, "https://api.sendblue.com/api/send-message");
        assert_eq!(request.credential.unwrap().as_str(), "sendblue_api_secret");
        assert_eq!(
            request.headers,
            vec![("content-type".into(), "application/json".into())]
        );
    }
}

#[tokio::test]
async fn rejection_and_ambiguous_acceptance_stop_before_later_chunks() {
    use ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer;
    for (body, ambiguous) in [
        (r#"{"status":"ERROR","error_code":400}"#, false),
        (r#"{"status":"QUEUED"}"#, true),
        ("not json", true),
    ] {
        let body = body.as_bytes().to_vec();
        let server = ScriptedVendorServer::new(Arc::new(move |_| RestrictedEgressResponse {
            status: 200,
            body: body.clone(),
            retry_after: None,
        }));
        let report = SendblueChannelAdapter
            .deliver(outbound(&"x".repeat(4001)), &server)
            .await
            .unwrap();
        assert_eq!(server.requests().len(), 1);
        assert!(if ambiguous {
            matches!(report.parts[0], PartDeliveryOutcome::Ambiguous { .. })
        } else {
            matches!(report.parts[0], PartDeliveryOutcome::Permanent { .. })
        });
    }
}

#[tokio::test]
async fn inbound_admission_filters_line_sender_echo_receipt_and_group_and_never_fetches_media() {
    use ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer;
    let server = ScriptedVendorServer::new(Arc::new(|_| panic!("ingress must never fetch media")));
    let config = vec![
        ("sendblue_from_number".into(), "+15555550100".into()),
        ("sendblue_allow_from".into(), "+15555550101".into()),
    ];
    let base = serde_json::json!({"from_number":"+15555550101","to_number":"+15555550100","is_outbound":false,"status":"RECEIVED","message_handle":"fixture-1","content":"hello"});
    for (key, value) in [
        ("to_number", serde_json::json!("+15555550999")),
        ("from_number", serde_json::json!("+15555550999")),
        ("is_outbound", serde_json::json!(true)),
        ("status", serde_json::json!("DELIVERED")),
        ("group_id", serde_json::json!("group-1")),
    ] {
        let mut body = base.clone();
        body[key] = value;
        let bytes = serde_json::to_vec(&body).unwrap();
        let result = SendblueChannelAdapter
            .receive(
                VerifiedInbound {
                    extension_id: "sendblue",
                    installation_id: "fixture",
                    config: &config,
                    body: &bytes,
                    headers: &[],
                    can_reply_in_threads: false,
                },
                &server,
            )
            .await
            .unwrap();
        assert!(matches!(result, InboundOutcome::Ignore));
    }
    let mut body = base;
    body["media_url"] = serde_json::json!("http://169.254.169.254/secret");
    let bytes = serde_json::to_vec(&body).unwrap();
    let result = SendblueChannelAdapter
        .receive(
            VerifiedInbound {
                extension_id: "sendblue",
                installation_id: "fixture",
                config: &config,
                body: &bytes,
                headers: &[],
                can_reply_in_threads: false,
            },
            &server,
        )
        .await
        .unwrap();
    let InboundOutcome::Messages(messages) = result else {
        panic!("message");
    };
    assert!(messages[0].text.contains("supports text only"));
    assert!(messages[0].attachments.is_empty());
}

#[tokio::test]
async fn ambiguous_partial_reply_checkpoint_prevents_replay() {
    use ironclaw_extension_contracts::reply::*;
    use ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let server = ScriptedVendorServer::new(Arc::new(move |_| RestrictedEgressResponse {
        status: 200,
        body: if count.fetch_add(1, Ordering::SeqCst) == 0 {
            br#"{"status":"SENT","message_handle":"accepted-part"}"#.to_vec()
        } else {
            b"broken response".to_vec()
        },
        retry_after: None,
    }));
    let mut request = reply_request(&"x".repeat(4001));
    let report = SendblueChannelAdapter
        .reconcile(request.clone(), &server)
        .await
        .unwrap();
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    assert_eq!(report.evidence.provider_refs.len(), 1);
    request.checkpoint = report.checkpoint;
    let replay = SendblueChannelAdapter
        .reconcile(request, &server)
        .await
        .unwrap();
    assert!(matches!(replay.outcome, ReplySinkOutcome::Permanent { .. }));
    assert_eq!(replay.evidence.provider_refs.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

fn reply_request(text: &str) -> ironclaw_extension_contracts::reply::ReplyReconcileRequest {
    use ironclaw_extension_contracts::reply::*;
    use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
    use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope};
    let user = UserId::new("fixture-user").unwrap();
    let mut document = ReplyDocument::default();
    document.finalize_answer(ReplyAnswerText::new(text.to_string()).unwrap(), Vec::new());
    document.complete();
    ReplyReconcileRequest {
        revision: ReplyRevision {
            revision: 1,
            document,
        },
        point: ReplyReconcilePoint::Terminal,
        target: ReplyTarget {
            scope: TurnScope::new_with_owner(
                TenantId::new("fixture-tenant").unwrap(),
                None,
                None,
                ThreadId::new("fixture-thread").unwrap(),
                Some(user.clone()),
            ),
            actor: TurnActor::new(user),
            run_id: TurnRunId::new(),
            conversation: Some(outbound("").target.conversation),
            thread_anchor: None,
            audience: ReplyAudience::Private,
        },
        reply_context: None,
        checkpoint: None,
        extension_generation: 1,
        materialized_attachments: Vec::new(),
    }
}

#[tokio::test]
async fn queued_reply_is_uncertain_and_checkpoint_prevents_replay() {
    use ironclaw_extension_contracts::reply::*;
    use ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer;
    let server = ScriptedVendorServer::new(Arc::new(|_| RestrictedEgressResponse {
        status: 200,
        body: br#"{"status":"QUEUED","message_handle":"accepted"}"#.to_vec(),
        retry_after: None,
    }));
    let mut request = reply_request(&"x".repeat(4001));
    let report = SendblueChannelAdapter
        .reconcile(request.clone(), &server)
        .await
        .unwrap();
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));
    assert!(report.evidence.provider_refs.is_empty());
    assert_eq!(server.requests().len(), 1);
    request.checkpoint = report.checkpoint;
    let replay = SendblueChannelAdapter
        .reconcile(request, &server)
        .await
        .unwrap();
    assert!(matches!(replay.outcome, ReplySinkOutcome::Permanent { .. }));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
#[tracing_test::traced_test]
async fn malformed_and_unsupported_checkpoints_fail_closed_with_distinct_reasons() {
    use ironclaw_extension_contracts::reply::*;
    use ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer;
    let server = ScriptedVendorServer::new(Arc::new(|_| {
        panic!("checkpoint must block provider access")
    }));
    for (version, payload, expected) in [
        (1, "{private-payload", "malformed"),
        (
            1,
            r#"{"applied":"private-payload","evidence":{}}"#,
            "malformed",
        ),
        (2, "{private-payload", "unsupported"),
    ] {
        let mut request = reply_request("hello");
        request.checkpoint = Some(ReplySinkCheckpoint::new(version, payload).unwrap());
        let report = SendblueChannelAdapter
            .reconcile(request, &server)
            .await
            .unwrap();
        let ReplySinkOutcome::Permanent { reason } = report.outcome else {
            panic!("must reject checkpoint")
        };
        assert!(reason.as_str().contains(expected));
        assert!(!reason.as_str().contains("private-payload"));
        assert_eq!(report.checkpoint.unwrap().payload(), payload);
        assert!(report.evidence.provider_refs.is_empty());
    }
    assert!(server.requests().is_empty());
    assert!(logs_contain("Malformed Sendblue reply checkpoint"));
    assert!(logs_contain(
        "Unsupported Sendblue reply checkpoint version"
    ));
    assert!(!logs_contain("private-payload"));
}
