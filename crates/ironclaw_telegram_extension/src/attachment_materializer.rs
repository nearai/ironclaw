//! Telegram file transfer into the canonical inbound attachment lander.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::{DEFAULT_ATTACHMENT_BUDGETS, InboundAttachment};
use ironclaw_common::{is_supported_mime, normalize_mime_type};
use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressMethod, EgressPath, EgressRequest,
    ProductAttachmentDescriptor, ProductInboundEnvelope, ProtocolHttpEgress,
    ProtocolHttpEgressError,
};
use ironclaw_product_workflow::{AttachmentMaterializationError, InboundAttachmentMaterializer};
use serde::Deserialize;

use crate::telegram_actor_identity::TELEGRAM_V2_ADAPTER_ID;
use ironclaw_telegram_v2_adapter::TELEGRAM_API_HOST;

pub(crate) struct TelegramAttachmentMaterializer {
    egress: Arc<dyn ProtocolHttpEgress>,
    credential_handle: EgressCredentialHandle,
}

impl TelegramAttachmentMaterializer {
    pub(crate) fn new(
        egress: Arc<dyn ProtocolHttpEgress>,
        credential_handle: EgressCredentialHandle,
    ) -> Self {
        Self {
            egress,
            credential_handle,
        }
    }
}

#[async_trait]
impl InboundAttachmentMaterializer for TelegramAttachmentMaterializer {
    async fn materialize(
        &self,
        envelope: &ProductInboundEnvelope,
        descriptors: &[ProductAttachmentDescriptor],
    ) -> Result<Vec<InboundAttachment>, AttachmentMaterializationError> {
        if envelope.adapter_id().as_str() != TELEGRAM_V2_ADAPTER_ID {
            return Err(AttachmentMaterializationError::permanent(
                "Telegram attachment materializer received a foreign adapter envelope",
            ));
        }
        preflight(descriptors)?;
        let mut materialized = Vec::with_capacity(descriptors.len());
        let mut total_bytes = 0usize;
        for (index, descriptor) in descriptors.iter().enumerate() {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("file_id", &descriptor.external_file_id)
                .finish();
            let response = self
                .egress
                .send(
                    request(format!("/getFile?{query}"), self.credential_handle.clone())?
                        .with_response_body_limit(64 * 1024),
                )
                .await
                .map_err(map_egress)?;
            if !(200..300).contains(&response.status()) {
                return Err(http_error(response.status()));
            }
            let file: TelegramGetFileResponse =
                serde_json::from_slice(response.body()).map_err(|error| {
                    tracing::debug!(%error, "Telegram returned an invalid getFile response");
                    AttachmentMaterializationError::permanent(
                        "Telegram returned an invalid file response",
                    )
                })?;
            if !file.ok {
                return Err(AttachmentMaterializationError::permanent(
                    "Telegram rejected the attachment lookup",
                ));
            }
            let result = file.result.ok_or_else(|| {
                AttachmentMaterializationError::permanent("Telegram attachment has no file result")
            })?;
            let expected_size = result.file_size.or(descriptor.size_bytes);
            validate_lookup_size(expected_size)?;
            let file_path = result.file_path.ok_or_else(|| {
                AttachmentMaterializationError::permanent(
                    "Telegram attachment has no downloadable path",
                )
            })?;
            validate_file_path(&file_path)?;
            let response = self
                .egress
                .send(
                    request(format!("/file/{file_path}"), self.credential_handle.clone())?
                        .with_response_body_limit(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64),
                )
                .await
                .map_err(map_egress)?;
            if !(200..300).contains(&response.status()) {
                return Err(http_error(response.status()));
            }
            if response.body().len() > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes {
                return Err(AttachmentMaterializationError::permanent(
                    "Telegram attachment exceeds the channel size limit",
                ));
            }
            if expected_size.is_some_and(|size| response.body().len() as u64 != size) {
                return Err(AttachmentMaterializationError::retryable(
                    "Telegram attachment download was incomplete",
                ));
            }
            total_bytes = total_bytes.saturating_add(response.body().len());
            if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
                return Err(AttachmentMaterializationError::permanent(
                    "Telegram attachments exceed the channel batch limit",
                ));
            }
            materialized.push(InboundAttachment {
                id: descriptor.external_file_id.clone(),
                mime_type: normalize_mime_type(&descriptor.mime_type),
                filename: descriptor
                    .filename
                    .clone()
                    .or_else(|| Some(format!("telegram-attachment-{}", index + 1))),
                bytes: response.body().to_vec(),
            });
        }
        Ok(materialized)
    }
}

fn preflight(
    descriptors: &[ProductAttachmentDescriptor],
) -> Result<(), AttachmentMaterializationError> {
    if descriptors.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(AttachmentMaterializationError::permanent(
            "Telegram message has too many attachments",
        ));
    }
    let mut declared_total = 0u64;
    for descriptor in descriptors {
        if !is_supported_mime(&descriptor.mime_type) {
            return Err(AttachmentMaterializationError::permanent(
                "Telegram attachment MIME type is not supported",
            ));
        }
        if let Some(size) = descriptor.size_bytes {
            if size > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 {
                return Err(AttachmentMaterializationError::permanent(
                    "Telegram attachment exceeds the channel size limit",
                ));
            }
            declared_total = declared_total.saturating_add(size);
        }
    }
    if declared_total > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes as u64 {
        return Err(AttachmentMaterializationError::permanent(
            "Telegram attachments exceed the channel batch limit",
        ));
    }
    Ok(())
}

fn request(
    path: String,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, AttachmentMaterializationError> {
    let host = DeclaredEgressHost::new(TELEGRAM_API_HOST).map_err(|error| {
        tracing::debug!(%error, "Telegram attachment host failed validation");
        AttachmentMaterializationError::permanent("Telegram attachment host is invalid")
    })?;
    let method = EgressMethod::new("GET").map_err(|error| {
        tracing::debug!(%error, "Telegram attachment method failed validation");
        AttachmentMaterializationError::permanent("Telegram attachment method is invalid")
    })?;
    let path = EgressPath::new(path).map_err(|error| {
        tracing::debug!(%error, "Telegram attachment path failed validation");
        AttachmentMaterializationError::permanent("Telegram attachment path is invalid")
    })?;
    Ok(EgressRequest::new(host, method, path).with_credential_handle(Some(credential_handle)))
}

fn validate_lookup_size(size: Option<u64>) -> Result<(), AttachmentMaterializationError> {
    if size.is_some_and(|size| size > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64) {
        return Err(AttachmentMaterializationError::permanent(
            "Telegram attachment exceeds the channel size limit",
        ));
    }
    Ok(())
}

fn validate_file_path(path: &str) -> Result<(), AttachmentMaterializationError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.contains(['?', '#', '\\'])
        || path.chars().any(char::is_control)
    {
        return Err(AttachmentMaterializationError::permanent(
            "Telegram returned an invalid attachment path",
        ));
    }
    Ok(())
}

fn map_egress(error: ProtocolHttpEgressError) -> AttachmentMaterializationError {
    match error {
        ProtocolHttpEgressError::Timeout
        | ProtocolHttpEgressError::Network(_)
        | ProtocolHttpEgressError::LeakDetected => AttachmentMaterializationError::retryable(
            "Telegram attachment transfer is temporarily unavailable",
        ),
        _ => AttachmentMaterializationError::permanent("Telegram attachment transfer was denied"),
    }
}

fn http_error(status: u16) -> AttachmentMaterializationError {
    if status >= 500 || status == 429 || status == 408 {
        AttachmentMaterializationError::retryable(
            "Telegram attachment transfer is temporarily unavailable",
        )
    } else {
        AttachmentMaterializationError::permanent("Telegram attachment could not be downloaded")
    }
}

#[derive(Deserialize)]
struct TelegramGetFileResponse {
    ok: bool,
    result: Option<TelegramFileResult>,
}

#[derive(Deserialize)]
struct TelegramFileResult {
    file_path: Option<String>,
    file_size: Option<u64>,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_product_adapters::{
        AdapterInstallationId, AuthRequirement, EgressResponse, ExternalActorRef,
        ExternalConversationRef, ExternalEventId, FakeProtocolHttpEgress, ParsedProductInbound,
        ProductAdapterId, ProductAttachmentKind, ProductInboundPayload, ProtocolAuthEvidence,
        TrustedInboundContext,
    };

    use super::*;

    fn inbound_envelope() -> ProductInboundEnvelope {
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "user");
        let context = TrustedInboundContext::from_verified_evidence(
            ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID).expect("adapter id"),
            AdapterInstallationId::new("telegram-installation").expect("installation id"),
            Utc::now(),
            &evidence,
        )
        .expect("trusted context");
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new("event-1").expect("event id"),
            ExternalActorRef::new("telegram_user", "user-1", None::<String>).expect("actor ref"),
            ExternalConversationRef::new(None, "chat-1", None, None).expect("conversation ref"),
            ProductInboundPayload::NoOp,
        )
        .expect("parsed inbound");
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("inbound envelope")
    }

    #[test]
    fn telegram_file_paths_are_confined_to_provider_relative_paths() {
        assert!(validate_file_path("documents/report.pdf").is_ok());
        for path in ["../secret", "/absolute", "documents/x?token=y", "a\\b"] {
            assert!(validate_file_path(path).is_err(), "{path} must be rejected");
        }
    }

    #[test]
    fn telegram_lookup_size_rejects_oversize_before_download() {
        assert!(
            validate_lookup_size(Some(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64)).is_ok()
        );
        assert!(
            validate_lookup_size(Some(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 + 1))
                .is_err()
        );
    }

    #[tokio::test]
    async fn materializer_rejects_oversize_get_file_result_before_download() {
        let egress = Arc::new(FakeProtocolHttpEgress::new([TELEGRAM_API_HOST.to_string()]));
        egress.allow_credential_handle("telegram_bot_token");
        egress.program_response(
            TELEGRAM_API_HOST,
            Ok(EgressResponse::new(
                200,
                format!(
                    r#"{{"ok":true,"result":{{"file_path":"documents/large.pdf","file_size":{}}}}}"#,
                    DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 + 1
                )
                .into_bytes(),
            )),
        );
        let materializer = TelegramAttachmentMaterializer::new(
            egress.clone(),
            EgressCredentialHandle::new("telegram_bot_token").expect("credential handle"),
        );
        let descriptor = ProductAttachmentDescriptor::new(
            "file-1",
            "application/pdf",
            Some("large.pdf".to_string()),
            None,
            ProductAttachmentKind::Document,
        )
        .expect("descriptor");

        let result = materializer
            .materialize(&inbound_envelope(), &[descriptor])
            .await;

        assert!(result.is_err());
        let calls = egress.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].path.starts_with("/getFile?"));
    }
}
