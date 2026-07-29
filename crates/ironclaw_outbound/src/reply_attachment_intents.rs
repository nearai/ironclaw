//! Run-scoped metadata intents for attaching workspace files to a final reply.
//!
//! This contract deliberately stores only stable scoped paths and bounded file
//! metadata. File bytes remain in the workspace filesystem and provider
//! delivery remains the responsibility of the transport layer.

use async_trait::async_trait;
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_host_api::{ResourceScope, RunId, ScopedPath};
use serde::{Deserialize, Serialize};

use crate::OutboundError;

const MAX_REPLY_ATTACHMENT_FILENAME_BYTES: usize = 255;
const MAX_REPLY_ATTACHMENT_MIME_TYPE_BYTES: usize = 127;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyAttachmentIntent {
    pub path: ScopedPath,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[async_trait]
pub trait ReplyAttachmentIntentPort: Send + Sync {
    async fn register(
        &self,
        scope: &ResourceScope,
        run_id: &RunId,
        intent: ReplyAttachmentIntent,
    ) -> Result<(), OutboundError>;

    async fn seal(
        &self,
        scope: &ResourceScope,
        run_id: &RunId,
    ) -> Result<Vec<ReplyAttachmentIntent>, OutboundError>;
}

pub(crate) fn validate_reply_attachment_intent(
    intent: &ReplyAttachmentIntent,
) -> Result<(), OutboundError> {
    let Some(relative_path) = intent.path.as_str().strip_prefix("/workspace/") else {
        return Err(OutboundError::InvalidRequest {
            reason: "reply attachment path must be inside /workspace",
        });
    };
    if relative_path.is_empty() {
        return Err(OutboundError::InvalidRequest {
            reason: "reply attachment path must name a workspace file",
        });
    }
    if !is_safe_filename(&intent.filename) {
        return Err(OutboundError::InvalidRequest {
            reason: "reply attachment filename is invalid",
        });
    }
    if !is_valid_mime_type(&intent.mime_type) {
        return Err(OutboundError::InvalidRequest {
            reason: "reply attachment MIME type is invalid",
        });
    }
    if intent.size_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 {
        return Err(OutboundError::ReplyAttachmentIntentLimitExceeded);
    }
    Ok(())
}

pub(crate) fn validate_reply_attachment_intents(
    intents: &[ReplyAttachmentIntent],
) -> Result<(), OutboundError> {
    if intents.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(OutboundError::ReplyAttachmentIntentLimitExceeded);
    }

    let mut total_bytes = 0_u64;
    for (index, intent) in intents.iter().enumerate() {
        validate_reply_attachment_intent(intent)?;
        if intents[..index]
            .iter()
            .any(|existing| existing.path == intent.path)
        {
            return Err(OutboundError::Serialization);
        }
        total_bytes = total_bytes
            .checked_add(intent.size_bytes)
            .ok_or(OutboundError::ReplyAttachmentIntentLimitExceeded)?;
    }
    if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes as u64 {
        return Err(OutboundError::ReplyAttachmentIntentLimitExceeded);
    }
    Ok(())
}

fn is_safe_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= MAX_REPLY_ATTACHMENT_FILENAME_BYTES
        && filename != "."
        && filename != ".."
        && !filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
}

fn is_valid_mime_type(mime_type: &str) -> bool {
    if mime_type.is_empty() || mime_type.len() > MAX_REPLY_ATTACHMENT_MIME_TYPE_BYTES {
        return false;
    }
    let Some((top_level, subtype)) = mime_type.split_once('/') else {
        return false;
    };
    !top_level.is_empty()
        && !subtype.is_empty()
        && !subtype.contains('/')
        && top_level.chars().all(is_mime_token_character)
        && subtype.chars().all(is_mime_token_character)
}

fn is_mime_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(
            character,
            '!' | '#' | '$' | '&' | '^' | '_' | '.' | '+' | '-'
        )
}
