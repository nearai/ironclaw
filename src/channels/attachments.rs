//! Shared attachment helpers for channel ingestion and persistence.

/// Maximum decoded size per inline attachment.
pub(crate) const MAX_INLINE_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;
/// Maximum total decoded size across all inline attachments in a message.
pub(crate) const MAX_INLINE_TOTAL_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
/// Maximum number of inline attachments in a single message.
pub(crate) const MAX_INLINE_ATTACHMENTS: usize = 5;
