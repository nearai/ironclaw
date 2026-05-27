//! Derive a short sidebar title from a free-form user message.
//!
//! The Reborn v2 WebUI sidebar falls back to `Thread <uuid_prefix>`
//! when `SessionThreadRecord.title` is `None`. The product workflow
//! calls this helper on the first inbound user message in a thread to
//! seed a stable, human-readable title.

/// Maximum character (not byte) length of the derived title, including
/// the trailing ellipsis when the source was truncated.
const MAX_CHARS: usize = 60;

/// Derive a short sidebar title from a free-form user message.
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
pub fn derive_title_from_message(message: &str) -> Option<String> {
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
