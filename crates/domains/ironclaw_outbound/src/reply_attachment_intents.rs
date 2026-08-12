//! Run-scoped metadata intents for attaching workspace files to a final reply.
//!
//! This contract deliberately stores only stable scoped paths and bounded file
//! metadata. File bytes remain in the workspace filesystem and provider
//! delivery remains the responsibility of the transport layer.

use async_trait::async_trait;
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_host_api::{ids::RunId, path::ScopedPath, resource::ResourceScope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::OutboundError;

const MAX_REPLY_ATTACHMENT_FILENAME_BYTES: usize = 255;
const MAX_REPLY_ATTACHMENT_MIME_TYPE_BYTES: usize = 127;

/// Opaque model-visible identity for one registered reply attachment.
///
/// The handle is deterministic for a `(run, scoped path)` pair so capability
/// retries and transcript finalization independently derive the same value
/// without persisting a second identifier or exposing the workspace path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ReplyAttachmentHandle(String);

impl ReplyAttachmentHandle {
    pub fn for_run_path(run_id: &RunId, path: &ScopedPath) -> Self {
        let mut digest = Sha256::new();
        digest.update(run_id.as_uuid().as_bytes());
        digest.update([0]);
        digest.update(path.as_str().as_bytes());
        Self(format!("att_{}", hex::encode(digest.finalize())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ReplyAttachmentHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyAttachmentIntent {
    pub path: ScopedPath,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

impl ReplyAttachmentIntent {
    /// Validate that this metadata-only intent is safe and within the shared
    /// per-file budget. Aggregate count/byte limits are enforced by the store.
    pub fn validate(&self) -> Result<(), OutboundError> {
        validate_reply_attachment_intent(self)
    }
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
            return Err(OutboundError::ReplyAttachmentIntentConflict);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_attachment_handle_is_deterministic_opaque_and_path_distinct() {
        let run_id = RunId::new();
        let report = ScopedPath::new("/workspace/report.csv").unwrap();
        let chart = ScopedPath::new("/workspace/chart.png").unwrap();

        let first = ReplyAttachmentHandle::for_run_path(&run_id, &report);
        let retry = ReplyAttachmentHandle::for_run_path(&run_id, &report);
        let other = ReplyAttachmentHandle::for_run_path(&run_id, &chart);
        let other_run = ReplyAttachmentHandle::for_run_path(&RunId::new(), &report);

        assert_eq!(first, retry);
        assert_ne!(first, other);
        assert_ne!(first, other_run);
        assert!(first.as_str().starts_with("att_"));
        assert!(!first.as_str().contains("workspace"));
        assert!(!first.as_str().contains("report"));
    }
}
