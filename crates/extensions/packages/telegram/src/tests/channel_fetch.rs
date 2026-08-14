use std::collections::VecDeque;
use std::sync::Mutex;

use ironclaw_extension_contracts::channel_adapter::ChannelAttachmentRef;
use ironclaw_extension_contracts::external::{ProductAttachmentDescriptor, ProductAttachmentKind};
use ironclaw_extension_contracts::tool_adapter::{RestrictedEgressError, RestrictedEgressResponse};

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

    fn response(
        status: u16,
        body: impl Into<Vec<u8>>,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Ok(RestrictedEgressResponse {
            status,
            body: body.into(),
        })
    }
}

#[async_trait]
impl RestrictedEgress for ScriptedEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        self.requests.lock().expect("requests lock").push(request);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .unwrap_or(Err(RestrictedEgressError::PolicyDenied))
    }
}

fn attachment(size_bytes: Option<u64>) -> ChannelAttachmentRef {
    ChannelAttachmentRef {
        descriptor: ProductAttachmentDescriptor::new(
            "descriptor-file-id",
            "text/plain",
            Some("original.txt".to_string()),
            size_bytes,
            ProductAttachmentKind::Document,
        )
        .expect("descriptor"),
        vendor_ref: "vendor-file-id".to_string(),
    }
}

#[tokio::test]
async fn fetch_attachment_looks_up_then_downloads_through_restricted_egress() {
    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::response(
            200,
            br#"{"ok":true,"result":{"file_size":5,"file_path":"documents/provider.txt"}}"#
                .to_vec(),
        ),
        ScriptedEgress::response(200, b"hello".to_vec()),
    ]);

    let config = [(
        TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
        "ironclaw_test_bot".to_string(),
    )];
    let outcome = TelegramChannelAdapter::default()
        .receive(
            VerifiedInbound {
                extension_id: "telegram",
                installation_id: "install_alpha",
                config: &config,
                body: br#"{
                    "update_id": 500,
                    "message": {
                        "message_id": 50,
                        "date": 1710000000,
                        "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
                        "chat": {"id": 555, "type": "private"},
                        "document": {
                            "file_id": "vendor-file-id",
                            "file_name": "original.txt",
                            "mime_type": "text/plain",
                            "file_size": 5
                        }
                    }
                }"#,
                headers: &[],
                can_reply_in_threads: false,
            },
            &egress,
        )
        .await
        .expect("attachment fetch succeeds during receive");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("expected complete message");
    };
    let fetched = &messages[0].attachments[0];

    assert_eq!(fetched.id, "vendor-file-id");
    assert_eq!(fetched.mime_type, "text/plain");
    assert_eq!(fetched.filename.as_deref(), Some("original.txt"));
    assert_eq!(fetched.bytes, b"hello");
    assert!(messages[0].conversation_context.is_none());

    let requests = egress.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert_eq!(
        requests[0].url,
        "https://api.telegram.org/bot{telegram_bot_token}/getFile"
    );
    let lookup_body: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_deref().expect("lookup body"))
            .expect("lookup json");
    assert_eq!(lookup_body["file_id"], "vendor-file-id");
    assert_eq!(requests[1].method, NetworkMethod::Get);
    assert_eq!(
        requests[1].url,
        "https://api.telegram.org/file/bot{telegram_bot_token}/documents/provider.txt"
    );
    assert_eq!(
        requests[1].credential.as_ref().map(SecretHandle::as_str),
        Some(TELEGRAM_BOT_TOKEN_HANDLE)
    );
}

#[tokio::test]
async fn receive_degrades_permanently_failed_attachments_and_keeps_the_message() {
    // Regression: a permanently failing transfer used to fail the whole
    // update, so ingress answered non-2xx and Telegram's in-order redelivery
    // wedged the chat behind a payload that could never improve.
    let egress = ScriptedEgress::new(vec![ScriptedEgress::response(403, Vec::new())]);
    let config = [(
        TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
        "ironclaw_test_bot".to_string(),
    )];
    let outcome = TelegramChannelAdapter::default()
        .receive(
            VerifiedInbound {
                extension_id: "telegram",
                installation_id: "install_alpha",
                config: &config,
                body: br#"{
                    "update_id": 510,
                    "message": {
                        "message_id": 51,
                        "date": 1710000000,
                        "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
                        "chat": {"id": 555, "type": "private"},
                        "caption": "look at this",
                        "document": {
                            "file_id": "vendor-file-id",
                            "file_name": "original.txt",
                            "mime_type": "text/plain",
                            "file_size": 5
                        }
                    }
                }"#,
                headers: &[],
                can_reply_in_threads: false,
            },
            &egress,
        )
        .await
        .expect("permanent transfer failure degrades instead of failing the update");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("expected degraded message");
    };
    assert_eq!(messages[0].text, "look at this");
    assert!(messages[0].attachments.is_empty());
}

#[tokio::test]
async fn receive_ignores_updates_degraded_to_nothing() {
    // A sticker-only update whose transfer fails permanently leaves neither
    // text nor attachments: acknowledge and ignore rather than start an empty
    // turn (and never hand the vendor a redeliverable failure status).
    let egress = ScriptedEgress::new(vec![ScriptedEgress::response(403, Vec::new())]);
    let config = [(
        TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
        "ironclaw_test_bot".to_string(),
    )];
    let outcome = TelegramChannelAdapter::default()
        .receive(
            VerifiedInbound {
                extension_id: "telegram",
                installation_id: "install_alpha",
                config: &config,
                body: br#"{
                    "update_id": 511,
                    "message": {
                        "message_id": 52,
                        "date": 1710000000,
                        "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
                        "chat": {"id": 555, "type": "private"},
                        "sticker": {"file_id": "st-1", "file_size": 4096}
                    }
                }"#,
                headers: &[],
                can_reply_in_threads: false,
            },
            &egress,
        )
        .await
        .expect("fully degraded update is acknowledged");
    assert!(matches!(outcome, InboundOutcome::Ignore), "expected Ignore");
}

#[tokio::test]
async fn receive_propagates_retryable_attachment_failures() {
    // Transient transfer failures must keep failing the request so ingress
    // answers 503 and vendor redelivery can succeed later with full content.
    let egress = ScriptedEgress::new(vec![ScriptedEgress::response(500, Vec::new())]);
    let config = [(
        TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
        "ironclaw_test_bot".to_string(),
    )];
    let result = TelegramChannelAdapter::default()
        .receive(
            VerifiedInbound {
                extension_id: "telegram",
                installation_id: "install_alpha",
                config: &config,
                body: br#"{
                    "update_id": 512,
                    "message": {
                        "message_id": 53,
                        "date": 1710000000,
                        "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
                        "chat": {"id": 555, "type": "private"},
                        "caption": "look at this",
                        "document": {
                            "file_id": "vendor-file-id",
                            "file_name": "original.txt",
                            "mime_type": "text/plain",
                            "file_size": 5
                        }
                    }
                }"#,
                headers: &[],
                can_reply_in_threads: false,
            },
            &egress,
        )
        .await;
    let Err(error) = result else {
        panic!("retryable transfer failure must propagate");
    };
    assert!(matches!(
        error,
        ChannelError::AttachmentTransfer {
            retryable: true,
            ..
        }
    ));
}

#[tokio::test]
async fn fetch_attachment_downloads_when_optional_size_metadata_is_absent() {
    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::response(
            200,
            br#"{"ok":true,"result":{"file_path":"documents/provider.txt"}}"#.to_vec(),
        ),
        ScriptedEgress::response(200, b"hello".to_vec()),
    ]);

    let fetched = crate::attachment_transfer::fetch_attachment(&attachment(None), &egress)
        .await
        .expect("bounded download succeeds without optional size hints");

    assert_eq!(fetched.bytes, b"hello");
    assert_eq!(egress.requests.lock().expect("requests lock").len(), 2);
}

#[tokio::test]
async fn fetch_attachment_rejects_missing_and_malformed_provider_paths() {
    for body in [
        br#"{"ok":true,"result":{}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"../secret"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"/absolute"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"https://evil.test/x"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"documents/x?token=y"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"documents\\\\x"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"documents//x"}}"#.as_slice(),
    ] {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::response(200, body.to_vec())]);
        let error = crate::attachment_transfer::fetch_attachment(&attachment(None), &egress)
            .await
            .expect_err("unsafe provider path must fail closed");
        assert!(matches!(
            error,
            ChannelError::AttachmentTransfer {
                retryable: false,
                ..
            }
        ));
        assert_eq!(egress.requests.lock().expect("requests lock").len(), 1);
    }
}

#[tokio::test]
async fn fetch_attachment_classifies_provider_and_restricted_egress_errors() {
    let cases = vec![
        (ScriptedEgress::response(500, Vec::new()), true),
        (ScriptedEgress::response(403, Vec::new()), false),
        (
            ScriptedEgress::response(
                200,
                br#"{"ok":false,"error_code":429,"description":"slow down"}"#.to_vec(),
            ),
            true,
        ),
        (
            Err(RestrictedEgressError::Transport {
                reason: "offline".to_string(),
            }),
            true,
        ),
        (Err(RestrictedEgressError::PolicyDenied), false),
    ];
    for (response, expected_retryable) in cases {
        let egress = ScriptedEgress::new(vec![response]);
        let error = crate::attachment_transfer::fetch_attachment(&attachment(None), &egress)
            .await
            .expect_err("provider failure must be classified");
        assert!(matches!(
            error,
            ChannelError::AttachmentTransfer { retryable, .. }
                if retryable == expected_retryable
        ));
    }
}

#[tokio::test]
async fn fetch_attachment_rejects_declared_provider_and_actual_oversize_or_truncation() {
    let too_large = ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 + 1;
    let egress = ScriptedEgress::new(Vec::new());
    let error = crate::attachment_transfer::fetch_attachment(&attachment(Some(too_large)), &egress)
        .await
        .expect_err("descriptor oversize is rejected before egress");
    assert!(matches!(
        error,
        ChannelError::AttachmentTransfer {
            retryable: false,
            ..
        }
    ));
    assert!(egress.requests.lock().expect("requests lock").is_empty());

    let egress = ScriptedEgress::new(vec![ScriptedEgress::response(
        200,
        format!(r#"{{"ok":true,"result":{{"file_size":{too_large},"file_path":"documents/x"}}}}"#)
            .into_bytes(),
    )]);
    assert!(
        crate::attachment_transfer::fetch_attachment(&attachment(None), &egress)
            .await
            .is_err()
    );
    assert_eq!(egress.requests.lock().expect("requests lock").len(), 1);

    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::response(
            200,
            br#"{"ok":true,"result":{"file_size":5,"file_path":"documents/x"}}"#.to_vec(),
        ),
        ScriptedEgress::response(200, b"four".to_vec()),
    ]);
    assert!(
        crate::attachment_transfer::fetch_attachment(&attachment(None), &egress)
            .await
            .is_err()
    );

    let actual_oversize = vec![0u8; too_large as usize];
    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::response(
            200,
            format!(
                r#"{{"ok":true,"result":{{"file_size":{},"file_path":"documents/x"}}}}"#,
                ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes
            )
            .into_bytes(),
        ),
        ScriptedEgress::response(200, actual_oversize),
    ]);
    assert!(
        crate::attachment_transfer::fetch_attachment(&attachment(None), &egress)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn fetch_attachment_treats_response_limit_overrun_as_permanent() {
    let egress = ScriptedEgress::new(vec![
        ScriptedEgress::response(
            200,
            br#"{"ok":true,"result":{"file_size":1,"file_path":"documents/x"}}"#.to_vec(),
        ),
        Err(RestrictedEgressError::ResponseTooLarge),
    ]);
    let error = crate::attachment_transfer::fetch_attachment(&attachment(None), &egress)
        .await
        .expect_err("host response cap must fail closed");
    assert!(matches!(
        error,
        ChannelError::AttachmentTransfer {
            retryable: false,
            ..
        }
    ));
}

/// The adapter's transfer bound and the manifest's declared egress limits are
/// one number in two files. A file between the two used to pass every code
/// check and then be refused by policy at the egress boundary — reported
/// inbound as "denied" rather than "too large", and outbound as a delivery
/// that never happened.
#[test]
fn manifest_transfer_bounds_match_the_adapter_constants() {
    const MANIFEST: &str = include_str!("../../manifest.toml");

    let download_cap = format!(
        "response_body_limit_bytes = {}",
        crate::attachment_transfer::TELEGRAM_MAX_TRANSFER_BYTES
    );
    assert!(
        MANIFEST.contains(&download_cap),
        "manifest download response cap must equal TELEGRAM_MAX_TRANSFER_BYTES ({download_cap})"
    );

    let upload_cap = format!(
        "request_body_limit_bytes = {}",
        crate::attachment_transfer::TELEGRAM_MAX_TRANSFER_BYTES
            + crate::attachment_transfer::TELEGRAM_MULTIPART_OVERHEAD_BYTES
    );
    assert!(
        MANIFEST.contains(&upload_cap),
        "manifest request cap must equal the file bound plus multipart overhead ({upload_cap})"
    );
}

/// The multipart boundary must not be derivable from the payload: the bytes
/// are attacker-authored (an inbound attachment can be landed and later
/// referenced by a reply), and a payload-derived candidate let a sender pad a
/// file with collisions to force unbounded full-payload rescans.
#[test]
fn multipart_boundary_is_unpredictable_and_absent_from_the_payload() {
    let payload = vec![b'a'; 4096];

    let first = crate::attachment_transfer::multipart_boundary(&payload).expect("boundary");
    let second = crate::attachment_transfer::multipart_boundary(&payload).expect("boundary");

    assert_ne!(
        first, second,
        "a payload-derived boundary is predictable to the sender"
    );
    for boundary in [&first, &second] {
        assert!(
            !payload
                .windows(boundary.len())
                .any(|window| window == boundary.as_bytes()),
            "boundary must not occur in the payload"
        );
        assert!(!boundary.contains(&payload.len().to_string()));
    }
}
