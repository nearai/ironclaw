//! Input validation for the safety layer.

use std::collections::HashSet;

/// Result of validating input.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the input is valid.
    pub is_valid: bool,
    /// Validation errors if any.
    pub errors: Vec<ValidationError>,
    /// Warnings that don't block processing.
    pub warnings: Vec<String>,
}

impl ValidationResult {
    /// Create a successful validation result.
    pub fn ok() -> Self {
        Self {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        }
    }

    /// Create a validation result with an error.
    pub fn error(error: ValidationError) -> Self {
        Self {
            is_valid: false,
            errors: vec![error],
            warnings: vec![],
        }
    }

    /// Add a warning to the result.
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// Merge another validation result into this one.
    pub fn merge(mut self, other: Self) -> Self {
        self.is_valid = self.is_valid && other.is_valid;
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        self
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::ok()
    }
}

/// A validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Field or aspect that failed validation.
    pub field: String,
    /// Error message.
    pub message: String,
    /// Error code for programmatic handling.
    pub code: ValidationErrorCode,
}

/// Error codes for validation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationErrorCode {
    Empty,
    TooLong,
    TooShort,
    InvalidFormat,
    ForbiddenContent,
    InvalidEncoding,
    SuspiciousPattern,
}

/// Input validator.
pub struct Validator {
    /// Maximum input length.
    max_length: usize,
    /// Minimum input length.
    min_length: usize,
    /// Forbidden substrings.
    forbidden_patterns: HashSet<String>,
}

impl Validator {
    /// Create a new validator with default settings.
    pub fn new() -> Self {
        Self {
            max_length: 100_000,
            min_length: 1,
            forbidden_patterns: HashSet::new(),
        }
    }

    /// Set maximum input length.
    pub fn with_max_length(mut self, max: usize) -> Self {
        self.max_length = max;
        self
    }

    /// Set minimum input length.
    pub fn with_min_length(mut self, min: usize) -> Self {
        self.min_length = min;
        self
    }

    /// Add a forbidden pattern.
    pub fn forbid_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.forbidden_patterns
            .insert(pattern.into().to_lowercase());
        self
    }

    /// Validate input text.
    pub fn validate(&self, input: &str) -> ValidationResult {
        // Check empty
        if input.is_empty() {
            return ValidationResult::error(ValidationError {
                field: "input".to_string(),
                message: "Input cannot be empty".to_string(),
                code: ValidationErrorCode::Empty,
            });
        }

        self.validate_non_empty_input(input, "input")
    }

    fn validate_non_empty_input(&self, input: &str, field: &str) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Check length
        if input.len() > self.max_length {
            result = result.merge(ValidationResult::error(ValidationError {
                field: field.to_string(),
                message: format!(
                    "Input too long: {} bytes (max {})",
                    input.len(),
                    self.max_length
                ),
                code: ValidationErrorCode::TooLong,
            }));
        }

        if input.len() < self.min_length {
            result = result.merge(ValidationResult::error(ValidationError {
                field: field.to_string(),
                message: format!(
                    "Input too short: {} bytes (min {})",
                    input.len(),
                    self.min_length
                ),
                code: ValidationErrorCode::TooShort,
            }));
        }

        // Check for valid UTF-8 (should always pass since we have a &str, but check for weird chars)
        if input.chars().any(|c| c == '\x00') {
            result = result.merge(ValidationResult::error(ValidationError {
                field: field.to_string(),
                message: "Input contains null bytes".to_string(),
                code: ValidationErrorCode::InvalidEncoding,
            }));
        }

        // Check forbidden patterns
        let lower_input = input.to_lowercase();
        for pattern in &self.forbidden_patterns {
            if lower_input.contains(pattern) {
                result = result.merge(ValidationResult::error(ValidationError {
                    field: field.to_string(),
                    message: format!("Input contains forbidden pattern: {}", pattern),
                    code: ValidationErrorCode::ForbiddenContent,
                }));
            }
        }

        // Check for excessive whitespace (might indicate padding attacks)
        let whitespace_ratio =
            input.chars().filter(|c| c.is_whitespace()).count() as f64 / input.len() as f64;
        if whitespace_ratio > 0.9 && input.len() > 100 {
            result = result.with_warning("Input has unusually high whitespace ratio");
        }

        // Check for repeated characters (might indicate padding)
        if has_excessive_repetition(input) {
            result = result.with_warning("Input has excessive character repetition");
        }

        result
    }

    /// Validate tool parameters.
    pub fn validate_tool_params(&self, params: &serde_json::Value) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Recursively check all string values in the JSON.
        // Depth is capped to prevent stack overflow on pathological input.
        const MAX_DEPTH: usize = 32;

        fn check_strings(
            value: &serde_json::Value,
            path: &str,
            validator: &Validator,
            result: &mut ValidationResult,
            depth: usize,
        ) {
            if depth > MAX_DEPTH {
                return;
            }
            match value {
                serde_json::Value::String(s) => {
                    let string_result = if s.is_empty() {
                        ValidationResult::ok()
                    } else {
                        validator.validate_non_empty_input(s, path)
                    };
                    *result = std::mem::take(result).merge(string_result);
                }
                serde_json::Value::Array(arr) => {
                    for (i, item) in arr.iter().enumerate() {
                        let child_path = format!("{path}[{i}]");
                        check_strings(item, &child_path, validator, result, depth + 1);
                    }
                }
                serde_json::Value::Object(obj) => {
                    for (k, v) in obj {
                        let child_path = if path.is_empty() {
                            k.clone()
                        } else {
                            format!("{path}.{k}")
                        };
                        check_strings(v, &child_path, validator, result, depth + 1);
                    }
                }
                _ => {}
            }
        }

        check_strings(params, "", self, &mut result, 0);
        result
    }
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if string has excessive repetition of characters.
fn has_excessive_repetition(s: &str) -> bool {
    if s.len() < 50 {
        return false;
    }

    let chars: Vec<char> = s.chars().collect();
    let mut max_repeat = 1;
    let mut current_repeat = 1;

    for i in 1..chars.len() {
        if chars[i] == chars[i - 1] {
            current_repeat += 1;
            max_repeat = max_repeat.max(current_repeat);
        } else {
            current_repeat = 1;
        }
    }

    // More than 20 repeated characters is suspicious
    max_repeat > 20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_input() {
        let validator = Validator::new();
        let result = validator.validate("Hello, this is a normal message.");
        assert!(result.is_valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_empty_input() {
        let validator = Validator::new();
        let result = validator.validate("");
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == ValidationErrorCode::Empty)
        );
    }

    #[test]
    fn test_too_long_input() {
        let validator = Validator::new().with_max_length(10);
        let result = validator.validate("This is way too long for the limit");
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == ValidationErrorCode::TooLong)
        );
    }

    #[test]
    fn test_forbidden_pattern() {
        let validator = Validator::new().forbid_pattern("forbidden");
        let result = validator.validate("This contains FORBIDDEN content");
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == ValidationErrorCode::ForbiddenContent)
        );
    }

    #[test]
    fn test_excessive_repetition_warning() {
        let validator = Validator::new();
        // String needs to be >= 50 chars for repetition check
        let result =
            validator.validate(&format!("Start of message{}End of message", "a".repeat(30)));
        assert!(result.is_valid); // Still valid, just a warning
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_tool_params_allow_empty_strings() {
        let validator = Validator::new();
        let result = validator.validate_tool_params(&serde_json::json!({
            "path": "",
            "nested": {
                "label": ""
            },
            "items": [""]
        }));

        assert!(result.is_valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_tool_params_still_block_null_bytes() {
        let validator = Validator::new();
        let result = validator.validate_tool_params(&serde_json::json!({
            "path": "bad\u{0000}path"
        }));

        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == ValidationErrorCode::InvalidEncoding)
        );
    }

    #[test]
    fn test_tool_params_still_block_forbidden_patterns() {
        let validator = Validator::new().forbid_pattern("forbidden");
        let result = validator.validate_tool_params(&serde_json::json!({
            "path": "contains forbidden content"
        }));

        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == ValidationErrorCode::ForbiddenContent)
        );
    }

    #[test]
    fn test_tool_params_still_warn_on_repetition() {
        let validator = Validator::new();
        let result = validator.validate_tool_params(&serde_json::json!({
            "content": format!("prefix{}suffix", "x".repeat(50))
        }));

        assert!(result.is_valid);
        assert!(
            result.warnings.iter().any(|w| w.contains("repetition")),
            "expected repetition warning for tool params, got: {:?}",
            result.warnings
        );
    }

    #[test]
    fn test_tool_params_still_warn_on_whitespace_ratio() {
        let validator = Validator::new();
        // >100 chars, >90% whitespace
        let result = validator.validate_tool_params(&serde_json::json!({
            "content": format!("a{}b", " ".repeat(200))
        }));

        assert!(result.is_valid);
        assert!(
            result.warnings.iter().any(|w| w.contains("whitespace")),
            "expected whitespace warning for tool params, got: {:?}",
            result.warnings
        );
    }

    #[test]
    fn test_tool_params_error_field_contains_json_path() {
        let validator = Validator::new().forbid_pattern("evil");
        let result = validator.validate_tool_params(&serde_json::json!({
            "metadata": {
                "tags": ["good", "evil"]
            }
        }));

        assert!(!result.is_valid);
        let error = result
            .errors
            .iter()
            .find(|e| e.code == ValidationErrorCode::ForbiddenContent)
            .expect("expected forbidden content error");
        assert_eq!(error.field, "metadata.tags[1]");
    }

    #[test]
    fn test_tool_params_depth_limit_prevents_stack_overflow() {
        let validator = Validator::new().forbid_pattern("evil");

        // Build a deeply nested JSON object (depth > MAX_DEPTH of 32)
        let mut value = serde_json::json!("evil payload");
        for _ in 0..50 {
            value = serde_json::json!({ "nested": value });
        }

        let result = validator.validate_tool_params(&value);

        // The "evil payload" is beyond the depth limit so it should NOT be
        // detected — the traversal stops before reaching it.
        assert!(
            result.is_valid,
            "Strings beyond depth limit should be silently skipped, got errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_tool_params_within_depth_limit_still_validated() {
        let validator = Validator::new().forbid_pattern("evil");

        // Build a nested object within the depth limit
        let mut value = serde_json::json!("evil payload");
        for _ in 0..5 {
            value = serde_json::json!({ "nested": value });
        }

        let result = validator.validate_tool_params(&value);
        assert!(
            !result.is_valid,
            "Strings within depth limit should still be validated"
        );
    }

    /// Adversarial tests for validator whitespace ratio, repetition detection,
    /// and Unicode edge cases.
    /// See <https://github.com/nearai/ironclaw/issues/1025>.
    mod adversarial {
        use super::*;

        // ── A. Performance guards ────────────────────────────────────

        #[test]
        fn validate_100kb_input_within_threshold() {
            let validator = Validator::new();
            let payload = "normal text content here. ".repeat(4500);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = validator.validate(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 100,
                "validate() took {}ms on 100KB input",
                elapsed.as_millis()
            );
        }

        #[test]
        fn excessive_repetition_100kb() {
            let validator = Validator::new();
            let payload = "a".repeat(100_001);

            let start = std::time::Instant::now();
            let result = validator.validate(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 100,
                "repetition check took {}ms on 100KB",
                elapsed.as_millis()
            );
            assert!(
                !result.warnings.is_empty(),
                "100KB of repeated 'a' should warn"
            );
        }

        #[test]
        fn tool_params_deeply_nested_100kb() {
            let validator = Validator::new().forbid_pattern("evil");
            // Wide JSON: many keys at top level, 100KB+ total
            let mut obj = serde_json::Map::new();
            for i in 0..2000 {
                obj.insert(
                    format!("key_{i}"),
                    serde_json::Value::String("normal content value ".repeat(3)),
                );
            }
            let value = serde_json::Value::Object(obj);

            let start = std::time::Instant::now();
            let _result = validator.validate_tool_params(&value);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 100,
                "tool_params validation took {}ms on wide JSON",
                elapsed.as_millis()
            );
        }

        // ── B. Unicode edge cases ────────────────────────────────────

        #[test]
        fn zwsp_not_counted_as_whitespace() {
            let validator = Validator::new();
            let input = "\u{200B}".repeat(200);
            let result = validator.validate(&input);
            assert!(
                !result.warnings.iter().any(|w| w.contains("whitespace")),
                "ZWSP should not count as whitespace (char::is_whitespace returns false)"
            );
        }

        #[test]
        fn zwnj_not_counted_as_whitespace() {
            let validator = Validator::new();
            let input = "\u{200C}".repeat(200);
            let result = validator.validate(&input);
            assert!(
                !result.warnings.iter().any(|w| w.contains("whitespace")),
                "ZWNJ should not count as whitespace (char::is_whitespace returns false)"
            );
        }

        #[test]
        fn zwnj_in_forbidden_pattern() {
            let validator = Validator::new().forbid_pattern("evil");
            let input = "some text ev\u{200C}il command here";
            let result = validator.validate_non_empty_input(input, "test");
            assert!(
                result.is_valid,
                "ZWNJ breaks forbidden pattern substring match — known bypass"
            );
        }

        #[test]
        fn zwj_not_counted_as_whitespace() {
            let validator = Validator::new();
            let input = "\u{200D}".repeat(200);
            let result = validator.validate(&input);
            assert!(
                !result.warnings.iter().any(|w| w.contains("whitespace")),
                "ZWJ should not count as whitespace (char::is_whitespace returns false)"
            );
        }

        #[test]
        fn actual_whitespace_padding_attack() {
            let validator = Validator::new();
            let input = format!("{}{}", " ".repeat(190), "real content");
            assert!(input.len() > 100);
            let result = validator.validate(&input);
            assert!(
                result.warnings.iter().any(|w| w.contains("whitespace")),
                "high whitespace ratio should be warned"
            );
        }

        #[test]
        fn combining_diacriticals_in_repetition() {
            let input = "a\u{0301}".repeat(30);
            assert!(!has_excessive_repetition(&input));
        }

        #[test]
        fn base_char_plus_50_distinct_combining_diacriticals() {
            let combining_marks: Vec<char> =
                (0x0300u32..=0x0331).filter_map(char::from_u32).collect();
            assert!(combining_marks.len() >= 50);
            let marks: String = combining_marks[..50].iter().collect();
            let input = format!("prefix a{marks}suffix padding to reach minimum length for check");
            assert!(
                !has_excessive_repetition(&input),
                "50 distinct combining marks should NOT trigger excessive repetition"
            );
        }

        #[test]
        fn multibyte_chars_at_max_length_boundary() {
            let max_len = 100;
            let validator = Validator::new().with_max_length(max_len);

            let input = "中".repeat(34);
            assert_eq!(input.len(), 102);
            let result = validator.validate(&input);
            assert!(
                !result.is_valid,
                "102 bytes of CJK should exceed max_length=100 (byte-based check)"
            );
            assert!(
                result
                    .errors
                    .iter()
                    .any(|e| e.code == ValidationErrorCode::TooLong),
                "should produce TooLong error"
            );

            let input = "中".repeat(33);
            assert_eq!(input.len(), 99);
            let result = validator.validate(&input);
            assert!(
                !result
                    .errors
                    .iter()
                    .any(|e| e.code == ValidationErrorCode::TooLong),
                "99 bytes of CJK should not exceed max_length=100"
            );
        }

        #[test]
        fn four_byte_emoji_at_max_length_boundary() {
            let max_len = 100;
            let validator = Validator::new().with_max_length(max_len);

            let input = "🔑".repeat(25);
            assert_eq!(input.len(), 100);
            let result = validator.validate(&input);
            assert!(
                !result
                    .errors
                    .iter()
                    .any(|e| e.code == ValidationErrorCode::TooLong),
                "exactly 100 bytes should not exceed max_length=100"
            );

            let input = "🔑".repeat(26);
            assert_eq!(input.len(), 104);
            let result = validator.validate(&input);
            assert!(
                result
                    .errors
                    .iter()
                    .any(|e| e.code == ValidationErrorCode::TooLong),
                "104 bytes should exceed max_length=100"
            );
        }

        #[test]
        fn single_codepoint_emoji_repetition() {
            let input = "😀".repeat(25);
            assert!(
                has_excessive_repetition(&input),
                "25 repeated emoji should count as excessive repetition"
            );
        }

        #[test]
        fn multibyte_input_whitespace_ratio_uses_len_not_chars() {
            let validator = Validator::new();
            let input = format!("{}{}", " ".repeat(50), "中".repeat(50));
            let result = validator.validate(&input);
            assert!(
                !result.warnings.iter().any(|w| w.contains("whitespace")),
                "multibyte chars make byte-length ratio low — documents len() vs chars() divergence"
            );
        }

        #[test]
        fn rtl_override_in_forbidden_pattern() {
            let validator = Validator::new().forbid_pattern("evil");
            let input = "some text \u{202E}evil command here";
            let result = validator.validate_non_empty_input(input, "test");
            assert!(
                !result.is_valid,
                "RTL override should not prevent forbidden pattern detection"
            );
        }

        // ── C. Control character variants ────────────────────────────

        #[test]
        fn control_chars_in_input_no_panic() {
            let validator = Validator::new();
            for byte in 0x01u8..=0x1f {
                let input = format!(
                    "prefix {} suffix content padding to be long enough",
                    char::from(byte)
                );
                let _result = validator.validate(&input);
            }
        }

        #[test]
        fn bom_with_forbidden_pattern() {
            let validator = Validator::new().forbid_pattern("evil");
            let input = "\u{FEFF}this is evil content";
            let result = validator.validate_non_empty_input(input, "test");
            assert!(
                !result.is_valid,
                "BOM prefix should not prevent forbidden pattern detection"
            );
        }

        #[test]
        fn control_chars_in_repetition_check() {
            let input = "\x07".repeat(55);
            let _ = has_excessive_repetition(&input);
        }
    }
}
