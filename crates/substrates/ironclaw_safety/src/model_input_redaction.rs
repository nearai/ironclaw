use std::{ops::Range, sync::LazyLock};

use regex::Regex;

use crate::{LeakDetector, LeakPatternClass};

const REDACTED_SECRET: &str = "[REDACTED_SECRET]";
const REDACTED_HOST_PATH: &str = "[REDACTED_HOST_PATH]";
const MAX_JSON_REDACTION_DEPTH: usize = 16;

static LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);
static LABELED_SECRET_PATTERNS: LazyLock<Result<Vec<Regex>, regex::Error>> = LazyLock::new(|| {
    [
        concat!(
            r"(?i)\b(?:access[ _-]?token|api[ _-]?key|api[ _-]?secret|client[ _-]?secret|",
            r"password|passwd|secret[ _-]?(?:key|token)|shared[ _-]?secret)\b",
            r#"[\"'`]?"#,
            r"(?:\s*(?::|=)\s*|\s+is\s+set\s+to\s+|\s+(?:is|was|equals)\s+)",
            r"(?:(?:token|value)\s+)?",
            r"(?P<value>[^\s,;]+)"
        ),
        concat!(
            r#"(?i)\bauthorization\b[\"'`]?\s*(?::|=)?\s*[\"'`]?"#,
            r"(?:basic|bearer|digest|negotiate|oauth|token)\s+",
            r"(?:(?:token|value)\s+)?(?P<value>[^\s,;]+)"
        ),
    ]
    .into_iter()
    .map(Regex::new)
    .collect()
});

/// A model-visible text field after deterministic secret redaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInputRedaction {
    text: String,
    redaction_count: usize,
}

impl ModelInputRedaction {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    pub fn redaction_count(&self) -> usize {
        self.redaction_count
    }

    pub fn was_modified(&self) -> bool {
        self.redaction_count > 0
    }
}

/// Redact detected secret values while preserving the surrounding model context.
///
/// This is deliberately infallible for valid Rust strings. Known credential
/// formats are handled by [`LeakDetector`]; the label-aware pass catches weak
/// values that have no distinctive shape, such as `password: letmein`.
pub fn redact_model_input_text(value: &str) -> ModelInputRedaction {
    redact_model_input_text_at_depth(value, 0)
}

/// Redact a provider-bound URL without rewriting inline `data:` image bytes.
pub fn redact_model_input_url(value: &str) -> ModelInputRedaction {
    if value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
    {
        return ModelInputRedaction {
            text: value.to_string(),
            redaction_count: 0,
        };
    }

    let Ok(mut parsed) = url::Url::parse(value) else {
        return redact_model_input_text(value);
    };
    let mut redaction_count = 0usize;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        if parsed.set_username("").is_err() || parsed.set_password(None).is_err() {
            return ModelInputRedaction {
                text: REDACTED_SECRET.to_string(),
                redaction_count: 1,
            };
        }
        redaction_count = redaction_count.saturating_add(1);
    }
    if parsed.query().is_some() {
        let mut query_redaction_count = 0usize;
        let pairs = parsed
            .query_pairs()
            .map(|(name, value)| {
                if crate::credential_detect::query_param_is_credential(&name)
                    && !is_redaction_marker(&value)
                {
                    query_redaction_count = query_redaction_count.saturating_add(1);
                    (name.into_owned(), REDACTED_SECRET.to_string())
                } else {
                    (name.into_owned(), value.into_owned())
                }
            })
            .collect::<Vec<_>>();
        if query_redaction_count > 0 {
            parsed.query_pairs_mut().clear();
            for (name, value) in pairs {
                parsed.query_pairs_mut().append_pair(&name, &value);
            }
            redaction_count = redaction_count.saturating_add(query_redaction_count);
        }
    }
    if let Some(fragment) = parsed.fragment() {
        let mut fragment_redaction_count = 0usize;
        let pairs = url::form_urlencoded::parse(fragment.as_bytes())
            .map(|(name, value)| {
                if crate::credential_detect::query_param_is_credential(&name)
                    && !is_redaction_marker(&value)
                {
                    fragment_redaction_count = fragment_redaction_count.saturating_add(1);
                    (name.into_owned(), REDACTED_SECRET.to_string())
                } else {
                    (name.into_owned(), value.into_owned())
                }
            })
            .collect::<Vec<_>>();
        if fragment_redaction_count > 0 {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer.extend_pairs(pairs);
            parsed.set_fragment(Some(&serializer.finish()));
            redaction_count = redaction_count.saturating_add(fragment_redaction_count);
        }
    }

    ModelInputRedaction {
        text: if redaction_count == 0 {
            value.to_string()
        } else {
            parsed.to_string().replace("://@", "://")
        },
        redaction_count,
    }
}

fn redact_plain_text(value: &str) -> ModelInputRedaction {
    let Ok(patterns) = LABELED_SECRET_PATTERNS.as_ref() else {
        // The expressions are compile-time literals, but a future edit can still
        // make one invalid. Fail closed at the model boundary without rejecting
        // the turn or exposing the input.
        return ModelInputRedaction {
            text: REDACTED_SECRET.to_string(),
            redaction_count: 1,
        };
    };
    // A shell can render file bytes as a character dump (`od -c`), placing
    // whitespace between every character. The model can reconstruct that
    // representation, but the ordinary label patterns cannot. Decode only
    // offset-prefixed dump lines for detection; when the reconstructed text
    // contains a credential assignment, fail closed for this encoded field.
    // Returning the decoded text would itself expose the value, and mapping a
    // decoded byte range back across line offsets is needlessly fragile.
    if character_dump_contains_labeled_secret(value, patterns) {
        return ModelInputRedaction {
            text: REDACTED_SECRET.to_string(),
            redaction_count: 1,
        };
    }
    let labeled_ranges = labeled_secret_ranges(value, patterns);
    let labeled_redacted = apply_redactions(value, &labeled_ranges, REDACTED_SECRET);
    let host_path_ranges = host_path_ranges(&labeled_redacted);
    let path_redacted = apply_redactions(&labeled_redacted, &host_path_ranges, REDACTED_HOST_PATH);

    // The shared detector's warn-only entropy heuristic deliberately flags
    // standalone 64-character hex strings. That is useful at an exfiltration
    // boundary, but too ambiguous for model input: ordinary SHA-256 fingerprints
    // would be rewritten on every turn. Strong detector findings still redact,
    // and a hex value after a credential label was already removed above.
    let detector_ranges = LEAK_DETECTOR
        .scan(&path_redacted)
        .matches
        .into_iter()
        .filter(|finding| finding.pattern_class() != LeakPatternClass::AmbiguousHexDigest)
        .map(|finding| finding.location)
        .collect::<Vec<_>>();
    let detector_ranges = merge_ranges(detector_ranges);
    let redaction_count = labeled_ranges
        .len()
        .saturating_add(host_path_ranges.len())
        .saturating_add(detector_ranges.len());
    let text = apply_redactions(&path_redacted, &detector_ranges, REDACTED_SECRET);
    ModelInputRedaction {
        text,
        redaction_count,
    }
}

fn character_dump_contains_labeled_secret(value: &str, patterns: &[Regex]) -> bool {
    let mut decoded = String::with_capacity(value.len());
    let mut dump_lines = 0usize;
    let mut decoded_tokens = 0usize;

    for line in value.lines() {
        let mut tokens = line.split_whitespace();
        let Some(offset) = tokens.next() else {
            continue;
        };
        if !is_character_dump_offset(offset) {
            continue;
        }
        dump_lines = dump_lines.saturating_add(1);
        for token in tokens {
            let Some(character) = decode_character_dump_token(token) else {
                continue;
            };
            decoded.push(character);
            decoded_tokens = decoded_tokens.saturating_add(1);
        }
    }

    dump_lines > 0 && decoded_tokens >= 8 && !labeled_secret_ranges(&decoded, patterns).is_empty()
}

fn is_character_dump_offset(token: &str) -> bool {
    (7..=16).contains(&token.len()) && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decode_character_dump_token(token: &str) -> Option<char> {
    let mut characters = token.chars();
    let first = characters.next()?;
    if characters.next().is_none() {
        return Some(first);
    }
    match token {
        r"\n" => Some('\n'),
        r"\r" => Some('\r'),
        r"\t" => Some('\t'),
        r"\0" => Some('\0'),
        r"\\" => Some('\\'),
        _ => {
            let octal = token.strip_prefix('\\').unwrap_or(token);
            if octal.len() != 3 || !octal.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
                return None;
            }
            u8::from_str_radix(octal, 8).ok().map(char::from)
        }
    }
}

// Tool results often wrap their model-visible text in one or more JSON string
// fields. Scan those fields after decoding so escaped labels such as
// `\"password\": \"value\"` cannot bypass the ordinary label-aware pass.
fn redact_model_input_text_at_depth(value: &str, encoded_depth: usize) -> ModelInputRedaction {
    if encoded_depth >= MAX_JSON_REDACTION_DEPTH
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(value)
    {
        let requires_fail_closed_redaction = match json {
            serde_json::Value::String(text) => !is_redaction_marker(&text),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => true,
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
                false
            }
        };
        if requires_fail_closed_redaction {
            return ModelInputRedaction {
                text: REDACTED_SECRET.to_string(),
                redaction_count: 1,
            };
        }
    }
    let mut json_redaction_count = 0usize;
    let mut json_redacted = None;
    if encoded_depth < MAX_JSON_REDACTION_DEPTH
        && let Ok(mut json) = serde_json::from_str::<serde_json::Value>(value)
    {
        json_redaction_count =
            redact_json_string_values(&mut json, encoded_depth.saturating_add(1));
        if json_redaction_count > 0 {
            match serde_json::to_string(&json) {
                Ok(text) => json_redacted = Some(text),
                Err(_) => {
                    return ModelInputRedaction {
                        text: REDACTED_SECRET.to_string(),
                        redaction_count: json_redaction_count.saturating_add(1),
                    };
                }
            }
        }
    }

    let plain = redact_plain_text(json_redacted.as_deref().unwrap_or(value));
    ModelInputRedaction {
        text: plain.text,
        redaction_count: json_redaction_count.saturating_add(plain.redaction_count),
    }
}

fn redact_json_string_values(value: &mut serde_json::Value, encoded_depth: usize) -> usize {
    match value {
        serde_json::Value::String(text) => {
            let redaction = redact_model_input_text_at_depth(text, encoded_depth);
            let count = redaction.redaction_count();
            if count > 0 {
                *text = redaction.into_text();
            }
            count
        }
        serde_json::Value::Array(values) => values.iter_mut().fold(0usize, |count, value| {
            count.saturating_add(redact_json_string_values(value, encoded_depth))
        }),
        serde_json::Value::Object(values) => values.values_mut().fold(0usize, |count, value| {
            count.saturating_add(redact_json_string_values(value, encoded_depth))
        }),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 0,
    }
}

fn labeled_secret_ranges(value: &str, patterns: &[Regex]) -> Vec<Range<usize>> {
    let mut ranges = patterns
        .iter()
        .flat_map(|pattern| pattern.captures_iter(value))
        .filter_map(|captures| {
            credential_candidate_range(
                value,
                captures.get(0)?.range(),
                captures.name("value")?.range(),
            )
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    merge_ranges(ranges)
}

fn credential_candidate_range(
    value: &str,
    full_match: Range<usize>,
    candidate: Range<usize>,
) -> Option<Range<usize>> {
    let candidate_text = value.get(candidate.clone())?;
    if let Some(quote) = candidate_text.chars().next().filter(|ch| is_quote(*ch)) {
        let start = candidate.start.saturating_add(quote.len_utf8());
        let end = quoted_value_end(value, start, quote);
        return validated_candidate_range(value, start..end);
    }

    // Authorization commonly quotes the whole scheme/value pair:
    // `Authorization: "Bearer value with spaces"`. The opening quote is
    // before the scheme and therefore outside the `value` capture. Recognize
    // that narrow prefix shape, then extend through its matching close quote.
    let prefix = value.get(full_match.start..candidate.start)?;
    if let Some((quote_offset, quote)) = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| is_quote(*character))
    {
        let after_quote = prefix.get(quote_offset.saturating_add(quote.len_utf8())..)?;
        if starts_with_authorization_scheme(after_quote) {
            let end = quoted_value_end(value, candidate.start, quote);
            return validated_candidate_range(value, candidate.start..end);
        }
    }

    trimmed_candidate_range(value, candidate)
}

fn quoted_value_end(value: &str, start: usize, quote: char) -> usize {
    closing_quote_offset(value, start, quote).unwrap_or_else(|| {
        value
            .get(start..)
            .and_then(|tail| tail.find('\n'))
            .map(|offset| start.saturating_add(offset))
            .unwrap_or(value.len())
    })
}

fn closing_quote_offset(value: &str, start: usize, quote: char) -> Option<usize> {
    let tail = value.get(start..)?;
    let mut escaped = false;
    for (offset, character) in tail.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
        } else if character == quote {
            return Some(start.saturating_add(offset));
        }
    }
    None
}

fn is_quote(character: char) -> bool {
    matches!(character, '\'' | '"' | '`')
}

fn starts_with_authorization_scheme(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    [
        "basic ",
        "bearer ",
        "digest ",
        "negotiate ",
        "oauth ",
        "token ",
    ]
    .iter()
    .any(|scheme| lowercase.starts_with(scheme))
}

fn validated_candidate_range(value: &str, range: Range<usize>) -> Option<Range<usize>> {
    let candidate = value.get(range.clone())?;
    if range.start >= range.end || is_redaction_marker(candidate) {
        return None;
    }
    Some(range)
}

fn trimmed_candidate_range(value: &str, range: Range<usize>) -> Option<Range<usize>> {
    let candidate = value.get(range.clone())?;
    let trimmed_start = candidate.trim_start_matches(['\'', '"', '`', '(', '[', '{', '<']);
    let start = range.start + candidate.len().saturating_sub(trimmed_start.len());
    let trimmed =
        trimmed_start.trim_end_matches(['\'', '"', '`', '.', ':', '!', '?', ')', ']', '}', '>']);
    let end = start + trimmed.len();
    if start >= end || is_redaction_marker(trimmed) {
        return None;
    }
    Some(start..end)
}

fn is_redaction_marker(value: &str) -> bool {
    let normalized = value.trim_matches(|character| {
        matches!(
            character,
            '\\' | '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
        )
    });
    matches!(
        normalized.to_ascii_lowercase().as_str(),
        "redacted"
            | "redacted_secret"
            | "placeholder"
            | "example"
            | "token"
            | "value"
            | "key"
            | "your-token"
            | "your_token"
    ) || normalized.contains("...")
}

fn merge_ranges(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(previous) if range.start <= previous.end => {
                previous.end = previous.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }
    merged
}

fn host_path_ranges(value: &str) -> Vec<Range<usize>> {
    const PREFIXES: [&str; 6] = [
        "/users/",
        "/home/",
        "/private/",
        "/tmp/", // safety: model-view path literal, not a filesystem temp path.
        "/var/",
        "/etc/",
    ];
    let lowercase = value.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for prefix in PREFIXES {
        let mut cursor = 0usize;
        while let Some(relative_start) = lowercase[cursor..].find(prefix) {
            let start = cursor.saturating_add(relative_start);
            let end = value[start..]
                .char_indices()
                .find(|(_, character)| {
                    character.is_whitespace()
                        || matches!(character, '\'' | '"' | '`' | '<' | '>' | ',' | ';')
                })
                .map(|(offset, _)| start.saturating_add(offset))
                .unwrap_or(value.len());
            let trimmed_end = value[start..end]
                .trim_end_matches(['.', ':', '!', '?', ')', ']', '}'])
                .len()
                .saturating_add(start);
            if start < trimmed_end {
                ranges.push(start..trimmed_end);
            }
            cursor = end.max(start.saturating_add(prefix.len()));
            if cursor >= lowercase.len() {
                break;
            }
        }
    }
    ranges.sort_by_key(|range| range.start);
    merge_ranges(ranges)
}

fn apply_redactions(value: &str, ranges: &[Range<usize>], replacement: &str) -> String {
    if ranges.is_empty() {
        return value.to_string();
    }
    let mut redacted = String::with_capacity(value.len());
    let mut cursor = 0;
    for range in ranges {
        redacted.push_str(&value[cursor..range.start]);
        redacted.push_str(replacement);
        cursor = range.end;
    }
    redacted.push_str(&value[cursor..]);
    redacted
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{MAX_JSON_REDACTION_DEPTH, redact_model_input_text, redact_model_input_url};

    #[test]
    fn redacts_labeled_values_without_dropping_surrounding_context() {
        for (input, secret) in [
            ("password: letmein", "letmein"),
            ("password was hunter2", "hunter2"),
            ("password is set to swordfish", "swordfish"),
            ("api key = abcdef", "abcdef"),
            (
                r#"{"password":"railway-test-fake-neutral-credential-7509"}"#,
                "railway-test-fake-neutral-credential-7509",
            ),
            (
                r#"{"Authorization":"Bearer ghp_structuredsecret123"}"#,
                "ghp_structuredsecret123",
            ),
            ("Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"),
            (
                "Authorization: Bearer token ghp_secretvalue123",
                "ghp_secretvalue123",
            ),
        ] {
            let redaction = redact_model_input_text(input);

            assert!(redaction.was_modified(), "expected redaction for {input:?}");
            assert!(!redaction.text().contains(secret));
            assert!(redaction.text().contains("[REDACTED_SECRET]"));
        }
    }

    #[test]
    fn redacts_complete_quoted_credential_values() {
        for (input, secret, expected) in [
            (
                r#"password="my secret,with;delimiters"; keep=visible"#,
                "my secret,with;delimiters",
                r#"password="[REDACTED_SECRET]"; keep=visible"#,
            ),
            (
                r#"api_key='single quoted,secret;value'; keep=visible"#,
                "single quoted,secret;value",
                "api_key='[REDACTED_SECRET]'; keep=visible",
            ),
            (
                r#"client-secret=`backtick quoted,secret;value`; keep=visible"#,
                "backtick quoted,secret;value",
                "client-secret=`[REDACTED_SECRET]`; keep=visible",
            ),
            (
                r#"password="escaped \"quote\",with;delimiters"; keep=visible"#,
                r#"escaped \"quote\",with;delimiters"#,
                r#"password="[REDACTED_SECRET]"; keep=visible"#,
            ),
            (
                r#"{"password":"json secret,with;delimiters","marker":"visible"}"#,
                "json secret,with;delimiters",
                r#"{"password":"[REDACTED_SECRET]","marker":"visible"}"#,
            ),
            (
                r#"Authorization: "Bearer auth secret,with;delimiters"; keep=visible"#,
                "auth secret,with;delimiters",
                r#"Authorization: "Bearer [REDACTED_SECRET]"; keep=visible"#,
            ),
            (
                r#"Authorization='Basic single quoted,secret;value'; keep=visible"#,
                "single quoted,secret;value",
                "Authorization='Basic [REDACTED_SECRET]'; keep=visible",
            ),
            (
                r#"Authorization=`Token backtick quoted,secret;value`; keep=visible"#,
                "backtick quoted,secret;value",
                "Authorization=`Token [REDACTED_SECRET]`; keep=visible",
            ),
            (
                r#"Authorization: "Bearer escaped \"quote\",with;delimiters"; keep=visible"#,
                r#"escaped \"quote\",with;delimiters"#,
                r#"Authorization: "Bearer [REDACTED_SECRET]"; keep=visible"#,
            ),
            (
                "password=\"unclosed secret,with;delimiters\nkeep=visible",
                "unclosed secret,with;delimiters",
                "password=\"[REDACTED_SECRET]\nkeep=visible",
            ),
        ] {
            let redaction = redact_model_input_text(input);

            assert!(redaction.was_modified(), "expected redaction for {input:?}");
            assert!(
                !redaction.text().contains(secret),
                "quoted secret remained in {input:?}: {:?}",
                redaction.text()
            );
            assert_eq!(redaction.text(), expected);
        }
    }

    #[test]
    fn redacts_credentials_inside_json_encoded_tool_preview() {
        let secret = "railway-test-fake-neutral-credential-encoded";
        let input = serde_json::json!({
            "schema_version": 1,
            "status": "success",
            "detail": {
                "kind": "result_reference",
                "preview": serde_json::json!({
                    "marker": "attachment-context",
                    "password": secret,
                })
                .to_string(),
            },
        })
        .to_string();

        let redaction = redact_model_input_text(&input);
        let repeated = redact_model_input_text(redaction.text());

        assert!(redaction.was_modified());
        assert!(!redaction.text().contains(secret));
        assert!(redaction.text().contains("attachment-context"));
        assert!(redaction.text().contains("[REDACTED_SECRET]"));
        assert_eq!(repeated.text(), redaction.text());
        assert!(
            !repeated.was_modified(),
            "second redaction changed {:?} into {:?}",
            redaction.text(),
            repeated.text()
        );
    }

    #[test]
    fn encoded_json_at_depth_limit_fails_closed() {
        let secret = "encoded-depth-limit-canary";
        let mut encoded = serde_json::json!({"password": secret}).to_string();
        for _ in 0..=MAX_JSON_REDACTION_DEPTH {
            encoded = serde_json::to_string(&encoded).expect("JSON string wrapper");
        }

        let redaction = redact_model_input_text(&encoded);

        assert!(redaction.was_modified());
        assert!(!redaction.text().contains(secret));
        assert!(redaction.text().contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn redacts_character_dump_that_reconstructs_a_labeled_credential() {
        let secret = "never-before-uploaded-canary-character-dump";
        let input = concat!(
            r#"0000000   {  \n   "   m   a   r   k   e   r   "   :   "   s   a   f   e  \n"#,
            "\n",
            r#"0000040   "   ,  \n   "   p   a   s   s   w   o   r   d   "   :   "   n   e   v   e   r   -  \n"#,
            "\n",
            r#"0000100   b   e   f   o   r   e   -   u   p   l   o   a   d   e   d   -  \n"#,
            "\n",
            r#"0000140   c   a   n   a   r   y   -   c   h   a   r   a   c   t   e   r  \n"#,
            "\n",
            r#"0000200   -   d   u   m   p   "  \n   }  \n"#,
            "\n0000211\n",
        );

        let redaction = redact_model_input_text(input);

        assert!(redaction.was_modified());
        assert!(!redaction.text().contains(secret));
        assert_eq!(redaction.text(), "[REDACTED_SECRET]");
    }

    #[test]
    fn keeps_benign_character_dump_without_a_credential_assignment() {
        let input = concat!(
            r#"0000000   S   e   c   r   e   t   a   r   y       o   f       t   h   e  \n"#,
            "\n",
            r#"0000040   T   r   e   a   s   u   r   y  \n"#,
            "\n",
            "0000050\n",
        );

        let redaction = redact_model_input_text(input);

        assert!(!redaction.was_modified());
        assert_eq!(redaction.text(), input);
    }

    #[test]
    fn keeps_security_prose_and_paths_unchanged() {
        for input in [
            "The report documents an authorization flow and API key rotation.",
            "The upstream service returned invalid API key.",
            "surface sha256:269cc57b4d0c4368d8b02738ab709c810adb6212729b24bbdc34efb539a3ed07",
            "password: redacted",
            "password: redacted_secret",
            "password: placeholder",
            "password: example",
            "password: token",
            "password: value",
            "password: key",
            r#"{"password":"example"}"#,
            r#"{"password":"","token":null}"#,
            "Authorization: Bearer your-token",
            "Authorization: Bearer your_token",
            r#"{"Authorization":"Bearer your-token"}"#,
            "password: ghp_abc...xyz",
        ] {
            let redaction = redact_model_input_text(input);

            assert!(
                !redaction.was_modified(),
                "unexpected redaction for {input:?}"
            );
            assert_eq!(redaction.text(), input);
        }
    }

    #[test]
    fn redacts_host_paths_without_rejecting_surrounding_context() {
        let input = "Read /Users/alice/.config/token and /etc/passwd before reviewing report.md.";
        let redaction = redact_model_input_text(input);

        assert_eq!(
            redaction.text(),
            "Read [REDACTED_HOST_PATH] and [REDACTED_HOST_PATH] before reviewing report.md."
        );
        assert_eq!(redaction.redaction_count(), 2);
    }

    #[test]
    fn redacts_url_credentials_but_preserves_data_urls() {
        let secret = "url-query-secret";
        let redaction = redact_model_input_url(&format!(
            "https://user:password@example.test/image.png?size=large&token={secret}#access_token=fragment-secret&state=visible"
        ));

        assert!(redaction.was_modified());
        assert!(!redaction.text().contains(secret));
        assert!(!redaction.text().contains("fragment-secret"));
        assert!(!redaction.text().contains("user:password"));
        assert!(redaction.text().contains("size=large"));
        assert!(redaction.text().contains("state=visible"));
        assert!(redaction.text().contains("REDACTED_SECRET"));

        let data_url = "data:image/png;base64,cGFzc3dvcmQ6IGxldG1laW4=";
        let preserved = redact_model_input_url(data_url);
        assert!(!preserved.was_modified());
        assert_eq!(preserved.text(), data_url);
    }

    #[test]
    fn url_redaction_preserves_valid_remote_paths_but_plain_text_paths_still_redact() {
        let remote = "https://cdn.example.test/users/42/avatar.png";
        let preserved = redact_model_input_url(remote);
        assert!(!preserved.was_modified());
        assert_eq!(preserved.text(), remote);

        let request_line = redact_model_input_url("GET /users/42/avatar.png");
        assert!(request_line.was_modified());
        assert_eq!(request_line.text(), "GET [REDACTED_HOST_PATH]");
    }

    #[test]
    fn labeled_hex_credential_is_redacted_even_though_unlabeled_digest_is_not() {
        let secret = "269cc57b4d0c4368d8b02738ab709c810adb6212729b24bbdc34efb539a3ed07";
        let redaction = redact_model_input_text(&format!("api key: {secret}"));

        assert!(redaction.was_modified());
        assert!(!redaction.text().contains(secret));
    }

    #[test]
    fn redacts_known_detector_patterns_and_is_idempotent() {
        let input = "token: ghp_012345678901234567890123456789012345";
        let once = redact_model_input_text(input);
        let twice = redact_model_input_text(once.text());

        assert!(once.was_modified());
        assert!(
            !once
                .text()
                .contains("ghp_012345678901234567890123456789012345")
        );
        assert_eq!(twice.text(), once.text());
        assert!(!twice.was_modified());
    }

    #[test]
    fn redacts_multibyte_labeled_value_on_valid_utf8_boundaries() {
        let redaction = redact_model_input_text("password: 秘密です; keep this context");

        assert_eq!(
            redaction.text(),
            "password: [REDACTED_SECRET]; keep this context"
        );
    }

    #[test]
    fn large_security_prose_near_miss_stays_bounded() {
        let input = "The API key rotation policy documents authorization flow.\n".repeat(2_000);
        let started = Instant::now();
        let redaction = redact_model_input_text(&input);

        assert!(!redaction.was_modified());
        assert_eq!(redaction.text(), input);
        assert!(
            started.elapsed().as_millis() < crate::REDOS_SCAN_BUDGET_MS,
            "model-input redaction exceeded the catastrophic-backtracking budget"
        );
    }
}
