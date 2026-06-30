//! Egress + tool-result assertions for [`RebornIntegrationHarness`] — the
//! canonical, richer egress-assertion API (design §3.3 `assertions.rs`, §3.6
//! P1 ergonomics).
//!
//! Slice 2 co-located three asserts in `builder.rs`
//! (`assert_reply_contains`/`assert_tool_invoked`/`assert_egress_request_matching`,
//! a single substring host check). This slice grows the `assert_*` family past
//! that threshold, so the egress/tool-result assertions move to their own file
//! per the long-planned split. They read the captured Tier-2
//! `RuntimeHttpEgressRequest`s and recorded capability results through the
//! `pub(super)` accessors on the harness (`captured_egress_requests` /
//! `captured_capability_results`) rather than re-reaching internals.
//!
//! All of these assert over the SAME captured `RecordingRuntimeHttpEgress`
//! request log slice 2 wired — there is one egress-assertion API, not a parallel
//! one (the O-egress MCP/OAuth interceptor folds its per-URL needs in here).

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module (e.g. `support_unit_tests.rs`), so
// its symbols read as dead there under the all-features `-D warnings` lane.
// Module-level allow matches `builder.rs`/`reply.rs`/`http_matcher.rs`.
#![allow(dead_code)]

use super::builder::RebornIntegrationHarness;

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl RebornIntegrationHarness {
    /// Assert exactly `expected` Tier-2 HTTP egress requests were captured.
    pub async fn assert_egress_count(&self, expected: usize) -> HarnessResult<()> {
        let actual = self.captured_egress_requests().len();
        if actual == expected {
            return Ok(());
        }
        Err(format!("expected {expected} captured egress request(s), saw {actual}").into())
    }

    /// Assert the captured egress URLs, IN CALL ORDER, each contain the matching
    /// substring in `expected` — and that the count matches `expected.len()`.
    /// Covers URL + ordering + count in one terse assertion.
    pub async fn assert_egress_url_order(&self, expected: &[&str]) -> HarnessResult<()> {
        let requests = self.captured_egress_requests();
        let seen: Vec<String> = requests.iter().map(|r| r.url.clone()).collect();
        if requests.len() != expected.len() {
            return Err(format!(
                "expected {} egress request(s), saw {}: {seen:?}",
                expected.len(),
                requests.len()
            )
            .into());
        }
        for (index, (request, expected_substr)) in requests.iter().zip(expected).enumerate() {
            if !request.url.contains(expected_substr) {
                return Err(format!(
                    "egress[{index}] url {:?} does not contain {expected_substr:?}; full log: {seen:?}",
                    request.url
                )
                .into());
            }
        }
        Ok(())
    }

    /// Assert the captured egress methods, IN CALL ORDER, equal `expected`
    /// (case-insensitive; methods render lowercase, e.g. `"get"`/`"post"`).
    /// Covers method + ordering + count.
    pub async fn assert_egress_method_order(&self, expected: &[&str]) -> HarnessResult<()> {
        let requests = self.captured_egress_requests();
        let seen: Vec<String> = requests.iter().map(|r| r.method.to_string()).collect();
        if requests.len() != expected.len() {
            return Err(format!(
                "expected {} egress request(s), saw {}: {seen:?}",
                expected.len(),
                requests.len()
            )
            .into());
        }
        for (index, (actual, expected_method)) in seen.iter().zip(expected).enumerate() {
            if !actual.eq_ignore_ascii_case(expected_method) {
                return Err(format!(
                    "egress[{index}] method {actual:?} != {expected_method:?}; full log: {seen:?}"
                )
                .into());
            }
        }
        Ok(())
    }

    /// Assert that the (first) captured egress request whose URL contains
    /// `url_substr` carried a body containing `body_substr`. Covers request-body
    /// capture for a keyed multi-step flow.
    pub async fn assert_egress_body_contains(
        &self,
        url_substr: &str,
        body_substr: &str,
    ) -> HarnessResult<()> {
        let requests = self.captured_egress_requests();
        let Some(request) = requests.iter().find(|r| r.url.contains(url_substr)) else {
            let seen: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
            return Err(format!(
                "no captured egress request matching url {url_substr:?}; saw {seen:?}"
            )
            .into());
        };
        let body = String::from_utf8_lossy(&request.body);
        if body.contains(body_substr) {
            return Ok(());
        }
        Err(format!(
            "egress request to {url_substr:?} body {body:?} does not contain {body_substr:?}"
        )
        .into())
    }

    /// Assert some recorded capability result (tool output) — i.e. a surfaced
    /// HTTP response — serializes to text containing `needle`. Proves the keyed
    /// scripted body actually surfaced back to the model as a tool result.
    pub async fn assert_tool_result_contains(&self, needle: &str) -> HarnessResult<()> {
        let results = self.captured_capability_results();
        if results
            .iter()
            .any(|result| result.output.to_string().contains(needle))
        {
            return Ok(());
        }
        let seen: Vec<String> = results
            .iter()
            .map(|result| result.capability_id.as_str().to_string())
            .collect();
        Err(format!(
            "no recorded capability result containing {needle:?}; saw results for {seen:?}"
        )
        .into())
    }
}
