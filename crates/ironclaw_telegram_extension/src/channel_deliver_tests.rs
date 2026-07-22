use std::collections::VecDeque;
use std::sync::Mutex;

use ironclaw_attachments::WorkspaceFile;
use ironclaw_host_api::{RestrictedEgressError, RestrictedEgressResponse};
use ironclaw_product_adapters::{
    ExternalConversationRef, OutboundEnvelope, OutboundPart, OutboundTarget, PartDeliveryOutcome,
};

use super::*;

struct ScriptedEgress {
    requests: Mutex<Vec<RestrictedEgressRequest>>,
    responses: Mutex<VecDeque<Result<RestrictedEgressResponse, RestrictedEgressError>>>,
}

impl ScriptedEgress {
    fn new(responses: Vec<Result<RestrictedEgressResponse, RestrictedEgressError>>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }

    fn ok(body: &str) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Ok(RestrictedEgressResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        })
    }
}

#[async_trait]
impl RestrictedEgress for ScriptedEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(RestrictedEgressError::PolicyDenied))
    }
}

fn envelope(parts: Vec<OutboundPart>, topic: Option<&str>) -> OutboundEnvelope {
    OutboundEnvelope {
        extension_id: "telegram".to_string(),
        installation_id: "install_alpha".to_string(),
        delivery_attempt_id: "attempt-1".to_string(),
        target: OutboundTarget {
            conversation: ExternalConversationRef::new(None, "8675309", topic, None)
                .expect("conversation"),
            thread_anchor: None,
        },
        parts,
        reply_context: None,
    }
}

fn envelope_with_reply(
    parts: Vec<OutboundPart>,
    topic: Option<&str>,
    reply_to: Option<&str>,
) -> OutboundEnvelope {
    let mut envelope = envelope(parts, topic);
    envelope.target.conversation =
        ExternalConversationRef::new(None, "8675309", topic, reply_to).expect("conversation");
    envelope
}

fn workspace_file(filename: Option<&str>, mime_type: &str, bytes: &[u8]) -> WorkspaceFile {
    WorkspaceFile {
        path: ironclaw_host_api::ScopedPath::new("/workspace/report.pdf").expect("workspace path"),
        filename: filename.map(str::to_string),
        mime_type: mime_type.to_string(),
        bytes: bytes.to_vec(),
    }
}

#[tokio::test]
async fn deliver_retract_part_deletes_the_referenced_message() {
    let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(r#"{"ok":true,"result":true}"#)]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(
                vec![OutboundPart::Retract {
                    vendor_message_ref: "42".to_string(),
                }],
                None,
            ),
            &egress,
        )
        .await
        .expect("deliver drives");

    assert_eq!(report.parts.len(), 1);
    assert!(matches!(
        &report.parts[0],
        PartDeliveryOutcome::Sent {
            vendor_message_ref: None
        }
    ));
    let requests = egress.requests.lock().unwrap();
    assert_eq!(
        requests[0].url,
        "https://api.telegram.org/bot{telegram_bot_token}/deleteMessage"
    );
    let body: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_deref().unwrap_or_default()).unwrap();
    assert_eq!(body["chat_id"], "8675309");
    assert_eq!(body["message_id"], 42);
}

#[tokio::test]
async fn deliver_retract_requires_true_result_evidence() {
    for body in [r#"{"ok":true,"result":false}"#, r#"{"ok":true}"#] {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(body)]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(
                    vec![OutboundPart::Retract {
                        vendor_message_ref: "42".to_string(),
                    }],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert!(
            !matches!(&report.parts[0], PartDeliveryOutcome::Sent { .. }),
            "deleteMessage must not report Sent without result:true: {body}"
        );
    }
}

#[tokio::test]
async fn deliver_retract_with_non_numeric_ref_is_permanent_without_egress() {
    let egress = ScriptedEgress::new(Vec::new());
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(
                vec![OutboundPart::Retract {
                    vendor_message_ref: "not-a-message-id".to_string(),
                }],
                None,
            ),
            &egress,
        )
        .await
        .expect("deliver drives");
    assert!(matches!(
        &report.parts[0],
        PartDeliveryOutcome::Permanent { .. }
    ));
    assert!(
        egress.requests.lock().unwrap().is_empty(),
        "an unparseable retract target must not reach the vendor"
    );
}

#[tokio::test]
async fn deliver_posts_send_message_through_the_token_path_template() {
    let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
        r#"{"ok":true,"result":{"message_id":42}}"#,
    )]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(vec![OutboundPart::Text("hello".to_string())], Some("77")),
            &egress,
        )
        .await
        .expect("deliver drives");

    assert_eq!(report.parts.len(), 1);
    assert!(matches!(
        &report.parts[0],
        PartDeliveryOutcome::Sent { vendor_message_ref: Some(id) } if id == "42"
    ));
    let requests = egress.requests.lock().unwrap();
    assert_eq!(
        requests[0].url, "https://api.telegram.org/bot{telegram_bot_token}/sendMessage",
        "the token rides the declared path placeholder, substituted host-side"
    );
    assert_eq!(
        requests[0].credential.as_ref().map(SecretHandle::as_str),
        Some(TELEGRAM_BOT_TOKEN_HANDLE)
    );
    let body: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_deref().unwrap_or_default()).unwrap();
    assert_eq!(body["chat_id"], "8675309");
    assert_eq!(body["text"], "hello");
    assert_eq!(body["message_thread_id"], 77, "numeric topic threads");
}

#[tokio::test]
async fn deliver_send_requires_message_id_evidence() {
    let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(r#"{"ok":true}"#)]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(vec![OutboundPart::Text("hello".to_string())], None),
            &egress,
        )
        .await
        .expect("deliver drives");

    assert!(
        !matches!(&report.parts[0], PartDeliveryOutcome::Sent { .. }),
        "sendMessage must not report Sent without result.message_id"
    );
}

#[tokio::test]
async fn deliver_splits_oversized_text_at_the_vendor_limit() {
    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::ok(r#"{"ok":true,"result":{"message_id":1}}"#),
        ScriptedEgress::ok(r#"{"ok":true,"result":{"message_id":2}}"#),
    ]);
    let long_text = "line\n".repeat(1_000); // 5000 chars > 4096
    let report = TelegramChannelAdapter::default()
        .deliver(envelope(vec![OutboundPart::Text(long_text)], None), &egress)
        .await
        .expect("deliver drives");
    assert_eq!(report.parts.len(), 2);
    assert!(
        report
            .parts
            .iter()
            .all(|part| matches!(part, PartDeliveryOutcome::Sent { .. }))
    );
    assert_eq!(egress.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn deliver_classifies_vendor_and_egress_failures_and_stops() {
    // 429 body → Retryable; the second text part is never attempted.
    let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
        r#"{"ok":false,"error_code":429,"description":"Too Many Requests"}"#,
    )]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(
                vec![
                    OutboundPart::Text("one".to_string()),
                    OutboundPart::Text("two".to_string()),
                ],
                None,
            ),
            &egress,
        )
        .await
        .expect("deliver drives");
    assert_eq!(report.parts.len(), 1);
    assert!(matches!(
        &report.parts[0],
        PartDeliveryOutcome::Retryable { .. }
    ));
    assert_eq!(egress.requests.lock().unwrap().len(), 1);

    // 403 → Unauthorized (bot kicked / token revoked).
    let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
        r#"{"ok":false,"error_code":403,"description":"Forbidden"}"#,
    )]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(vec![OutboundPart::Text("x".to_string())], None),
            &egress,
        )
        .await
        .expect("deliver drives");
    assert!(matches!(
        &report.parts[0],
        PartDeliveryOutcome::Unauthorized { .. }
    ));

    // Missing credential material → Unauthorized without vendor traffic
    // beyond the failed attempt.
    let egress = ScriptedEgress::new(vec![Err(RestrictedEgressError::AuthRequired {
        required_secrets: Vec::new(),
        credential_requirements: Vec::new(),
    })]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(vec![OutboundPart::Text("x".to_string())], None),
            &egress,
        )
        .await
        .expect("deliver drives");
    assert!(matches!(
        &report.parts[0],
        PartDeliveryOutcome::Unauthorized { .. }
    ));
}

#[tokio::test]
async fn deliver_rejects_empty_envelopes() {
    let egress = ScriptedEgress::new(Vec::new());
    let error = TelegramChannelAdapter::default()
        .deliver(envelope(Vec::new(), None), &egress)
        .await
        .expect_err("empty envelope is a render error");
    assert!(matches!(error, ChannelError::Render { .. }));
    assert!(egress.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn deliver_preserves_text_file_order_target_thread_and_reply_context() {
    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::ok(r#"{"ok":true,"result":{"message_id":41}}"#),
        ScriptedEgress::ok(r#"{"ok":true,"result":{"message_id":42}}"#),
    ]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope_with_reply(
                vec![
                    OutboundPart::Text("before".to_string()),
                    OutboundPart::File(workspace_file(
                        Some("report.pdf"),
                        "application/pdf",
                        b"pdf bytes",
                    )),
                ],
                Some("77"),
                Some("66"),
            ),
            &egress,
        )
        .await
        .expect("ordered delivery succeeds");

    assert_eq!(report.parts.len(), 2);
    assert!(matches!(
        &report.parts[0],
        PartDeliveryOutcome::Sent { vendor_message_ref: Some(id) } if id == "41"
    ));
    assert!(matches!(
        &report.parts[1],
        PartDeliveryOutcome::Sent { vendor_message_ref: Some(id) } if id == "42"
    ));

    let requests = egress.requests.lock().unwrap();
    assert!(requests[0].url.ends_with("/sendMessage"));
    assert!(requests[1].url.ends_with("/sendDocument"));
    let text: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(text["chat_id"], "8675309");
    assert_eq!(text["message_thread_id"], 77);
    assert_eq!(text["reply_to_message_id"], 66);
    let multipart = String::from_utf8_lossy(requests[1].body.as_deref().unwrap());
    assert!(multipart.contains("name=\"chat_id\"\r\n\r\n8675309"));
    assert!(multipart.contains("name=\"message_thread_id\"\r\n\r\n77"));
    assert!(multipart.contains("name=\"reply_to_message_id\"\r\n\r\n66"));
}

#[tokio::test]
async fn deliver_file_only_uses_native_send_document_multipart() {
    let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
        r#"{"ok":true,"result":{"message_id":42}}"#,
    )]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(
                vec![OutboundPart::File(workspace_file(
                    Some("report.pdf"),
                    "application/pdf",
                    b"pdf bytes",
                ))],
                None,
            ),
            &egress,
        )
        .await
        .expect("file-only delivery succeeds");
    assert!(matches!(
        report.parts.as_slice(),
        [PartDeliveryOutcome::Sent { .. }]
    ));

    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, NetworkMethod::Post);
    assert_eq!(
        request.url,
        "https://api.telegram.org/bot{telegram_bot_token}/sendDocument"
    );
    let content_type = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
        .expect("multipart content type");
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    let body = request.body.as_deref().expect("multipart body");
    assert!(
        body.windows(b"pdf bytes".len())
            .any(|window| window == b"pdf bytes")
    );
    let rendered = String::from_utf8_lossy(body);
    assert!(rendered.contains("name=\"document\"; filename=\"report.pdf\""));
    assert!(rendered.contains("Content-Type: application/pdf"));
    assert!(
        rendered.ends_with("--\r\n"),
        "multipart must have a closing boundary"
    );
}

#[tokio::test]
async fn deliver_sanitizes_multipart_filename_and_mime_headers() {
    let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
        r#"{"ok":true,"result":{"message_id":42}}"#,
    )]);
    TelegramChannelAdapter::default()
        .deliver(
            envelope(
                vec![OutboundPart::File(workspace_file(
                    Some("bad\"\r\nX-Evil: yes/../../report.pdf"),
                    "text/plain\r\nX-Evil: yes",
                    b"safe bytes",
                ))],
                None,
            ),
            &egress,
        )
        .await
        .expect("unsafe metadata is sanitized, not interpolated");

    let requests = egress.requests.lock().unwrap();
    let body = String::from_utf8_lossy(requests[0].body.as_deref().unwrap());
    assert!(!body.contains("X-Evil:"));
    assert!(!body.contains("../"));
    assert!(body.contains("Content-Type: application/octet-stream"));
}

#[tokio::test]
async fn deliver_classifies_send_document_failures_and_partial_delivery() {
    for (response, expected) in [
        (
            ScriptedEgress::ok(r#"{"ok":false,"error_code":401,"description":"bad token"}"#),
            "unauthorized",
        ),
        (
            ScriptedEgress::ok(r#"{"ok":false,"error_code":429,"description":"slow"}"#),
            "retryable",
        ),
        (
            ScriptedEgress::ok(r#"{"ok":false,"error_code":400,"description":"bad file"}"#),
            "permanent",
        ),
    ] {
        let egress = ScriptedEgress::new(vec![response]);
        let report = TelegramChannelAdapter::default()
            .deliver(
                envelope(
                    vec![OutboundPart::File(workspace_file(
                        Some("report.pdf"),
                        "application/pdf",
                        b"pdf",
                    ))],
                    None,
                ),
                &egress,
            )
            .await
            .expect("delivery reports vendor failure");
        let matches_expected = matches!(
            (expected, &report.parts[0]),
            ("unauthorized", PartDeliveryOutcome::Unauthorized { .. })
                | ("retryable", PartDeliveryOutcome::Retryable { .. })
                | ("permanent", PartDeliveryOutcome::Permanent { .. })
        );
        assert!(
            matches_expected,
            "expected {expected}: {:?}",
            report.parts[0]
        );
    }

    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::ok(r#"{"ok":true,"result":{"message_id":41}}"#),
        ScriptedEgress::ok(r#"{"ok":false,"error_code":500,"description":"down"}"#),
    ]);
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(
                vec![
                    OutboundPart::Text("sent first".to_string()),
                    OutboundPart::File(workspace_file(
                        Some("report.pdf"),
                        "application/pdf",
                        b"pdf",
                    )),
                    OutboundPart::Text("must not send".to_string()),
                ],
                None,
            ),
            &egress,
        )
        .await
        .expect("partial delivery is reported");
    assert!(matches!(report.parts[0], PartDeliveryOutcome::Sent { .. }));
    assert!(matches!(
        report.parts[1],
        PartDeliveryOutcome::Retryable { .. }
    ));
    assert_eq!(egress.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn deliver_rejects_oversized_workspace_file_before_egress() {
    let bytes = vec![0; ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1];
    let egress = ScriptedEgress::new(Vec::new());
    let report = TelegramChannelAdapter::default()
        .deliver(
            envelope(
                vec![OutboundPart::File(workspace_file(
                    Some("huge.bin"),
                    "application/octet-stream",
                    &bytes,
                ))],
                None,
            ),
            &egress,
        )
        .await
        .expect("oversize is a per-part permanent outcome");
    assert!(matches!(
        report.parts[0],
        PartDeliveryOutcome::Permanent { .. }
    ));
    assert!(egress.requests.lock().unwrap().is_empty());
}
