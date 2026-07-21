//! Slack file transfer into the canonical inbound attachment lander.

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
use ironclaw_slack_v2_adapter::{
    SLACK_API_HOST, SLACK_FILES_HOST, SLACK_V2_ADAPTER_ID, confined_slack_file_path,
};
use serde::Deserialize;

/// Materializes Slack file descriptors through mediated provider egress.
pub struct SlackAttachmentMaterializer {
    egress: Arc<dyn ProtocolHttpEgress>,
    credential_handle: EgressCredentialHandle,
}

impl SlackAttachmentMaterializer {
    /// Creates a materializer with the host-provided mediated egress and
    /// credential handle for the selected Slack installation.
    pub fn new(
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
impl InboundAttachmentMaterializer for SlackAttachmentMaterializer {
    async fn materialize(
        &self,
        envelope: &ProductInboundEnvelope,
        descriptors: &[ProductAttachmentDescriptor],
    ) -> Result<Vec<InboundAttachment>, AttachmentMaterializationError> {
        if envelope.adapter_id().as_str() != SLACK_V2_ADAPTER_ID {
            return Err(AttachmentMaterializationError::permanent(
                "Slack attachment materializer received a foreign adapter envelope",
            ));
        }
        preflight(descriptors)?;
        let mut materialized = Vec::with_capacity(descriptors.len());
        let mut refreshed_total_bytes = 0u64;
        for descriptor in descriptors {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("file", &descriptor.external_file_id)
                .finish();
            let response = self
                .egress
                .send(
                    request(
                        SLACK_API_HOST,
                        format!("/api/files.info?{query}"),
                        self.credential_handle.clone(),
                    )?
                    .with_response_body_limit(64 * 1024),
                )
                .await
                .map_err(map_egress)?;
            if !(200..300).contains(&response.status()) {
                return Err(http_error(response.status()));
            }
            let file_info: SlackFileInfoResponse = serde_json::from_slice(response.body())
                .map_err(|error| {
                    tracing::debug!(%error, "Slack returned an invalid files.info response");
                    AttachmentMaterializationError::permanent(
                        "Slack returned an invalid file response",
                    )
                })?;
            if !file_info.ok {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack rejected the attachment lookup",
                ));
            }
            let file = file_info.file.ok_or_else(|| {
                AttachmentMaterializationError::permanent("Slack attachment has no file metadata")
            })?;
            if file.id != descriptor.external_file_id {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack returned metadata for a different attachment",
                ));
            }
            let mime_type = normalize_mime_type(&file.mimetype);
            if !is_supported_mime(&mime_type) {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack attachment MIME type is not supported",
                ));
            }
            if file.size > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack attachment exceeds the channel size limit",
                ));
            }
            refreshed_total_bytes = refreshed_total_bytes.saturating_add(file.size);
            if refreshed_total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes as u64 {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack attachments exceed the channel batch limit",
                ));
            }
            let private_url = file
                .url_private_download
                .or(file.url_private)
                .ok_or_else(|| {
                    AttachmentMaterializationError::permanent(
                        "Slack attachment has no downloadable URL",
                    )
                })?;
            let path = confined_slack_file_path(&private_url).map_err(|error| {
                tracing::debug!(%error, "Slack returned an unconfined attachment URL");
                AttachmentMaterializationError::permanent(
                    "Slack attachment URL escaped the allowed file host",
                )
            })?;
            let response = self
                .egress
                .send(
                    request(SLACK_FILES_HOST, path, self.credential_handle.clone())?
                        .with_response_body_limit(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64),
                )
                .await
                .map_err(map_egress)?;
            if !(200..300).contains(&response.status()) {
                return Err(http_error(response.status()));
            }
            if response.body().len() > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack attachment exceeds the channel size limit",
                ));
            }
            if response.body().len() as u64 != file.size {
                return Err(AttachmentMaterializationError::retryable(
                    "Slack attachment download was incomplete",
                ));
            }
            materialized.push(InboundAttachment {
                id: file.id,
                mime_type,
                filename: file.name,
                bytes: response.body().to_vec(),
            });
        }
        Ok(materialized)
    }
}

fn request(
    host: &str,
    path: String,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, AttachmentMaterializationError> {
    let host = DeclaredEgressHost::new(host).map_err(|error| {
        tracing::debug!(%error, "Slack attachment host failed validation");
        AttachmentMaterializationError::permanent("Slack attachment host is invalid")
    })?;
    let method = EgressMethod::new("GET").map_err(|error| {
        tracing::debug!(%error, "Slack attachment method failed validation");
        AttachmentMaterializationError::permanent("Slack attachment method is invalid")
    })?;
    let path = EgressPath::new(path).map_err(|error| {
        tracing::debug!(%error, "Slack attachment path failed validation");
        AttachmentMaterializationError::permanent("Slack attachment path is invalid")
    })?;
    Ok(EgressRequest::new(host, method, path).with_credential_handle(Some(credential_handle)))
}

fn preflight(
    descriptors: &[ProductAttachmentDescriptor],
) -> Result<(), AttachmentMaterializationError> {
    if descriptors.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(AttachmentMaterializationError::permanent(
            "Slack message has too many attachments",
        ));
    }
    Ok(())
}

fn map_egress(error: ProtocolHttpEgressError) -> AttachmentMaterializationError {
    match error {
        ProtocolHttpEgressError::Timeout
        | ProtocolHttpEgressError::Network(_)
        | ProtocolHttpEgressError::LeakDetected => AttachmentMaterializationError::retryable(
            "Slack attachment transfer is temporarily unavailable",
        ),
        _ => AttachmentMaterializationError::permanent("Slack attachment transfer was denied"),
    }
}

fn http_error(status: u16) -> AttachmentMaterializationError {
    if status >= 500 || status == 429 || status == 408 {
        AttachmentMaterializationError::retryable(
            "Slack attachment transfer is temporarily unavailable",
        )
    } else {
        AttachmentMaterializationError::permanent("Slack attachment could not be downloaded")
    }
}

#[derive(Deserialize)]
struct SlackFileInfoResponse {
    ok: bool,
    file: Option<SlackFileInfo>,
}

#[derive(Deserialize)]
struct SlackFileInfo {
    id: String,
    name: Option<String>,
    mimetype: String,
    size: u64,
    url_private: Option<String>,
    url_private_download: Option<String>,
}
