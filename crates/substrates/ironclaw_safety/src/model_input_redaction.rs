use std::{ops::Range, sync::LazyLock};

use regex::Regex;

use crate::LeakDetector;

const REDACTED_SECRET: &str = "[REDACTED_SECRET]";
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
    let labeled_ranges = labeled_secret_ranges(value, patterns);
    let labeled_redacted = apply_redactions(value, &labeled_ranges);

    // The shared detector's warn-only entropy heuristic deliberately flags
    // standalone 64-character hex strings. That is useful at an exfiltration
    // boundary, but too ambiguous for model input: ordinary SHA-256 fingerprints
    // would be rewritten on every turn. Strong detector findings still redact,
    // and a hex value after a credential label was already removed above.
    let detector_ranges = LEAK_DETECTOR
        .scan(&labeled_redacted)
        .matches
        .into_iter()
        .filter(|finding| finding.pattern_name != "high_entropy_hex")
        .map(|finding| finding.location)
        .collect::<Vec<_>>();
    let detector_ranges = merge_ranges(detector_ranges);
    let redaction_count = labeled_ranges.len().saturating_add(detector_ranges.len());
    let text = apply_redactions(&labeled_redacted, &detector_ranges);
    ModelInputRedaction {
        text,
        redaction_count,
    }
}

// Tool results often wrap their model-visible text in one or more JSON string
// fields. Scan those fields after decoding so escaped labels such as
// `\"password\": \"value\"` cannot bypass the ordinary label-aware pass.
fn redact_model_input_text_at_depth(value: &str, encoded_depth: usize) -> ModelInputRedaction {
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
        .filter_map(|captures| captures.name("value"))
        .filter_map(|candidate| trimmed_candidate_range(value, candidate.range()))
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    merge_ranges(ranges)
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
    matches!(
        value.to_ascii_lowercase().as_str(),
        "redacted"
            | "redacted_secret"
            | "placeholder"
            | "example"
            | "token"
            | "value"
            | "key"
            | "your-token"
            | "your_token"
    ) || value.contains("...")
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

fn apply_redactions(value: &str, ranges: &[Range<usize>]) -> String {
    if ranges.is_empty() {
        return value.to_string();
    }
    let mut redacted = String::with_capacity(value.len());
    let mut cursor = 0;
    for range in ranges {
        redacted.push_str(&value[cursor..range.start]);
        redacted.push_str(REDACTED_SECRET);
        cursor = range.end;
    }
    redacted.push_str(&value[cursor..]);
    redacted
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::redact_model_input_text;

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
        assert!(!repeated.was_modified());
    }

    #[test]
    fn keeps_security_prose_and_paths_unchanged() {
        for input in [
            "The report documents an authorization flow and API key rotation.",
            "Read /Users/alice/.config/token before reviewing the report.",
            "The upstream service returned invalid API key.",
            "surface sha256:269cc57b4d0c4368d8b02738ab709c810adb6212729b24bbdc34efb539a3ed07",
            "/etc/passwd",
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
