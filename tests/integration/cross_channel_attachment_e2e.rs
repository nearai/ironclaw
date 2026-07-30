//! Cross-channel attachment parity over the real Slack and Telegram adapters.
//!
//! The production runtime test covers model -> write_file -> explicit attach ->
//! finalized durable ref. These rows cover the provider side of that seam:
//! equivalent inbound files normalize to the same contract, and the same
//! transient workspace file is transferred by each supported upload protocol.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::product_adapter::{
    ChannelAdapter, ChannelAttachmentRef, DeliveryReport, ExternalConversationRef,
    OutboundEnvelope, OutboundPart, OutboundTarget, PartDeliveryOutcome,
    ProductAttachmentDescriptor, ProductAttachmentKind,
};
use ironclaw_host_api::{
    NetworkMethod, RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest,
    RestrictedEgressResponse, ScopedPath, WorkspaceFile,
};
use ironclaw_slack_extension::SlackChannelAdapter;
use ironclaw_telegram_extension::TelegramChannelAdapter;

struct ScriptedEgress {
    requests: Mutex<Vec<RestrictedEgressRequest>>,
    responses: Mutex<VecDeque<Result<RestrictedEgressResponse, RestrictedEgressError>>>,
}

impl ScriptedEgress {
    fn new(responses: Vec<Result<RestrictedEgressResponse, RestrictedEgressError>>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into()),
        }
    }

    fn json(body: &str) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Ok(RestrictedEgressResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        })
    }

    fn bytes(bytes: &[u8]) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Ok(RestrictedEgressResponse {
            status: 200,
            body: bytes.to_vec(),
        })
    }

    fn requests(&self) -> Vec<RestrictedEgressRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

#[async_trait]
impl RestrictedEgress for ScriptedEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        self.requests.lock().expect("requests").push(request);
        self.responses
            .lock()
            .expect("responses")
            .pop_front()
            .unwrap_or(Err(RestrictedEgressError::PolicyDenied))
    }
}

fn descriptor(external_file_id: &str) -> ProductAttachmentDescriptor {
    ProductAttachmentDescriptor::new(
        external_file_id,
        "text/plain",
        Some("report.txt".to_string()),
        Some(5),
        ProductAttachmentKind::Document,
    )
    .expect("descriptor")
}

fn workspace_file() -> WorkspaceFile {
    WorkspaceFile {
        path: ScopedPath::new("/workspace/report.txt").expect("path"),
        filename: Some("report.txt".to_string()),
        mime_type: "text/plain".to_string(),
        bytes: b"hello".to_vec(),
    }
}

fn envelope(file: WorkspaceFile) -> OutboundEnvelope {
    OutboundEnvelope {
        extension_id: "test".to_string(),
        installation_id: "install-alpha".to_string(),
        delivery_attempt_id: "attempt-one".to_string(),
        target: OutboundTarget {
            conversation: ExternalConversationRef::new(Some("T-A"), "D123", None, None)
                .expect("conversation"),
            thread_anchor: None,
        },
        parts: vec![
            OutboundPart::Text("The report is attached.".to_string()),
            OutboundPart::File(file),
        ],
        reply_context: None,
    }
}

fn prose_only_envelope() -> OutboundEnvelope {
    OutboundEnvelope {
        extension_id: "test".to_string(),
        installation_id: "install-alpha".to_string(),
        delivery_attempt_id: "attempt-prose".to_string(),
        target: OutboundTarget {
            conversation: ExternalConversationRef::new(Some("T-A"), "D123", None, None)
                .expect("conversation"),
            thread_anchor: None,
        },
        parts: vec![OutboundPart::Text(
            "The report is at /workspace/report.txt.".to_string(),
        )],
        reply_context: None,
    }
}

fn assert_text_and_file_sent(report: &DeliveryReport) {
    assert_eq!(report.parts.len(), 2);
    assert!(
        report
            .parts
            .iter()
            .all(|part| matches!(part, PartDeliveryOutcome::Sent { .. }))
    );
}

#[tokio::test]
async fn real_channel_adapters_normalize_inbound_files_to_the_same_contract() {
    let slack_egress = ScriptedEgress::new(vec![
        ScriptedEgress::json(
            r#"{"ok":true,"file":{"id":"F123","name":"report.txt","mimetype":"text/plain","size":5,"url_private_download":"https://files.slack.com/files-pri/T-F/download/report.txt"}}"#,
        ),
        ScriptedEgress::bytes(b"hello"),
    ]);
    let telegram_egress = ScriptedEgress::new(vec![
        ScriptedEgress::json(
            r#"{"ok":true,"result":{"file_size":5,"file_path":"documents/report.txt"}}"#,
        ),
        ScriptedEgress::bytes(b"hello"),
    ]);

    let slack = SlackChannelAdapter
        .fetch_attachment(
            &ChannelAttachmentRef {
                descriptor: descriptor("F123"),
                vendor_ref: "F123".to_string(),
            },
            &slack_egress,
        )
        .await
        .expect("slack fetch");
    let telegram = TelegramChannelAdapter::default()
        .fetch_attachment(
            &ChannelAttachmentRef {
                descriptor: descriptor("telegram-file"),
                vendor_ref: "telegram-file".to_string(),
            },
            &telegram_egress,
        )
        .await
        .expect("telegram fetch");

    assert_eq!(slack.mime_type, telegram.mime_type);
    assert_eq!(slack.filename, telegram.filename);
    assert_eq!(slack.bytes, telegram.bytes);
    assert_eq!(slack_egress.requests().len(), 2);
    assert_eq!(telegram_egress.requests().len(), 2);
}

#[tokio::test]
async fn real_channel_adapters_transfer_the_same_workspace_file_without_prose_scanning() {
    let slack_egress = ScriptedEgress::new(vec![
        ScriptedEgress::json(r#"{"ok":true,"ts":"1710000001.000001"}"#),
        ScriptedEgress::json(
            r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/ticket","file_id":"FNEW"}"#,
        ),
        ScriptedEgress::bytes(b"OK - 5"),
        ScriptedEgress::json(r#"{"ok":true,"files":[{"id":"FNEW"}]}"#),
        ScriptedEgress::json(
            r#"{"ok":true,"file":{"id":"FNEW","name":"report.txt","size":5,"ims":["D123"]}}"#,
        ),
    ]);
    let telegram_egress = ScriptedEgress::new(vec![
        ScriptedEgress::json(r#"{"ok":true,"result":{"message_id":41}}"#),
        ScriptedEgress::json(r#"{"ok":true,"result":{"message_id":42}}"#),
    ]);

    let slack_report = SlackChannelAdapter
        .deliver(envelope(workspace_file()), &slack_egress)
        .await
        .expect("slack delivery");
    let telegram_report = TelegramChannelAdapter::default()
        .deliver(envelope(workspace_file()), &telegram_egress)
        .await
        .expect("telegram delivery");

    assert_text_and_file_sent(&slack_report);
    assert_text_and_file_sent(&telegram_report);
    let slack_requests = slack_egress.requests();
    assert_eq!(
        slack_requests
            .iter()
            .map(|request| request.url.as_str())
            .collect::<Vec<_>>(),
        vec![
            "https://slack.com/api/chat.postMessage",
            "https://slack.com/api/files.getUploadURLExternal?filename=report.txt&length=5",
            "https://files.slack.com/upload/v1/ticket",
            "https://slack.com/api/files.completeUploadExternal",
            "https://slack.com/api/files.info?file=FNEW",
        ]
    );
    assert_eq!(slack_requests[2].body.as_deref(), Some(b"hello".as_slice()));

    let telegram_requests = telegram_egress.requests();
    assert_eq!(telegram_requests.len(), 2);
    assert!(
        telegram_requests[1].url.ends_with("/sendDocument"),
        "Telegram must use sendDocument"
    );
    assert_eq!(telegram_requests[1].method, NetworkMethod::Post);
    assert!(
        telegram_requests[1]
            .body
            .as_deref()
            .is_some_and(|body| body.windows(5).any(|window| window == b"hello"))
    );
}

#[tokio::test]
async fn real_channel_adapters_never_infer_files_from_workspace_paths_in_prose() {
    let slack_egress = ScriptedEgress::new(vec![ScriptedEgress::json(
        r#"{"ok":true,"ts":"1710000001.000001"}"#,
    )]);
    let telegram_egress = ScriptedEgress::new(vec![ScriptedEgress::json(
        r#"{"ok":true,"result":{"message_id":41}}"#,
    )]);

    SlackChannelAdapter
        .deliver(prose_only_envelope(), &slack_egress)
        .await
        .expect("slack text delivery");
    TelegramChannelAdapter::default()
        .deliver(prose_only_envelope(), &telegram_egress)
        .await
        .expect("telegram text delivery");

    assert_eq!(slack_egress.requests().len(), 1);
    assert!(
        slack_egress.requests()[0]
            .url
            .ends_with("/api/chat.postMessage")
    );
    assert_eq!(telegram_egress.requests().len(), 1);
    assert!(telegram_egress.requests()[0].url.ends_with("/sendMessage"));
}
