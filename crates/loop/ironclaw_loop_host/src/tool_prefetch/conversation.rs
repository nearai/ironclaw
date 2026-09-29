//! The conversation text a selection sees: building the bounded
//! [`ConversationContext`] from the thread's user messages, and cutting it
//! into segments for a ranker that reads only the start of a query.
//!
//! The context type is a plain carrier owned by `ironclaw_loop_contracts`;
//! which messages go in, how they are bounded and how they are cut are the
//! host's decisions and live here.

use ironclaw_loop_contracts::{ConversationContext, MAX_CONVERSATION_CONTEXT_BYTES};

/// One user message is cut to this many bytes before a selection sees it. At
/// the default segment size (2 KiB) an opening message is at most eight
/// segments.
pub(crate) const MAX_CONTEXT_MESSAGE_BYTES: usize = 16 * 1_024;

/// The most user messages a context holds, and the most segments
/// [`segments`] returns. A deployment's configured count is at most this.
pub const MAX_CONTEXT_MESSAGES: usize = 64;

/// The smallest segment size [`segments`] cuts to; a smaller request is
/// raised to it.
pub const MIN_CONTEXT_SEGMENT_BYTES: usize = 128;

/// The largest segment size [`segments`] cuts to; a larger request is
/// lowered to it. Rankers read a few hundred tokens of a query at most, so a
/// larger segment would only be truncated by the ranker.
pub const MAX_CONTEXT_SEGMENT_BYTES: usize = 4 * 1_024;

/// The turn-start context: the opening request alone, trimmed and cut to
/// [`MAX_CONTEXT_MESSAGE_BYTES`]; empty when the request has no text.
pub(crate) fn opening_request(text: &str) -> ConversationContext {
    let text = truncate_to_char_boundary(text.trim(), MAX_CONTEXT_MESSAGE_BYTES);
    ConversationContext::new(
        Some(text)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .into_iter()
            .collect(),
    )
}

/// A re-selection's context: the `max_messages` most recent user messages
/// (given newest first), plus the conversation's `first` message when it
/// still fits. Messages are trimmed; blank ones are skipped and do not count.
/// Every message is cut to [`MAX_CONTEXT_MESSAGE_BYTES`]; an older message
/// that would take the total past [`MAX_CONVERSATION_CONTEXT_BYTES`] ends the
/// window, and so does the message count. `first` fits when neither bound is
/// reached with it. `max_messages` is at least 1 and at most
/// [`MAX_CONTEXT_MESSAGES`]. `first` must not also be among `newest_first`.
pub(crate) fn window<'a>(
    newest_first: impl IntoIterator<Item = &'a str>,
    first: Option<&str>,
    max_messages: usize,
) -> ConversationContext {
    let max_messages = max_messages.clamp(1, MAX_CONTEXT_MESSAGES);
    let mut newest: Vec<String> = Vec::new();
    let mut used = 0_usize;
    for text in newest_first.into_iter().map(str::trim) {
        if text.is_empty() {
            continue;
        }
        let kept = truncate_to_char_boundary(text, MAX_CONTEXT_MESSAGE_BYTES);
        if newest.len() >= max_messages
            || used.saturating_add(kept.len()) > MAX_CONVERSATION_CONTEXT_BYTES
        {
            break;
        }
        used += kept.len();
        newest.push(kept.to_string());
    }
    let first = first
        .map(str::trim)
        .filter(|first| !first.is_empty())
        .map(|first| truncate_to_char_boundary(first, MAX_CONTEXT_MESSAGE_BYTES));
    let mut user_messages: Vec<String> = first
        .filter(|first| {
            newest.len() < max_messages
                && used.saturating_add(first.len()) <= MAX_CONVERSATION_CONTEXT_BYTES
        })
        .map(str::to_string)
        .into_iter()
        .collect();
    user_messages.extend(newest.into_iter().rev());
    ConversationContext::new(user_messages)
}

/// The context's messages cut into segments of at most `segment_bytes`
/// each, for a ranker that ranks one segment at a time: newest message
/// first, and a message's segments in reading order, at most `max_segments`
/// in all.
///
/// A message that fits is one segment. A longer one is cut at the last
/// paragraph break that leaves a segment at least half full, else at the
/// last sentence end that does, else at the last whitespace, else at the
/// last character boundary; segments are trimmed and never empty, and a cut
/// never splits a character. `segment_bytes` is held between
/// [`MIN_CONTEXT_SEGMENT_BYTES`] and [`MAX_CONTEXT_SEGMENT_BYTES`], and
/// `max_segments` between 1 and [`MAX_CONTEXT_MESSAGES`].
pub(crate) fn segments(
    context: &ConversationContext,
    segment_bytes: usize,
    max_segments: usize,
) -> Vec<&str> {
    let budget = segment_bytes.clamp(MIN_CONTEXT_SEGMENT_BYTES, MAX_CONTEXT_SEGMENT_BYTES);
    let max_segments = max_segments.clamp(1, MAX_CONTEXT_MESSAGES);
    let mut segments = Vec::new();
    for message in context.user_messages().iter().rev() {
        let mut rest = message.trim();
        while !rest.is_empty() && segments.len() < max_segments {
            let cut = segment_cut(rest, budget);
            let (head, tail) = rest.split_at(cut);
            let head = head.trim_end();
            if !head.is_empty() {
                segments.push(head);
            }
            rest = tail.trim_start();
        }
    }
    segments
}

/// Where the next segment of `text` ends: a byte offset in `1..=budget` (or
/// the length of the first character, when that alone is larger) on a
/// character boundary. See [`segments`] for the order of preference.
fn segment_cut(text: &str, budget: usize) -> usize {
    if text.len() <= budget {
        return text.len();
    }
    let window = truncate_to_char_boundary(text, budget);
    let Some(first) = text.chars().next() else {
        return 0;
    };
    if window.is_empty() {
        return first.len_utf8();
    }
    let half = window.len() / 2;
    if let Some(index) = window.rfind("\n\n").filter(|index| *index >= half) {
        return index + 2;
    }
    // A sentence end: terminal punctuation or a line break followed by
    // whitespace (the character after the window counts).
    let sentence_end = window
        .char_indices()
        .rev()
        .filter(|(_, character)| matches!(character, '.' | '!' | '?' | '\n'))
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|end| *end >= half)
        .find(|end| text[*end..].chars().next().is_some_and(char::is_whitespace));
    if let Some(end) = sentence_end {
        return end;
    }
    match window.rfind(char::is_whitespace) {
        Some(index) if index > 0 => index,
        _ => window.len(),
    }
}

fn truncate_to_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
