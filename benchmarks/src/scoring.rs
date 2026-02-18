use regex::Regex;

use crate::suite::BenchScore;

/// Normalize an answer string for comparison: lowercase, trim whitespace,
/// strip trailing punctuation, collapse internal whitespace.
pub fn normalize_answer(s: &str) -> String {
    let trimmed = s.trim().to_lowercase();
    let collapsed: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches(['.', ',', ';', '!']).to_string()
}

/// Exact match after normalization.
pub fn exact_match(expected: &str, actual: &str) -> BenchScore {
    let norm_expected = normalize_answer(expected);
    let norm_actual = normalize_answer(actual);
    if norm_expected == norm_actual {
        BenchScore::pass()
    } else {
        BenchScore::fail(format!(
            "expected \"{norm_expected}\", got \"{norm_actual}\""
        ))
    }
}

/// Check if the actual answer contains the expected substring (normalized).
pub fn contains_match(expected_substring: &str, actual: &str) -> BenchScore {
    let norm_expected = normalize_answer(expected_substring);
    let norm_actual = normalize_answer(actual);
    if norm_actual.contains(&norm_expected) {
        BenchScore::pass()
    } else {
        BenchScore::fail(format!("response does not contain \"{norm_expected}\""))
    }
}

/// Check if the actual answer matches a regex pattern.
pub fn regex_match(pattern: &str, actual: &str) -> BenchScore {
    match Regex::new(pattern) {
        Ok(re) => {
            if re.is_match(actual) {
                BenchScore::pass()
            } else {
                BenchScore::fail(format!("response does not match pattern /{pattern}/"))
            }
        }
        Err(e) => BenchScore::fail(format!("invalid regex pattern: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_answer() {
        assert_eq!(normalize_answer("  Hello   World.  "), "hello world");
        assert_eq!(normalize_answer("Yes!"), "yes");
        assert_eq!(normalize_answer("42"), "42");
        assert_eq!(normalize_answer("  "), "");
    }

    #[test]
    fn test_exact_match_pass() {
        let score = exact_match("Hello World", "  hello   world.  ");
        assert_eq!(score.value, 1.0);
        assert_eq!(score.label, "pass");
    }

    #[test]
    fn test_exact_match_fail() {
        let score = exact_match("hello", "world");
        assert_eq!(score.value, 0.0);
        assert_eq!(score.label, "fail");
    }

    #[test]
    fn test_contains_match_pass() {
        let score = contains_match("world", "Hello World!");
        assert_eq!(score.value, 1.0);
    }

    #[test]
    fn test_contains_match_fail() {
        let score = contains_match("xyz", "Hello World!");
        assert_eq!(score.value, 0.0);
    }

    #[test]
    fn test_regex_match_pass() {
        let score = regex_match(r"\d{4}", "The year is 2024.");
        assert_eq!(score.value, 1.0);
    }

    #[test]
    fn test_regex_match_fail() {
        let score = regex_match(r"\d{4}", "No numbers here.");
        assert_eq!(score.value, 0.0);
    }

    #[test]
    fn test_regex_match_invalid_pattern() {
        let score = regex_match(r"[invalid", "anything");
        assert_eq!(score.value, 0.0);
        assert!(
            score
                .details
                .as_deref()
                .unwrap_or("")
                .contains("invalid regex")
        );
    }
}
