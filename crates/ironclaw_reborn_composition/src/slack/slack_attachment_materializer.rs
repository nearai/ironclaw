//! Slack file transfer into the canonical inbound attachment lander.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::{DEFAULT_ATTACHMENT_BUDGETS, InboundAttachment};
use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressMethod, EgressPath, EgressRequest,
    ProductAttachmentDescriptor, ProductInboundEnvelope, ProtocolHttpEgress,
    ProtocolHttpEgressError,
};
use ironclaw_product_workflow::{AttachmentMaterializationError, InboundAttachmentMaterializer};
use ironclaw_slack_v2_adapter::{SLACK_API_HOST, SLACK_FILES_HOST, SLACK_V2_ADAPTER_ID};
use serde::Deserialize;
use url::Url;

pub(crate) struct SlackAttachmentMaterializer {
    egress: Arc<dyn ProtocolHttpEgress>,
    credential_handle: EgressCredentialHandle,
}

impl SlackAttachmentMaterializer {
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
        let mut total_bytes = 0usize;
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
                .map_err(|_| {
                    AttachmentMaterializationError::permanent(
                        "Slack returned an invalid file response",
                    )
                })?;
            if !file_info.ok {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack rejected the attachment lookup",
                ));
            }
            let private_url = file_info
                .file
                .and_then(|file| file.url_private_download.or(file.url_private))
                .ok_or_else(|| {
                    AttachmentMaterializationError::permanent(
                        "Slack attachment has no downloadable URL",
                    )
                })?;
            let path = confined_download_path(&private_url)?;
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
            total_bytes = total_bytes.saturating_add(response.body().len());
            if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack attachments exceed the channel batch limit",
                ));
            }
            materialized.push(InboundAttachment {
                id: descriptor.external_file_id.clone(),
                mime_type: descriptor.mime_type.clone(),
                filename: descriptor.filename.clone(),
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
    let host = DeclaredEgressHost::new(host).map_err(|_| {
        AttachmentMaterializationError::permanent("Slack attachment host is invalid")
    })?;
    let method = EgressMethod::new("GET").map_err(|_| {
        AttachmentMaterializationError::permanent("Slack attachment method is invalid")
    })?;
    let path = EgressPath::new(path).map_err(|_| {
        AttachmentMaterializationError::permanent("Slack attachment path is invalid")
    })?;
    Ok(EgressRequest::new(host, method, path).with_credential_handle(Some(credential_handle)))
}

fn confined_download_path(raw: &str) -> Result<String, AttachmentMaterializationError> {
    let parsed = Url::parse(raw).map_err(|_| {
        AttachmentMaterializationError::permanent("Slack returned an invalid attachment URL")
    })?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(SLACK_FILES_HOST)
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AttachmentMaterializationError::permanent(
            "Slack attachment URL escaped the allowed file host",
        ));
    }
    let mut path = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(path)
}

fn preflight(
    descriptors: &[ProductAttachmentDescriptor],
) -> Result<(), AttachmentMaterializationError> {
    if descriptors.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(AttachmentMaterializationError::permanent(
            "Slack message has too many attachments",
        ));
    }
    let mut declared_total = 0u64;
    for descriptor in descriptors {
        if let Some(size) = descriptor.size_bytes {
            if size > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 {
                return Err(AttachmentMaterializationError::permanent(
                    "Slack attachment exceeds the channel size limit",
                ));
            }
            declared_total = declared_total.saturating_add(size);
        }
    }
    if declared_total > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes as u64 {
        return Err(AttachmentMaterializationError::permanent(
            "Slack attachments exceed the channel batch limit",
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
    url_private: Option<String>,
    url_private_download: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_private_download_url_is_exact_host_confined() {
        assert_eq!(
            confined_download_path("https://files.slack.com/files-pri/T-F/report.pdf?pub_secret=x")
                .expect("exact Slack file host"),
            "/files-pri/T-F/report.pdf?pub_secret=x"
        );
        for url in [
            "http://files.slack.com/files-pri/report.pdf",
            "https://files.slack.com.evil.example/report.pdf",
            "https://user@files.slack.com/report.pdf",
        ] {
            assert!(
                confined_download_path(url).is_err(),
                "{url} must be rejected"
            );
        }
    }
}
