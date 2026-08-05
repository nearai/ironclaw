//! Slack attachment transfer over the generic restricted-egress boundary.
//!
//! Events retain only stable file IDs. Current metadata and private URLs are
//! resolved immediately before a bounded download and are never persisted or
//! returned to the model.

use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use std::collections::HashMap;

use ironclaw_extension_contracts::channel_adapter::{
    ChannelAttachmentRef, ChannelError, PartDeliveryOutcome,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest,
};
use ironclaw_host_api::{
    action::NetworkMethod,
    attachment::{InboundAttachment, WorkspaceFile},
    ids::SecretHandle,
};
use serde::Deserialize;
use url::Url;

use crate::payload::SLACK_API_HOST;
use crate::{
    channel::{part_outcome_for_egress_error, part_outcome_for_kind},
    delivery::{SlackDeliveryFailureKind, slack_error_kind},
};

const SLACK_FILES_HOST: &str = "files.slack.com";
const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
const SLACK_PRIVATE_FILE_PATH_PREFIX: &str = "/files-pri/";
pub(crate) const SLACK_MAX_TRANSFER_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Deserialize)]
struct SlackFilesInfoResponse {
    ok: bool,
    error: Option<String>,
    file: Option<SlackFileInfo>,
}

#[derive(Deserialize)]
struct SlackFileInfo {
    id: String,
    name: Option<String>,
    mimetype: Option<String>,
    size: Option<u64>,
    url_private: Option<String>,
    url_private_download: Option<String>,
    #[serde(default)]
    channels: Vec<String>,
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    ims: Vec<String>,
    #[serde(default)]
    shares: SlackFileShares,
}

#[derive(Default, Deserialize)]
struct SlackFileShares {
    #[serde(default)]
    public: HashMap<String, Vec<SlackFileShare>>,
    #[serde(default)]
    private: HashMap<String, Vec<SlackFileShare>>,
}

#[derive(Deserialize)]
struct SlackFileShare {
    thread_ts: Option<String>,
}

#[derive(Deserialize)]
struct SlackGetUploadUrlResponse {
    ok: bool,
    error: Option<String>,
    upload_url: Option<String>,
    file_id: Option<String>,
}

#[derive(Deserialize)]
struct SlackCompleteUploadResponse {
    ok: bool,
    error: Option<String>,
    #[serde(default)]
    files: Vec<SlackCompletedFile>,
}

#[derive(Deserialize)]
struct SlackCompletedFile {
    id: String,
}

struct ValidatedSlackUpload<'a> {
    filename: String,
    mime_type: String,
    file: &'a WorkspaceFile,
}

struct StagedSlackUpload<'a> {
    file_id: String,
    filename: String,
    file: &'a WorkspaceFile,
}

pub(super) async fn fetch_attachment(
    attachment: &ChannelAttachmentRef,
    egress: &dyn RestrictedEgress,
) -> Result<InboundAttachment, ChannelError> {
    let max_file_bytes = max_transfer_bytes();
    if attachment
        .descriptor
        .size_bytes
        .is_some_and(|size| size > max_file_bytes)
    {
        return Err(transfer_error(
            "slack attachment exceeds the channel size limit",
            false,
        ));
    }

    let info = egress
        .send(files_info_request(&attachment.vendor_ref)?)
        .await
        .map_err(map_egress_error)?;
    if !(200..300).contains(&info.status) {
        return Err(status_error(info.status));
    }
    let info: SlackFilesInfoResponse = serde_json::from_slice(&info.body)
        .map_err(|_| transfer_error("slack returned an invalid file response", true))?;
    if !info.ok {
        return Err(api_error(info.error.as_deref()));
    }
    let file = info
        .file
        .ok_or_else(|| transfer_error("slack file response omitted file metadata", false))?;
    if file.id != attachment.vendor_ref || file.id != attachment.descriptor.external_file_id {
        return Err(transfer_error(
            "slack attachment identity did not match",
            false,
        ));
    }

    let provider_mime = file
        .mimetype
        .as_deref()
        .unwrap_or("application/octet-stream")
        .to_ascii_lowercase();
    if !provider_mime.eq_ignore_ascii_case(&attachment.descriptor.mime_type) {
        return Err(transfer_error(
            "slack attachment MIME metadata did not match",
            false,
        ));
    }
    if let (Some(provider_size), Some(descriptor_size)) =
        (file.size, attachment.descriptor.size_bytes)
        && provider_size != descriptor_size
    {
        return Err(transfer_error(
            "slack attachment size metadata did not match",
            false,
        ));
    }
    let expected_size = file.size.or(attachment.descriptor.size_bytes);
    if expected_size.is_some_and(|size| size > max_file_bytes) {
        return Err(transfer_error(
            "slack attachment exceeds the channel size limit",
            false,
        ));
    }

    let private_url = file
        .url_private_download
        .or(file.url_private)
        .ok_or_else(|| transfer_error("slack attachment has no downloadable URL", false))?;
    validate_private_file_url(&private_url)?;
    let download = egress
        .send(private_file_request(private_url)?)
        .await
        .map_err(map_egress_error)?;
    if !(200..300).contains(&download.status) {
        return Err(status_error(download.status));
    }
    let actual_size = download.body.len() as u64;
    if actual_size > max_file_bytes {
        return Err(transfer_error(
            "slack attachment exceeds the channel size limit",
            false,
        ));
    }
    if let Some(expected_size) = expected_size
        && actual_size != expected_size
    {
        return Err(transfer_error(
            "slack attachment download size did not match provider metadata",
            actual_size < expected_size,
        ));
    }

    let filename = inbound_filename(attachment.descriptor.filename.clone().or(file.name))?;
    Ok(InboundAttachment {
        id: attachment.descriptor.external_file_id.clone(),
        mime_type: attachment.descriptor.mime_type.clone(),
        filename,
        bytes: download.body,
    })
}

pub(super) async fn send_files(
    egress: &dyn RestrictedEgress,
    credential: &SecretHandle,
    channel: &str,
    thread_ts: Option<&str>,
    files: &[&WorkspaceFile],
) -> Vec<PartDeliveryOutcome> {
    let max_file_bytes = max_transfer_bytes();
    // Validate the whole batch before acquiring provider tickets. Slack only
    // receives bytes after every local filename, MIME, and size is safe.
    let mut validated = Vec::with_capacity(files.len());
    for file in files {
        if file.bytes.is_empty() {
            return vec![PartDeliveryOutcome::Permanent {
                reason: "slack does not accept empty attachments".to_string(),
            }];
        }
        if file.bytes.len() as u64 > max_file_bytes {
            return vec![PartDeliveryOutcome::Permanent {
                reason: "slack attachment exceeds the channel size limit".to_string(),
            }];
        }
        let filename = match outbound_filename(file) {
            Ok(filename) => filename,
            Err(reason) => return vec![PartDeliveryOutcome::Permanent { reason }],
        };
        let mime_type = match outbound_mime_type(&file.mime_type) {
            Ok(mime_type) => mime_type,
            Err(reason) => return vec![PartDeliveryOutcome::Permanent { reason }],
        };
        validated.push(ValidatedSlackUpload {
            filename,
            mime_type,
            file,
        });
    }

    // Upload every file to its private ticket before sharing any file into
    // the destination. The single completion below is the provider-visible
    // batch boundary and preserves the finalized attachment order.
    let mut staged = Vec::with_capacity(validated.len());
    for upload in validated {
        let ticket_request =
            match upload_ticket_request(&upload.filename, upload.file.bytes.len(), credential) {
                Ok(request) => request,
                Err(reason) => return vec![PartDeliveryOutcome::Permanent { reason }],
            };
        let ticket_response = match egress.send(ticket_request).await {
            Ok(response) => response,
            Err(error) => return vec![part_outcome_for_egress_error(&error)],
        };
        if !(200..300).contains(&ticket_response.status) {
            return vec![part_outcome_for_kind(
                SlackDeliveryFailureKind::from_http_status(ticket_response.status),
                format!(
                    "slack files.getUploadURLExternal returned status {}",
                    ticket_response.status
                ),
            )];
        }
        let ticket: SlackGetUploadUrlResponse = match serde_json::from_slice(&ticket_response.body)
        {
            Ok(ticket) => ticket,
            Err(_) => {
                return vec![PartDeliveryOutcome::Retryable {
                    reason: "slack files.getUploadURLExternal returned an invalid response"
                        .to_string(),
                }];
            }
        };
        if !ticket.ok {
            return vec![outbound_api_error(
                "files.getUploadURLExternal",
                ticket.error.as_deref(),
            )];
        }
        let Some(upload_url) = ticket.upload_url else {
            return vec![PartDeliveryOutcome::Permanent {
                reason: "slack upload ticket omitted the upload URL".to_string(),
            }];
        };
        let Some(file_id) = ticket.file_id.filter(|id| valid_slack_file_id(id)) else {
            return vec![PartDeliveryOutcome::Permanent {
                reason: "slack upload ticket omitted a valid file ID".to_string(),
            }];
        };

        let upload_request =
            match upload_file_request(upload_url, upload.mime_type, &upload.file.bytes) {
                Ok(request) => request,
                Err(reason) => return vec![PartDeliveryOutcome::Permanent { reason }],
            };
        let upload_response = match egress.send(upload_request).await {
            Ok(response) => response,
            Err(error) => return vec![part_outcome_for_egress_error(&error)],
        };
        if !(200..300).contains(&upload_response.status) {
            return vec![part_outcome_for_kind(
                SlackDeliveryFailureKind::from_http_status(upload_response.status),
                format!(
                    "slack external file upload returned status {}",
                    upload_response.status
                ),
            )];
        }
        staged.push(StagedSlackUpload {
            file_id,
            filename: upload.filename,
            file: upload.file,
        });
    }

    let complete_request = match complete_upload_request(&staged, channel, thread_ts, credential) {
        Ok(request) => request,
        Err(reason) => return vec![PartDeliveryOutcome::Permanent { reason }],
    };
    let complete_response = match egress.send(complete_request).await {
        Ok(response) => response,
        Err(error) => return vec![part_outcome_for_egress_error(&error)],
    };
    if !(200..300).contains(&complete_response.status) {
        return vec![part_outcome_for_kind(
            SlackDeliveryFailureKind::from_http_status(complete_response.status),
            format!(
                "slack files.completeUploadExternal returned status {}",
                complete_response.status
            ),
        )];
    }
    let completed: SlackCompleteUploadResponse =
        match serde_json::from_slice(&complete_response.body) {
            Ok(completed) => completed,
            Err(_) => {
                return vec![PartDeliveryOutcome::Retryable {
                    reason: "slack files.completeUploadExternal returned an invalid response"
                        .to_string(),
                }];
            }
        };
    if !completed.ok {
        return vec![outbound_api_error(
            "files.completeUploadExternal",
            completed.error.as_deref(),
        )];
    }

    let mut outcomes = Vec::with_capacity(staged.len());
    for upload in staged {
        if !completed.files.iter().any(|file| file.id == upload.file_id) {
            outcomes.push(PartDeliveryOutcome::Permanent {
                reason: "slack upload completion did not confirm the file ID".to_string(),
            });
            continue;
        }
        outcomes.push(read_back_staged_upload(egress, channel, thread_ts, &upload).await);
    }
    outcomes
}

pub(crate) const SLACK_FILE_READBACK_MAX_ATTEMPTS: u8 = 8;
const SLACK_FILE_READBACK_BASE_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);
const SLACK_FILE_READBACK_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);

// Slack can acknowledge completion before destination indexes become visible
// through files.info. Only this read-back is retried: tickets, byte uploads,
// and completion are never replayed after provider acceptance.
async fn read_back_staged_upload(
    egress: &dyn RestrictedEgress,
    channel: &str,
    thread_ts: Option<&str>,
    upload: &StagedSlackUpload<'_>,
) -> PartDeliveryOutcome {
    for attempt in 1..=SLACK_FILE_READBACK_MAX_ATTEMPTS {
        let info_request = match files_info_request(&upload.file_id) {
            Ok(request) => request,
            Err(_) => {
                return PartDeliveryOutcome::Permanent {
                    reason: "slack file read-back request was invalid".to_string(),
                };
            }
        };
        let info_response = match egress.send(info_request).await {
            Ok(response) => response,
            Err(error) => {
                let outcome = part_outcome_for_egress_error(&error);
                if attempt < SLACK_FILE_READBACK_MAX_ATTEMPTS
                    && matches!(outcome, PartDeliveryOutcome::Retryable { .. })
                {
                    tokio::time::sleep(file_readback_backoff(attempt)).await;
                    continue;
                }
                return terminal_readback_outcome(outcome);
            }
        };
        if !(200..300).contains(&info_response.status) {
            let outcome = part_outcome_for_kind(
                SlackDeliveryFailureKind::from_http_status(info_response.status),
                format!(
                    "slack files.info read-back returned status {}",
                    info_response.status
                ),
            );
            if attempt < SLACK_FILE_READBACK_MAX_ATTEMPTS
                && matches!(outcome, PartDeliveryOutcome::Retryable { .. })
            {
                tokio::time::sleep(file_readback_backoff(attempt)).await;
                continue;
            }
            return terminal_readback_outcome(outcome);
        }
        let info: SlackFilesInfoResponse = match serde_json::from_slice(&info_response.body) {
            Ok(info) => info,
            Err(_) if attempt < SLACK_FILE_READBACK_MAX_ATTEMPTS => {
                tokio::time::sleep(file_readback_backoff(attempt)).await;
                continue;
            }
            Err(_) => {
                return PartDeliveryOutcome::Permanent {
                    reason: "slack file read-back remained unavailable after upload completion"
                        .to_string(),
                };
            }
        };
        if !info.ok {
            if info.error.as_deref() == Some("file_not_found")
                && attempt < SLACK_FILE_READBACK_MAX_ATTEMPTS
            {
                tokio::time::sleep(file_readback_backoff(attempt)).await;
                continue;
            }
            return outbound_api_error("files.info", info.error.as_deref());
        }
        let Some(info) = info.file else {
            if attempt < SLACK_FILE_READBACK_MAX_ATTEMPTS {
                tokio::time::sleep(file_readback_backoff(attempt)).await;
                continue;
            }
            return PartDeliveryOutcome::Permanent {
                reason: "slack file read-back remained unavailable after upload completion"
                    .to_string(),
            };
        };
        if info.id != upload.file_id
            || info.size != Some(upload.file.bytes.len() as u64)
            || info.name.as_deref() != Some(upload.filename.as_str())
        {
            return PartDeliveryOutcome::Permanent {
                reason: "slack file read-back did not match the requested delivery".to_string(),
            };
        }
        if destination_matches(&info, channel, thread_ts) {
            return PartDeliveryOutcome::Sent {
                vendor_message_ref: Some(upload.file_id.clone()),
            };
        }
        if attempt < SLACK_FILE_READBACK_MAX_ATTEMPTS {
            tokio::time::sleep(file_readback_backoff(attempt)).await;
            continue;
        }
        return PartDeliveryOutcome::Permanent {
            reason: "slack file read-back did not match the requested delivery".to_string(),
        };
    }

    PartDeliveryOutcome::Permanent {
        reason: "slack file read-back remained unavailable after upload completion".to_string(),
    }
}

fn file_readback_backoff(attempt: u8) -> std::time::Duration {
    let exponent = attempt.saturating_sub(1).min(4);
    SLACK_FILE_READBACK_BASE_BACKOFF
        .saturating_mul(1_u32 << exponent)
        .min(SLACK_FILE_READBACK_MAX_BACKOFF)
}

fn terminal_readback_outcome(outcome: PartDeliveryOutcome) -> PartDeliveryOutcome {
    if matches!(outcome, PartDeliveryOutcome::Retryable { .. }) {
        PartDeliveryOutcome::Permanent {
            reason: "slack file read-back remained unavailable after upload completion".to_string(),
        }
    } else {
        outcome
    }
}

fn max_transfer_bytes() -> u64 {
    SLACK_MAX_TRANSFER_BYTES.min(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64)
}

fn upload_ticket_request(
    filename: &str,
    length: usize,
    credential: &SecretHandle,
) -> Result<RestrictedEgressRequest, String> {
    let mut url = Url::parse(&format!(
        "https://{SLACK_API_HOST}/api/files.getUploadURLExternal"
    ))
    .map_err(|_| "slack upload ticket URL is invalid".to_string())?;
    url.query_pairs_mut()
        .append_pair("filename", filename)
        .append_pair("length", &length.to_string());
    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Get,
        url: url.into(),
        headers: Vec::new(),
        body: None,
        credential: Some(credential.clone()),
        body_credentials: Vec::new(),
    })
}

fn upload_file_request(
    upload_url: String,
    mime_type: String,
    bytes: &[u8],
) -> Result<RestrictedEgressRequest, String> {
    validate_upload_url(&upload_url)?;
    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Post,
        url: upload_url,
        headers: vec![("content-type".to_string(), mime_type)],
        body: Some(bytes.to_vec()),
        credential: None,
        body_credentials: Vec::new(),
    })
}

fn complete_upload_request(
    files: &[StagedSlackUpload<'_>],
    channel: &str,
    thread_ts: Option<&str>,
    credential: &SecretHandle,
) -> Result<RestrictedEgressRequest, String> {
    let files: Vec<_> = files
        .iter()
        .map(|file| serde_json::json!({"id": file.file_id, "title": file.filename}))
        .collect();
    let mut body = serde_json::json!({
        "files": files,
        "channel_id": channel,
    });
    if let Some(thread_ts) = thread_ts {
        body["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
    }
    let body = serde_json::to_vec(&body)
        .map_err(|_| "slack upload completion body did not serialize".to_string())?;
    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Post,
        url: format!("https://{SLACK_API_HOST}/api/files.completeUploadExternal"),
        headers: vec![(
            "content-type".to_string(),
            "application/json; charset=utf-8".to_string(),
        )],
        body: Some(body),
        credential: Some(credential.clone()),
        body_credentials: Vec::new(),
    })
}

fn files_info_request(file_id: &str) -> Result<RestrictedEgressRequest, ChannelError> {
    let mut url = Url::parse(&format!("https://{SLACK_API_HOST}/api/files.info"))
        .map_err(|_| transfer_error("slack file lookup URL is invalid", false))?;
    url.query_pairs_mut().append_pair("file", file_id);
    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Get,
        url: url.into(),
        headers: Vec::new(),
        body: None,
        credential: Some(bot_token_handle()?),
        body_credentials: Vec::new(),
    })
}

fn private_file_request(url: String) -> Result<RestrictedEgressRequest, ChannelError> {
    validate_private_file_url(&url)?;
    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Get,
        url,
        headers: Vec::new(),
        body: None,
        credential: Some(bot_token_handle()?),
        body_credentials: Vec::new(),
    })
}

fn bot_token_handle() -> Result<SecretHandle, ChannelError> {
    SecretHandle::new(SLACK_BOT_TOKEN_HANDLE)
        .map_err(|_| transfer_error("slack attachment credentials are unavailable", false))
}

fn validate_private_file_url(value: &str) -> Result<(), ChannelError> {
    let parsed = Url::parse(value)
        .map_err(|_| transfer_error("slack returned an invalid attachment URL", false))?;
    let valid = parsed.scheme() == "https"
        && parsed.host_str() == Some(SLACK_FILES_HOST)
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none()
        && parsed.path().starts_with(SLACK_PRIVATE_FILE_PATH_PREFIX);
    if valid {
        Ok(())
    } else {
        Err(transfer_error(
            "slack returned an invalid attachment URL",
            false,
        ))
    }
}

fn validate_upload_url(value: &str) -> Result<(), String> {
    let parsed =
        Url::parse(value).map_err(|_| "slack returned an invalid upload URL".to_string())?;
    let valid = parsed.scheme() == "https"
        && parsed.host_str() == Some(SLACK_FILES_HOST)
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none()
        && parsed.path().starts_with("/upload/");
    if valid {
        Ok(())
    } else {
        Err("slack returned an invalid upload URL".to_string())
    }
}

fn outbound_filename(file: &WorkspaceFile) -> Result<String, String> {
    let filename = file
        .filename
        .as_deref()
        .or_else(|| file.path.as_str().rsplit('/').next())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 255
                && !matches!(*value, "." | "..")
                && !value.contains(['/', '\\', '\r', '\n', '\0'])
        })
        .ok_or_else(|| "slack attachment filename is invalid".to_string())?;
    Ok(filename.to_string())
}

fn inbound_filename(filename: Option<String>) -> Result<Option<String>, ChannelError> {
    let Some(filename) = filename else {
        return Ok(None);
    };
    let valid = !filename.is_empty()
        && filename.len() <= 255
        && !matches!(filename.as_str(), "." | "..")
        && !filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'));
    if valid {
        Ok(Some(filename))
    } else {
        Err(transfer_error(
            "slack attachment filename is invalid",
            false,
        ))
    }
}

fn outbound_mime_type(value: &str) -> Result<String, String> {
    let valid = !value.is_empty()
        && value.len() <= 127
        && value.is_ascii()
        && value == value.to_ascii_lowercase()
        && value.contains('/')
        && !value.chars().any(|character| character.is_ascii_control());
    if valid {
        Ok(value.to_string())
    } else {
        Err("slack attachment MIME type is invalid".to_string())
    }
}

fn valid_slack_file_id(value: &str) -> bool {
    value.starts_with('F')
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn destination_matches(file: &SlackFileInfo, channel: &str, thread_ts: Option<&str>) -> bool {
    let shared_to_channel = file.channels.iter().any(|value| value == channel)
        || file.groups.iter().any(|value| value == channel)
        || file.ims.iter().any(|value| value == channel);
    if !shared_to_channel {
        return false;
    }
    let Some(thread_ts) = thread_ts else {
        return true;
    };
    [&file.shares.public, &file.shares.private]
        .into_iter()
        .filter_map(|shares| shares.get(channel))
        .flatten()
        .any(|share| share.thread_ts.as_deref() == Some(thread_ts))
}

fn outbound_api_error(method: &str, error: Option<&str>) -> PartDeliveryOutcome {
    let error = error.unwrap_or("unknown_error");
    part_outcome_for_kind(
        slack_error_kind(error),
        format!("slack rejected {method} ({error})"),
    )
}

fn transfer_error(reason: &str, retryable: bool) -> ChannelError {
    ChannelError::AttachmentTransfer {
        reason: reason.to_string(),
        retryable,
    }
}

fn status_error(status: u16) -> ChannelError {
    let retryable = status >= 500 || matches!(status, 408 | 429);
    transfer_error(
        if retryable {
            "slack attachment transfer is temporarily unavailable"
        } else if matches!(status, 401 | 403) {
            "slack attachment transfer is unauthorized"
        } else {
            "slack attachment could not be downloaded"
        },
        retryable,
    )
}

fn api_error(error: Option<&str>) -> ChannelError {
    let retryable = matches!(
        error,
        Some(
            "fatal_error"
                | "internal_error"
                | "service_unavailable"
                | "request_timeout"
                | "ratelimited"
        )
    );
    let unauthorized = matches!(
        error,
        Some(
            "not_authed"
                | "invalid_auth"
                | "account_inactive"
                | "token_revoked"
                | "missing_scope"
                | "no_permission"
                | "not_allowed_token_type"
        )
    );
    transfer_error(
        if unauthorized {
            "slack attachment transfer is unauthorized"
        } else if retryable {
            "slack attachment transfer is temporarily unavailable"
        } else {
            "slack attachment could not be downloaded"
        },
        retryable,
    )
}

fn map_egress_error(error: RestrictedEgressError) -> ChannelError {
    match error {
        RestrictedEgressError::Transport { .. } => {
            transfer_error("slack attachment transfer is temporarily unavailable", true)
        }
        RestrictedEgressError::AuthRequired { .. }
        | RestrictedEgressError::UndeclaredCredential { .. } => {
            transfer_error("slack attachment transfer is unauthorized", false)
        }
        RestrictedEgressError::ResponseTooLarge => {
            transfer_error("slack attachment exceeds the channel size limit", false)
        }
        RestrictedEgressError::UndeclaredHost { .. }
        | RestrictedEgressError::UndeclaredMethod
        | RestrictedEgressError::HostOwnedHeader { .. }
        | RestrictedEgressError::PolicyDenied => {
            transfer_error("slack attachment transfer was denied", false)
        }
    }
}
