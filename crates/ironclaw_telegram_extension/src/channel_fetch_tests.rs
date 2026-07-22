use std::collections::VecDeque;
use std::sync::Mutex;

use ironclaw_host_api::{RestrictedEgressError, RestrictedEgressResponse};
use ironclaw_product_adapters::{
    AttachmentRef, ProductAttachmentDescriptor, ProductAttachmentKind,
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

fn attachment(size_bytes: Option<u64>) -> AttachmentRef {
    AttachmentRef {
        descriptor: ProductAttachmentDescriptor::new(
            "descriptor-file-id",
            "text/plain",
            Some("original.txt".to_string()),
            size_bytes,
            ProductAttachmentKind::Document,
        )
        .expect("descriptor"),
        vendor_ref: "vendor-file-id".to_string(),
        mime_hint: Some("application/octet-stream".to_string()),
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

    let fetched = TelegramChannelAdapter::default()
        .fetch_attachment(&attachment(Some(5)), &egress)
        .await
        .expect("attachment fetch succeeds");

    assert_eq!(fetched.id, "descriptor-file-id");
    assert_eq!(fetched.mime_type, "text/plain");
    assert_eq!(fetched.filename.as_deref(), Some("original.txt"));
    assert_eq!(fetched.bytes, b"hello");

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
async fn fetch_attachment_rejects_missing_and_malformed_provider_paths() {
    for body in [
        br#"{"ok":true,"result":{}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"../secret"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"/absolute"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"https://evil.test/x"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"documents/x?token=y"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"documents\\\\x"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"documents//x"}}"#.as_slice(),
        br#"{"ok":true,"result":{"file_path":"documents/x"}}"#.as_slice(),
    ] {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::response(200, body.to_vec())]);
        let error = TelegramChannelAdapter::default()
            .fetch_attachment(&attachment(None), &egress)
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
        let error = TelegramChannelAdapter::default()
            .fetch_attachment(&attachment(None), &egress)
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
    let error = TelegramChannelAdapter::default()
        .fetch_attachment(&attachment(Some(too_large)), &egress)
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
        TelegramChannelAdapter::default()
            .fetch_attachment(&attachment(None), &egress)
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
        TelegramChannelAdapter::default()
            .fetch_attachment(&attachment(None), &egress)
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
        TelegramChannelAdapter::default()
            .fetch_attachment(&attachment(None), &egress)
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
    let error = TelegramChannelAdapter::default()
        .fetch_attachment(&attachment(None), &egress)
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
