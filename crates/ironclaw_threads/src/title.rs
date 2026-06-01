//! Derive a short sidebar title from a thread's first user message.
//!
//! `SessionThreadService::list_threads_for_scope` invokes this when
//! `SessionThreadRecord.title` is `None` so the Reborn v2 WebUI
//! sidebar (and any other channel listing threads) can render a
//! stable, human-readable label instead of falling back to the raw
//! thread id. Derivation is read-time only — nothing is persisted
//! back into the thread record, so an explicit creator-supplied
//! title from `EnsureThreadRequest.title` keeps winning whenever it
//! exists.

use crate::contract::{MessageKind, ThreadMessageRecord};

/// Maximum character (not byte) length of the derived title, including
/// the trailing ellipsis when the source was truncated.
const MAX_CHARS: usize = 60;

/// Derive a sidebar title from a thread's transcript: take the
/// lowest-sequence user message and feed its body through
/// [`derive_title_from_message`].
///
/// Returns `None` when the transcript has no user messages, the
/// chosen message has no body, or the body is whitespace-only.
/// Backends implementing `list_threads_for_scope` call this on the
/// per-thread `ThreadMessageRecord` slice when `record.title.is_none()`
/// so the two impls share one definition of "first user message".
///
/// `min_by_key(sequence)` is used instead of `find(...)` so the result
/// does not depend on the caller passing a pre-sorted slice — a
/// `Vec` of messages assembled in arbitrary order still resolves to
/// the earliest user message in the thread.
pub(crate) fn derive_thread_title(messages: &[ThreadMessageRecord]) -> Option<String> {
    messages
        .iter()
        .filter(|m| m.kind == MessageKind::User)
        .min_by_key(|m| m.sequence)
        .and_then(|m| m.content.as_deref())
        .and_then(derive_title_from_message)
}

/// Derive a short sidebar title from a single message body.
///
/// Takes the first non-empty line, trims leading and trailing
/// whitespace, and truncates by character count (not bytes) so
/// multibyte input does not panic. Returns `None` if the message is
/// all whitespace.
///
/// When the trimmed first line exceeds [`MAX_CHARS`], the result is
/// truncated to `MAX_CHARS - 1` characters with a trailing `…`. When
/// it is exactly `MAX_CHARS` characters long the ellipsis is omitted
/// (no truncation occurred).
///
/// Backends should usually call [`derive_thread_title`] rather than
/// this lower-level helper — that wrapper picks the right message
/// from a transcript by itself.
pub(crate) fn derive_title_from_message(message: &str) -> Option<String> {
    let trimmed = message.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut chars = trimmed.chars();
    let mut out: String = chars.by_ref().take(MAX_CHARS - 1).collect();
    if let Some(next) = chars.next() {
        if chars.next().is_none() {
            out.push(next);
        } else {
            out.push('…');
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_for_empty() {
        assert_eq!(derive_title_from_message(""), None);
    }

    #[test]
    fn returns_none_for_whitespace_only() {
        assert_eq!(derive_title_from_message("   \n\t\n  "), None);
    }

    #[test]
    fn takes_first_non_empty_line() {
        assert_eq!(
            derive_title_from_message("\n\n  hello world\nsecond line"),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            derive_title_from_message("   trimmed   "),
            Some("trimmed".to_string())
        );
    }

    #[test]
    fn short_message_passes_through() {
        assert_eq!(
            derive_title_from_message("ok echo"),
            Some("ok echo".to_string())
        );
    }

    #[test]
    fn exact_max_chars_no_ellipsis() {
        let s: String = "a".repeat(MAX_CHARS);
        assert_eq!(derive_title_from_message(&s), Some(s));
    }

    #[test]
    fn over_max_chars_appends_ellipsis() {
        let s: String = "a".repeat(MAX_CHARS + 5);
        let title = derive_title_from_message(&s).unwrap();
        assert_eq!(title.chars().count(), MAX_CHARS);
        assert!(title.ends_with('…'));
        assert!(title.starts_with(&"a".repeat(MAX_CHARS - 1)));
    }

    #[test]
    fn multibyte_chars_do_not_panic() {
        let s: String = "你好".repeat(40);
        let title = derive_title_from_message(&s).unwrap();
        assert!(title.chars().count() <= MAX_CHARS);
        assert!(title.ends_with('…'));
    }
}
