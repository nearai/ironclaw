//! Extract `/server:prompt-name [key=value ...]` mentions from a user message.
//!
//! Mirrors `crates/ironclaw_skills/src/selector.rs::extract_skill_mentions` —
//! same boundary rule, same reverse-order replacement pattern used by the
//! dispatcher to rewrite the message in place. Diverges in two ways:
//!
//! 1. Requires a literal `:` between the server name and the prompt name.
//!    A bare `/foo` is a skill mention, not a prompt mention, so the two
//!    namespaces can co-exist without parser coupling.
//! 2. Consumes an optional trailing `key=value [key=value ...]` argument
//!    list. The arg tail stops at the first non-`key=value` token.

use serde_json::{Map, Value};

/// A single `/server:prompt-name [args]` mention found in a message.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptMention {
    /// MCP server name, preserved exactly as authored. Resolvers
    /// downstream fall back to a case-insensitive lookup
    /// (`McpClientStore::get_ci`) because `McpServerName` permits
    /// uppercase and config casing is not canonicalised.
    pub server: String,
    /// Prompt name, exactly as the server declared it.
    pub prompt: String,
    /// Parsed `key=value` tail. Every value is a string — typed coercion
    /// is a follow-up (see plan's "Out of scope").
    pub arguments: Map<String, Value>,
    /// Byte offsets `[start, end)` in the original message. The
    /// substring is always `&message[span.0..span.1]`.
    pub span: (usize, usize),
}

/// Return every prompt mention in source order, with non-overlapping spans.
///
/// Non-matches (`/skill-name` without a colon, malformed tokens, etc.) pass
/// through as literal text — never fails the parse. Callers replace the
/// spans in reverse order to preserve byte indices.
pub fn extract_prompt_mentions(message: &str) -> Vec<PromptMention> {
    // Fast path: messages without `/` can't contain a mention. memchr
    // makes this a single tight loop over the whole message instead of
    // the branch-heavy byte walk below.
    if !message.contains('/') {
        return Vec::new();
    }
    let bytes = message.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'/' {
            i += 1;
            continue;
        }
        // Only match at a word boundary (start, whitespace, or one of the
        // same punctuation markers used by the skill extractor — keeps the
        // two surfaces symmetrical).
        let is_boundary = i == 0
            || bytes[i - 1] == b' '
            || bytes[i - 1] == b'\n'
            || bytes[i - 1] == b'\t'
            || bytes[i - 1] == b'"'
            || bytes[i - 1] == b'(';
        if !is_boundary {
            i += 1;
            continue;
        }

        let mention_start = i;
        let server_start = i + 1;
        let mut j = server_start;
        while j < bytes.len() && is_server_ident_byte(bytes[j]) {
            j += 1;
        }
        if j == server_start || j >= bytes.len() || bytes[j] != b':' {
            i += 1;
            continue;
        }
        let server = message[server_start..j].to_string();
        // Skip past the colon.
        let prompt_start = j + 1;
        let mut k = prompt_start;
        while k < bytes.len() && is_prompt_ident_byte(bytes[k]) {
            k += 1;
        }
        if k == prompt_start {
            i += 1;
            continue;
        }
        let prompt = message[prompt_start..k].to_string();

        // Argument tail: zero or more space-separated `key=value` tokens.
        // A token that doesn't parse as `key=value` ends the mention; what
        // follows stays as literal text after the rendered block.
        let mut arguments: Map<String, Value> = Map::new();
        let mut end = k;
        loop {
            // Require exactly one space before each arg token.
            if end >= bytes.len() || bytes[end] != b' ' {
                break;
            }
            let token_start = end + 1;
            let Some((token_end, key, value)) = try_parse_kv(message, token_start) else {
                break;
            };
            arguments.insert(key, Value::String(value));
            end = token_end;
        }

        out.push(PromptMention {
            server,
            prompt,
            arguments,
            span: (mention_start, end),
        });
        i = end;
    }
    out
}

fn is_server_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_prompt_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
}

fn is_key_start_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_key_cont_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Attempt to parse a single `key=value` token starting at `start`. Returns
/// `(end_offset_exclusive, key, value)` on success. `value` is either a bare
/// token (no whitespace) or a double-quoted string with backslash escapes
/// `\"` and `\\`.
fn try_parse_kv(message: &str, start: usize) -> Option<(usize, String, String)> {
    let bytes = message.as_bytes();
    if start >= bytes.len() || !is_key_start_byte(bytes[start]) {
        return None;
    }
    let mut k = start + 1;
    while k < bytes.len() && is_key_cont_byte(bytes[k]) {
        k += 1;
    }
    if k >= bytes.len() || bytes[k] != b'=' {
        return None;
    }
    let key = message[start..k].to_string();
    let value_start = k + 1;
    if value_start >= bytes.len() {
        // `key=` with no value — treat as non-match so we don't silently
        // drop a typo into an empty string.
        return None;
    }
    if bytes[value_start] == b'"' {
        parse_quoted_value(message, value_start).map(|(end, value)| (end, key, value))
    } else {
        parse_bare_value(message, value_start).map(|(end, value)| (end, key, value))
    }
}

fn parse_bare_value(message: &str, start: usize) -> Option<(usize, String)> {
    let bytes = message.as_bytes();
    let mut j = start;
    while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    if j == start {
        return None;
    }
    Some((j, message[start..j].to_string()))
}

/// Parse a double-quoted value starting at `start` (pointing at the opening
/// `"`). Walks by character, not byte, so multi-byte UTF-8 content (non-ASCII
/// titles, paths) round-trips unchanged. Supports `\"` and `\\` escapes;
/// other backslash sequences are preserved verbatim. Returns `(end_offset,
/// value)` where `end_offset` points one byte past the closing quote.
fn parse_quoted_value(message: &str, start: usize) -> Option<(usize, String)> {
    debug_assert!(message.as_bytes()[start] == b'"');
    let mut buf = String::new();
    // `char_indices` iterates over Unicode scalar values with their byte
    // offsets, preserving multi-byte codepoints as whole units.
    let mut iter = message[start + 1..].char_indices();
    while let Some((offset, c)) = iter.next() {
        let absolute = start + 1 + offset;
        match c {
            '"' => return Some((absolute + 1, buf)),
            '\n' => return None, // unterminated
            '\\' => match iter.next() {
                Some((_, '"')) => buf.push('"'),
                Some((_, '\\')) => buf.push('\\'),
                Some((_, other)) => {
                    buf.push('\\');
                    buf.push(other);
                }
                None => return None, // dangling backslash before EOF
            },
            other => buf.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_mention_at_start() {
        let m = extract_prompt_mentions("/notion:search tell me about docs");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].server, "notion");
        assert_eq!(m[0].prompt, "search");
        assert!(m[0].arguments.is_empty());
        assert_eq!(m[0].span, (0, "/notion:search".len()));
    }

    #[test]
    fn mention_mid_sentence_respects_word_boundary() {
        let m = extract_prompt_mentions("run /github:list-pr please");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].server, "github");
        assert_eq!(m[0].prompt, "list-pr");
    }

    #[test]
    fn no_match_when_not_at_word_boundary() {
        let m = extract_prompt_mentions("foo/bar:baz");
        assert!(m.is_empty());
    }

    #[test]
    fn skill_style_bare_name_is_not_a_prompt_mention() {
        // `/github` has no `:` — it's a skill mention, not a prompt mention.
        let m = extract_prompt_mentions("try /github for me");
        assert!(m.is_empty());
    }

    #[test]
    fn server_half_preserves_case() {
        // `McpServerName`'s allowlist permits uppercase letters, and
        // stored config rows may use mixed case. The parser preserves
        // the substring as authored; case-insensitive resolution happens
        // at `ExtensionManager::get_prompt_for_user`.
        let m = extract_prompt_mentions("/Notion:create-page");
        assert_eq!(m[0].server, "Notion");
    }

    #[test]
    fn prompt_half_is_case_sensitive() {
        let m = extract_prompt_mentions("/notion:CreatePage");
        assert_eq!(m[0].prompt, "CreatePage");
    }

    #[test]
    fn empty_server_or_prompt_rejects() {
        assert!(extract_prompt_mentions("/:foo").is_empty());
        assert!(extract_prompt_mentions("/notion:").is_empty());
        assert!(extract_prompt_mentions("/notion").is_empty());
    }

    #[test]
    fn single_arg() {
        let msg = "/notion:search query=docs";
        let m = extract_prompt_mentions(msg);
        assert_eq!(m[0].arguments.len(), 1);
        assert_eq!(m[0].arguments["query"], Value::String("docs".into()));
        // span covers the entire mention including the consumed arg
        assert_eq!(&msg[m[0].span.0..m[0].span.1], msg);
    }

    #[test]
    fn multiple_args() {
        let m = extract_prompt_mentions("/notion:create-page parent_id=abc title=foo");
        assert_eq!(m[0].arguments.len(), 2);
        assert_eq!(m[0].arguments["parent_id"], Value::String("abc".into()));
        assert_eq!(m[0].arguments["title"], Value::String("foo".into()));
    }

    #[test]
    fn quoted_value_with_spaces() {
        let m = extract_prompt_mentions(r#"/notion:create-page title="Q2 Review""#);
        assert_eq!(m[0].arguments["title"], Value::String("Q2 Review".into()));
    }

    #[test]
    fn quoted_value_with_escaped_quote() {
        let m = extract_prompt_mentions(r#"/notion:create-page title="a \"b\" c""#);
        assert_eq!(m[0].arguments["title"], Value::String(r#"a "b" c"#.into()));
    }

    #[test]
    fn arg_list_stops_at_first_non_kv_token() {
        let m = extract_prompt_mentions("/notion:search query=docs please read");
        assert_eq!(m[0].arguments.len(), 1);
        assert_eq!(m[0].arguments["query"], Value::String("docs".into()));
        // The mention span ends before `please`, so the dispatcher will
        // leave it as literal text after the rendered block.
        let end = m[0].span.1;
        let tail = &"/notion:search query=docs please read"[end..];
        assert_eq!(tail, " please read");
    }

    #[test]
    fn multiple_mentions_preserved_in_order() {
        let msg = "first /a:one then /b:two done";
        let m = extract_prompt_mentions(msg);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].server, "a");
        assert_eq!(m[1].server, "b");
        // Non-overlapping spans.
        assert!(m[0].span.1 <= m[1].span.0);
    }

    #[test]
    fn key_must_start_with_letter_or_underscore() {
        // `1foo=bar` is not a valid key — the arg tail stops and the
        // mention ends at the prompt name.
        let m = extract_prompt_mentions("/notion:search 1foo=bar");
        assert!(m[0].arguments.is_empty());
        let end = m[0].span.1;
        assert_eq!(&"/notion:search 1foo=bar"[end..], " 1foo=bar");
    }

    #[test]
    fn bare_equals_with_no_value_rejects_the_token() {
        let m = extract_prompt_mentions("/notion:search key= ");
        assert!(m[0].arguments.is_empty());
    }

    #[test]
    fn unterminated_quoted_value_rejects_the_token() {
        let m = extract_prompt_mentions(r#"/notion:search title="oops"#);
        assert!(m[0].arguments.is_empty());
    }

    #[test]
    fn quoted_value_preserves_multibyte_utf8() {
        // Regression: the original implementation cast each non-ASCII
        // byte to `char`, which mangled multi-byte UTF-8 sequences. An
        // accented character like `à` (two bytes `0xC3 0xA0`) would
        // decode as two separate Latin-1 chars instead of one code
        // point.
        let m = extract_prompt_mentions(r#"/notion:create-page title="café à Paris""#);
        assert_eq!(
            m[0].arguments["title"],
            Value::String("café à Paris".into()),
        );
    }

    #[test]
    fn quoted_value_preserves_emoji() {
        // 4-byte UTF-8 sequence. `as char` on any of the four individual
        // bytes would produce garbage; char-iteration keeps the emoji
        // intact.
        let m = extract_prompt_mentions(r#"/notion:post body="ship it 🚀""#);
        assert_eq!(m[0].arguments["body"], Value::String("ship it 🚀".into()),);
    }

    #[test]
    fn quoted_value_preserves_cjk() {
        let m = extract_prompt_mentions(r#"/notion:page title="日本語タイトル""#);
        assert_eq!(
            m[0].arguments["title"],
            Value::String("日本語タイトル".into()),
        );
    }

    #[test]
    fn escape_before_multibyte_char_preserves_codepoint() {
        // `\X` where X is non-ASCII should pass through as `\` + the
        // full codepoint, not `\` + the first byte.
        let m = extract_prompt_mentions(r#"/notion:page title="a \é b""#);
        assert_eq!(m[0].arguments["title"], Value::String(r#"a \é b"#.into()),);
    }

    #[test]
    fn span_ends_on_char_boundary_after_multibyte_value() {
        // The returned span must be on a UTF-8 boundary so the dispatcher's
        // `replace_range` call won't panic.
        let msg = r#"/notion:page title="café" trailing"#;
        let m = extract_prompt_mentions(msg);
        let (start, end) = m[0].span;
        assert!(msg.is_char_boundary(start));
        assert!(msg.is_char_boundary(end));
        assert_eq!(&msg[end..], " trailing");
    }
}
