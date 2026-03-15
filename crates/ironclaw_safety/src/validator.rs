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

    /// Regression: empty input is rejected by validate(), but image-only messages
    /// (empty text + attachments) should bypass this check at the caller level.
    /// This test confirms the validator correctly rejects empty strings — the
    /// bypass logic lives in thread_ops.rs, not in the validator itself.
    #[test]
    fn test_empty_input_rejected() {
        let validator = Validator::new();
        let result = validator.validate("");
        assert!(!result.is_valid, "Empty input must be rejected");
        assert_eq!(result.errors[0].code, ValidationErrorCode::Empty);
    }

    /// Non-empty input with valid content should pass validation.
    #[test]
    fn test_nonempty_input_passes() {
        let validator = Validator::new();
        let result = validator.validate("Hello, world!");
        assert!(result.is_valid, "Non-empty valid input must pass");
    }
}
