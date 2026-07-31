//! Telegram attachment transfer over the generic channel restricted-egress
//! boundary. Bytes stay transient and provider paths are validated before
//! they are composed under the manifest-declared file endpoint.

use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_host_api::product_adapter::{ChannelAttachmentRef, ChannelError, PartDeliveryOutcome};
use ironclaw_host_api::{
    action::NetworkMethod,
    attachment::{InboundAttachment, WorkspaceFile},
    ids::SecretHandle,
    tool_adapter::{RestrictedEgress, RestrictedEgressRequest},
};

use crate::channel::{
    TELEGRAM_BOT_TOKEN_HANDLE, TELEGRAM_TOKEN_PLACEHOLDER, telegram_message_response_outcome,
    telegram_outcome_for_egress_error,
};
use ironclaw_telegram_v2_adapter::TELEGRAM_API_HOST;

/// Largest attachment payload a Telegram transfer carries, in bytes.
///
/// This is the same number the manifest's `[[channel.egress]]` declarations
/// enforce (the download target's `response_body_limit_bytes`, and the Bot API
/// target's `request_body_limit_bytes` less the multipart allowance below).
/// Checking it adapter-side means an oversize file is refused with an accurate
/// reason *before* egress instead of being policy-denied afterwards — inbound
/// that turned "too large" into "denied", and outbound it let the coordinator
/// report a send that never happened.
pub(crate) const TELEGRAM_MAX_TRANSFER_BYTES: u64 = 5 * 1024 * 1024;

/// Multipart field overhead the manifest's request cap allows above the file.
pub(crate) const TELEGRAM_MULTIPART_OVERHEAD_BYTES: u64 = 64 * 1024;

/// The binding per-file bound: the channel's declared transfer limit and the
/// host-wide attachment budget both apply, tightest wins.
/// The declared bot-token handle. Constructed in one place: a `None` here
/// would ship a request with an unsubstituted `{telegram_bot_token}`
/// placeholder, so the failure mode is worth having a single site for.
fn bot_token_handle() -> Option<SecretHandle> {
    SecretHandle::new(TELEGRAM_BOT_TOKEN_HANDLE).ok()
}

fn max_transfer_bytes() -> u64 {
    TELEGRAM_MAX_TRANSFER_BYTES.min(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64)
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
            "telegram attachment exceeds the channel size limit",
            false,
        ));
    }

    let lookup = egress
        .send(bot_api_request(
            "getFile",
            serde_json::json!({ "file_id": attachment.vendor_ref }),
        ))
        .await
        .map_err(map_egress_error)?;
    if !(200..300).contains(&lookup.status) {
        return Err(status_error(lookup.status));
    }
    let lookup: TelegramGetFileResponse =
        serde_json::from_slice(&lookup.body).map_err(|error| {
            // The cause stays server-side; the user-facing reason is a fixed
            // literal so no provider body fragment crosses the boundary.
            tracing::debug!(%error, "telegram getFile response failed to parse");
            transfer_error("telegram returned an invalid file response", true)
        })?;
    if !lookup.ok {
        return Err(status_error(lookup.error_code.unwrap_or(400)));
    }
    let result = lookup
        .result
        .ok_or_else(|| transfer_error("telegram file response omitted result", false))?;
    let file_path = result
        .file_path
        .ok_or_else(|| transfer_error("telegram attachment has no downloadable path", false))?;
    validate_file_path(&file_path)?;

    if let (Some(provider_size), Some(descriptor_size)) =
        (result.file_size, attachment.descriptor.size_bytes)
        && provider_size != descriptor_size
    {
        return Err(transfer_error(
            "telegram attachment size metadata did not match",
            false,
        ));
    }
    let expected_size = result.file_size.or(attachment.descriptor.size_bytes);
    if expected_size.is_some_and(|size| size > max_file_bytes) {
        return Err(transfer_error(
            "telegram attachment exceeds the channel size limit",
            false,
        ));
    }

    let download = egress
        .send(file_download_request(&file_path)?)
        .await
        .map_err(map_egress_error)?;
    if !(200..300).contains(&download.status) {
        return Err(status_error(download.status));
    }
    let actual_size = download.body.len() as u64;
    if actual_size > max_file_bytes {
        return Err(transfer_error(
            "telegram attachment exceeds the channel size limit",
            false,
        ));
    }
    if let Some(expected_size) = expected_size
        && actual_size != expected_size
    {
        return Err(transfer_error(
            "telegram attachment download size did not match provider metadata",
            actual_size < expected_size,
        ));
    }

    Ok(InboundAttachment {
        id: attachment.descriptor.external_file_id.clone(),
        mime_type: attachment.descriptor.mime_type.clone(),
        filename: attachment
            .descriptor
            .filename
            .clone()
            .or_else(|| provider_filename(&file_path)),
        bytes: download.body,
    })
}

pub(super) async fn send_document(
    egress: &dyn RestrictedEgress,
    chat_id: &str,
    message_thread_id: Option<i64>,
    reply_to_message_id: Option<i64>,
    file: &WorkspaceFile,
) -> PartDeliveryOutcome {
    if file.bytes.len() as u64 > max_transfer_bytes() {
        return PartDeliveryOutcome::Permanent {
            reason: "telegram attachment exceeds the channel size limit".to_string(),
        };
    }
    let request = match document_request(chat_id, message_thread_id, reply_to_message_id, file) {
        Ok(request) => request,
        Err(reason) => return PartDeliveryOutcome::Permanent { reason },
    };
    let response = match egress.send(request).await {
        Ok(response) => response,
        Err(error) => return telegram_outcome_for_egress_error(&error),
    };
    telegram_message_response_outcome("sendDocument", response.status, &response.body)
}

#[derive(Debug, serde::Deserialize)]
struct TelegramGetFileResponse {
    ok: bool,
    error_code: Option<u16>,
    result: Option<TelegramFileResult>,
}

#[derive(Debug, serde::Deserialize)]
struct TelegramFileResult {
    file_size: Option<u64>,
    file_path: Option<String>,
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
            "telegram attachment transfer is temporarily unavailable"
        } else if matches!(status, 401 | 403) {
            "telegram attachment transfer is unauthorized"
        } else {
            "telegram attachment could not be downloaded"
        },
        retryable,
    )
}

fn map_egress_error(error: ironclaw_host_api::tool_adapter::RestrictedEgressError) -> ChannelError {
    use ironclaw_host_api::tool_adapter::RestrictedEgressError as EgressError;
    match error {
        EgressError::Transport { .. } => transfer_error(
            "telegram attachment transfer is temporarily unavailable",
            true,
        ),
        EgressError::AuthRequired { .. } | EgressError::UndeclaredCredential { .. } => {
            transfer_error("telegram attachment transfer is unauthorized", false)
        }
        // Size and policy are distinct outcomes: reporting an oversize
        // download as "denied" sends the user chasing a permission problem.
        EgressError::ResponseTooLarge => {
            transfer_error("telegram attachment exceeds the channel size limit", false)
        }
        EgressError::UndeclaredHost { .. }
        | EgressError::UndeclaredMethod
        | EgressError::HostOwnedHeader { .. }
        | EgressError::PolicyDenied => {
            transfer_error("telegram attachment transfer was denied", false)
        }
    }
}

fn validate_file_path(path: &str) -> Result<(), ChannelError> {
    let valid = !path.is_empty()
        && path.len() <= 1_024
        && !path.starts_with('/')
        && !path.contains("://")
        && !path.contains(['?', '#', '\\', '%'])
        && !path.chars().any(|character| character.is_control())
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && !matches!(segment, "." | "..")
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
                })
        });
    if valid {
        Ok(())
    } else {
        Err(transfer_error(
            "telegram returned an invalid attachment path",
            false,
        ))
    }
}

fn file_download_request(file_path: &str) -> Result<RestrictedEgressRequest, ChannelError> {
    validate_file_path(file_path)?;
    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Get,
        url: format!(
            "https://{TELEGRAM_API_HOST}/file/bot{{{TELEGRAM_TOKEN_PLACEHOLDER}}}/{file_path}"
        ),
        headers: Vec::new(),
        body: None,
        credential: bot_token_handle(),
        body_credentials: Vec::new(),
    })
}

fn provider_filename(file_path: &str) -> Option<String> {
    file_path
        .rsplit('/')
        .next()
        .filter(|filename| !filename.is_empty())
        .map(str::to_string)
}

fn bot_api_request(method: &str, body: serde_json::Value) -> RestrictedEgressRequest {
    RestrictedEgressRequest {
        method: NetworkMethod::Post,
        url: format!("https://{TELEGRAM_API_HOST}/bot{{{TELEGRAM_TOKEN_PLACEHOLDER}}}/{method}"),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: Some(body.to_string().into_bytes()),
        credential: bot_token_handle(),
        body_credentials: Vec::new(),
    }
}

fn document_request(
    chat_id: &str,
    message_thread_id: Option<i64>,
    reply_to_message_id: Option<i64>,
    file: &WorkspaceFile,
) -> Result<RestrictedEgressRequest, String> {
    if chat_id.chars().any(|character| character.is_control()) {
        return Err("telegram chat id is invalid".to_string());
    }
    let boundary = multipart_boundary(&file.bytes)?;
    let mut body = Vec::with_capacity(file.bytes.len().saturating_add(1_024));
    push_text(&mut body, &boundary, "chat_id", chat_id);
    if let Some(thread_id) = message_thread_id {
        push_text(
            &mut body,
            &boundary,
            "message_thread_id",
            &thread_id.to_string(),
        );
    }
    if let Some(reply_to) = reply_to_message_id {
        push_text(
            &mut body,
            &boundary,
            "reply_to_message_id",
            &reply_to.to_string(),
        );
    }
    let filename = safe_filename(
        file.filename
            .as_deref()
            .or_else(|| file.path.as_str().rsplit('/').next()),
    );
    let mime_type = safe_mime(&file.mime_type);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"document\"; filename=\"{filename}\"\r\nContent-Type: {mime_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&file.bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    // The assembled body, not just the file, is what the manifest's
    // `request_body_limit_bytes` bounds. Checking it here fails the delivery
    // with an accurate reason; without it an in-budget file whose multipart
    // fields pushed the body over the declared cap was rejected by policy
    // *after* the coordinator had already committed to the send.
    let request_cap = TELEGRAM_MAX_TRANSFER_BYTES + TELEGRAM_MULTIPART_OVERHEAD_BYTES;
    if body.len() as u64 > request_cap {
        return Err("telegram attachment exceeds the channel size limit".to_string());
    }

    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Post,
        url: format!(
            "https://{TELEGRAM_API_HOST}/bot{{{TELEGRAM_TOKEN_PLACEHOLDER}}}/sendDocument"
        ),
        headers: vec![(
            "content-type".to_string(),
            format!("multipart/form-data; boundary={boundary}"),
        )],
        body: Some(body),
        credential: bot_token_handle(),
        body_credentials: Vec::new(),
    })
}

/// Random multipart boundary, verified absent from the payload exactly once.
///
/// The payload is attacker-authored (an inbound attachment can be landed in
/// the workspace and later referenced by a reply), so the candidate must not
/// be derivable from the bytes: a predictable candidate lets a sender pad a
/// file with collisions and force an unbounded number of full-payload scans.
/// A v4 UUID makes a collision negligible, so one scan settles it.
pub(crate) fn multipart_boundary(bytes: &[u8]) -> Result<String, String> {
    let boundary = format!("ironclaw-telegram-{}", uuid::Uuid::new_v4().simple());
    if bytes
        .windows(boundary.len())
        .any(|window| window == boundary.as_bytes())
    {
        return Err("telegram multipart boundary could not be generated".to_string());
    }
    Ok(boundary)
}

fn push_text(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    );
}

fn safe_filename(filename: Option<&str>) -> String {
    let sanitized: String = filename
        .unwrap_or("attachment.bin")
        .chars()
        .take(128)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() || matches!(sanitized.as_str(), "." | "..") {
        "attachment.bin".to_string()
    } else {
        sanitized
    }
}

fn safe_mime(mime_type: &str) -> &str {
    let valid = !mime_type.is_empty()
        && mime_type.len() <= 127
        && mime_type.is_ascii()
        && mime_type == mime_type.to_ascii_lowercase()
        && mime_type
            .split_once('/')
            .is_some_and(|(kind, subtype)| !kind.is_empty() && !subtype.is_empty())
        && mime_type.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '.' | '+' | '-')
        });
    if valid {
        mime_type
    } else {
        "application/octet-stream"
    }
}
