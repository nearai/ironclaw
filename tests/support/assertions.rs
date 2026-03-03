//! Shared assertion helpers for E2E tests.
//!
//! Extracted from `e2e_spot_checks.rs` so they can be reused across all E2E
//! test files. Mirrors the assertion types from `nearai/benchmarks` SpotSuite.

#![allow(dead_code)]

use regex::Regex;

/// Assert the response contains all `needles` (case-insensitive).
pub fn assert_response_contains(response: &str, needles: &[&str]) {
    let lower = response.to_lowercase();
    for needle in needles {
        assert!(
            lower.contains(&needle.to_lowercase()),
            "response_contains: missing \"{needle}\" in response: {response}"
        );
    }
}

/// Assert the response matches the given regex `pattern`.
pub fn assert_response_matches(response: &str, pattern: &str) {
    let re = Regex::new(pattern).expect("invalid regex pattern");
    assert!(
        re.is_match(response),
        "response_matches: /{pattern}/ did not match response: {response}"
    );
}

/// Assert that all `expected` tool names appear in `started`.
pub fn assert_tools_used(started: &[String], expected: &[&str]) {
    for tool in expected {
        assert!(
            started.iter().any(|s| s == tool),
            "tools_used: \"{tool}\" not called, got: {started:?}"
        );
    }
}

/// Assert that none of the `forbidden` tool names appear in `started`.
pub fn assert_tools_not_used(started: &[String], forbidden: &[&str]) {
    for tool in forbidden {
        assert!(
            !started.iter().any(|s| s == tool),
            "tools_not_used: \"{tool}\" was called, got: {started:?}"
        );
    }
}

/// Assert at most `max` tool calls were started.
pub fn assert_max_tool_calls(started: &[String], max: usize) {
    assert!(
        started.len() <= max,
        "max_tool_calls: expected <= {max}, got {}. Tools: {started:?}",
        started.len()
    );
}

/// Assert ALL completed tools succeeded. Panics listing failed tools.
pub fn assert_all_tools_succeeded(completed: &[(String, bool)]) {
    let failed: Vec<&str> = completed
        .iter()
        .filter(|(_, success)| !*success)
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        failed.is_empty(),
        "Expected all tools to succeed, but these failed: {failed:?}. All: {completed:?}"
    );
}

/// Assert a specific tool completed successfully at least once.
pub fn assert_tool_succeeded(completed: &[(String, bool)], tool_name: &str) {
    let found = completed
        .iter()
        .any(|(name, success)| name == tool_name && *success);
    assert!(
        found,
        "Expected '{tool_name}' to complete successfully, got: {completed:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- assert_all_tools_succeeded -------------------------------------------

    #[test]
    fn all_tools_succeeded_passes_when_all_true() {
        let completed = vec![("echo".to_string(), true), ("time".to_string(), true)];
        assert_all_tools_succeeded(&completed);
    }

    #[test]
    fn all_tools_succeeded_passes_on_empty() {
        assert_all_tools_succeeded(&[]);
    }

    #[test]
    #[should_panic(expected = "Expected all tools to succeed")]
    fn all_tools_succeeded_panics_on_failure() {
        let completed = vec![("echo".to_string(), true), ("shell".to_string(), false)];
        assert_all_tools_succeeded(&completed);
    }

    // -- assert_tool_succeeded ------------------------------------------------

    #[test]
    fn tool_succeeded_passes_when_present_and_true() {
        let completed = vec![("echo".to_string(), true), ("time".to_string(), false)];
        assert_tool_succeeded(&completed, "echo");
    }

    #[test]
    #[should_panic(expected = "Expected 'echo' to complete successfully")]
    fn tool_succeeded_panics_when_tool_missing() {
        let completed = vec![("time".to_string(), true)];
        assert_tool_succeeded(&completed, "echo");
    }

    #[test]
    #[should_panic(expected = "Expected 'shell' to complete successfully")]
    fn tool_succeeded_panics_when_tool_failed() {
        let completed = vec![("shell".to_string(), false)];
        assert_tool_succeeded(&completed, "shell");
    }
}
