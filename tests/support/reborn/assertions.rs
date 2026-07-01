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

/// The two model-visible tool-error outcome classes a capability can surface
/// (`CapabilityOutcome::Failed` vs `Denied`). A `Failed` and a `Denied` outcome
/// can render the SAME reason token (e.g. `policy_denied` is both
/// `CapabilityFailureKind::PolicyDenied` and the `Denied` reason), so
/// [`assert_tool_error`](RebornIntegrationHarness::assert_tool_error) takes the
/// class as a typed argument rather than trusting a needle prefix — the class is
/// then discriminated structurally, not by convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorClass {
    Failed,
    Denied,
}

impl ToolErrorClass {
    /// The `safe_summary` prefix the executor writes for this class — see
    /// `capability_{failed,denied}_summary` in
    /// `crates/ironclaw_agent_loop/src/executor/capabilities.rs`.
    fn summary_prefix(self) -> &'static str {
        match self {
            Self::Failed => "capability failed with ",
            Self::Denied => "capability denied with ",
        }
    }
}

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

    /// Assert a model-visible tool error of `class` carrying `reason` was
    /// persisted for this thread. Unlike [`assert_tool_result_contains`] (which
    /// reads the in-process recorder, populated only on the *Completed* write
    /// path), a `Failed`/`Denied` capability outcome is persisted through a
    /// different pipeline — `append_capability_result_ref` →
    /// `append_tool_result_reference` — as a `MessageKind::ToolResultReference`
    /// message whose `content` is the JSON-serialized `ToolResultReferenceEnvelope`.
    /// Reaching this state at all (rather than a terminal `driver_unavailable`)
    /// also proves the failure was a recoverable, model-visible tool error.
    ///
    /// This parses the envelope and checks its **`safe_summary` field** — NOT a
    /// raw-JSON substring — so `reason` cannot match incidentally inside
    /// `model_observation`/`result_ref`, and JSON escaping can't skew the match.
    /// The summary reads `"capability <failed|denied> with <token>: …"`; the
    /// assertion requires the summary to start with [`class`](ToolErrorClass)'s
    /// prefix AND contain `reason`. `class` therefore discriminates
    /// Failed-vs-Denied structurally: a regression that flips one into the other
    /// fails even when both classes render the same `reason` token (e.g.
    /// `policy_denied`).
    ///
    /// **Scans the full thread history, not baseline-sliced** (unlike the
    /// sibling `assert_egress_*`/`assert_tool_result_contains`, which slice
    /// `[baseline..]` off the shared in-process recorder). Every current caller
    /// is a single-turn, single-tool-call harness, so there is at most one
    /// `ToolResultReference` message and no earlier-thread bleed-through is
    /// reachable. A future multi-turn or group-thread reuse of this assertion
    /// MUST add baseline scoping first (thread a `baseline_history_len` through
    /// harness construction, mirroring `baseline_egress_count` etc.) — do not
    /// assume this helper is safe to reuse as-is once a thread has more than one
    /// turn.
    pub async fn assert_tool_error(
        &self,
        class: ToolErrorClass,
        reason: &str,
    ) -> HarnessResult<()> {
        let history = self
            .thread_harness
            .history(self.binding.thread_id.clone())
            .await?;
        let summaries: Vec<String> = history
            .iter()
            .filter(|message| message.kind == ironclaw_threads::MessageKind::ToolResultReference)
            .filter_map(|message| message.content.as_deref())
            .filter_map(|content| {
                serde_json::from_str::<ironclaw_threads::ToolResultReferenceEnvelope>(content).ok()
            })
            .map(|envelope| envelope.safe_summary.as_str().to_string())
            .collect();
        let prefix = class.summary_prefix();
        if summaries
            .iter()
            .any(|summary| summary.starts_with(prefix) && summary.contains(reason))
        {
            return Ok(());
        }
        Err(format!(
            "no persisted tool-error summary of class {class:?} with reason {reason:?}; saw {summaries:?}"
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
