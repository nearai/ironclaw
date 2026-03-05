//! Shared assertion helpers for E2E tests.
//!
//! Extracted from `e2e_spot_checks.rs` so they can be reused across all E2E
//! test files. Mirrors the assertion types from `nearai/benchmarks` SpotSuite.

#![allow(dead_code)]

use regex::Regex;

use crate::support::trace_llm::TraceExpects;

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

/// Assert the response does NOT contain any of `forbidden` (case-insensitive).
pub fn assert_response_not_contains(response: &str, forbidden: &[&str]) {
    let lower = response.to_lowercase();
    for needle in forbidden {
        assert!(
            !lower.contains(&needle.to_lowercase()),
            "response_not_contains: found \"{needle}\" in response: {response}"
        );
    }
}

/// Assert that `expected` tools appear in `started` in the given order.
///
/// The tools need not be consecutive — only relative ordering is checked.
/// For example, `assert_tool_order(started, &["write_file", "read_file"])`
/// passes if `write_file` appears before `read_file`, even with other tools
/// in between.
pub fn assert_tool_order(started: &[String], expected: &[&str]) {
    let mut search_from = 0;
    for tool in expected {
        let pos = started[search_from..]
            .iter()
            .position(|s| s == tool)
            .map(|p| p + search_from);
        match pos {
            Some(idx) => search_from = idx + 1,
            None => {
                panic!(
                    "assert_tool_order: \"{tool}\" not found after position {search_from} \
                     in: {started:?}. Expected order: {expected:?}"
                );
            }
        }
    }
}

/// Verify all expectations from a `TraceExpects` against actual data.
///
/// `label` is used in assertion messages to identify context (e.g. "top-level" or "turn 0").
/// `responses` are the response content strings, `started` are tool names started,
/// `completed` are (name, success) pairs, `results` are (name, preview) pairs.
pub fn verify_expects(
    expects: &TraceExpects,
    responses: &[String],
    started: &[String],
    completed: &[(String, bool)],
    results: &[(String, String)],
    label: &str,
) {
    if expects.is_empty() {
        return;
    }

    // min_responses
    if let Some(min) = expects.min_responses {
        assert!(
            responses.len() >= min,
            "[{label}] min_responses: expected >= {min}, got {}",
            responses.len()
        );
    }

    // response_contains / response_not_contains / response_matches — checked against joined response
    let joined = responses.join("\n");

    if !expects.response_contains.is_empty() {
        let needles: Vec<&str> = expects
            .response_contains
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_response_contains(&joined, &needles);
    }

    if !expects.response_not_contains.is_empty() {
        let forbidden: Vec<&str> = expects
            .response_not_contains
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_response_not_contains(&joined, &forbidden);
    }

    if let Some(ref pattern) = expects.response_matches {
        assert_response_matches(&joined, pattern);
    }

    // tools_used
    if !expects.tools_used.is_empty() {
        let expected: Vec<&str> = expects.tools_used.iter().map(|s| s.as_str()).collect();
        assert_tools_used(started, &expected);
    }

    // tools_not_used
    if !expects.tools_not_used.is_empty() {
        let forbidden: Vec<&str> = expects.tools_not_used.iter().map(|s| s.as_str()).collect();
        assert_tools_not_used(started, &forbidden);
    }

    // all_tools_succeeded
    if expects.all_tools_succeeded == Some(true) {
        assert_all_tools_succeeded(completed);
    }

    // max_tool_calls
    if let Some(max) = expects.max_tool_calls {
        assert_max_tool_calls(started, max);
    }

    // tools_order
    if !expects.tools_order.is_empty() {
        let expected: Vec<&str> = expects.tools_order.iter().map(|s| s.as_str()).collect();
        assert_tool_order(started, &expected);
    }

    // tool_results_contain
    for (tool_name, substring) in &expects.tool_results_contain {
        let found = results.iter().find(|(name, _)| name == tool_name);
        assert!(
            found.is_some(),
            "[{label}] tool_results_contain: no result for tool \"{tool_name}\", got: {results:?}"
        );
        let (_, preview) = found.unwrap();
        assert!(
            preview.to_lowercase().contains(&substring.to_lowercase()),
            "[{label}] tool_results_contain: tool \"{tool_name}\" result does not contain \"{substring}\", got: \"{preview}\""
        );
    }
}
