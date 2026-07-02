//! Egress + tool-result + model-prompt assertions for [`RebornIntegrationHarness`]
//! — the canonical, richer egress-assertion API (design §3.3 `assertions.rs`,
//! §3.6 P1 ergonomics).
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
//! The egress-assertion group (`assert_egress_count` / `assert_egress_url_order`
//! / `assert_egress_method_order` / `assert_egress_body_contains`) all assert
//! over the SAME captured `RecordingRuntimeHttpEgress` request log slice 2 wired
//! — there is one runtime-lane egress-assertion API, not a parallel one (the
//! O-egress MCP/OAuth interceptor folds its per-URL needs in here). The one
//! exception is `assert_network_egress_header_contains`, which reads the
//! recording *network* egress lane — required for the T0-SECRET-INJECT
//! credential-injection proof, whose harness routes through the host egress
//! pipeline over the network recorder (see that method's docs for why).
//! `assert_system_prompt_contains` reads a different capture source — the
//! scripted `TraceLlm`'s captured requests, via the harness's
//! `captured_system_prompts` accessor.

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

    /// Assert some model-visible `System`-role prompt captured across all
    /// requests captured by the harness so far contains `text`. Reads the
    /// scripted `TraceLlm` retained before the `dyn LlmProvider` upcast —
    /// proves prompt-injected content (safety banners, skill instructions,
    /// profile lines) actually reached the model.
    pub async fn assert_system_prompt_contains(&self, text: &str) -> HarnessResult<()> {
        let prompts = self.captured_system_prompts();
        if prompts.iter().any(|prompt| prompt.contains(text)) {
            return Ok(());
        }
        let seen: Vec<String> = prompts
            .iter()
            .map(|prompt| match prompt.char_indices().nth(200) {
                Some((cutoff, _)) => format!("{}...[truncated]", &prompt[..cutoff]),
                None => prompt.clone(),
            })
            .collect();
        Err(format!(
            "no captured system prompt containing {text:?}; saw {} system message(s): {seen:?}",
            prompts.len()
        )
        .into())
    }

    /// Assert that some model request this thread sent to the scripted provider
    /// contains `needle` anywhere in its serialized messages — the caller-tier
    /// proof that host-injected context (e.g. activated-skill instructions)
    /// actually reached the model. Reads the retained `TraceLlm`'s captured
    /// requests (E-SKILL half B).
    pub async fn assert_model_request_contains(&self, needle: &str) -> HarnessResult<()> {
        let requests = self.scripted_llm.captured_requests();
        for messages in &requests {
            let rendered = serde_json::to_string(messages)
                .map_err(|e| format!("serialize captured model request: {e}"))?;
            if rendered.contains(needle) {
                return Ok(());
            }
        }
        Err(format!(
            "no model request contained {needle:?}; captured {} request(s)",
            requests.len()
        )
        .into())
    }

    /// Collects the persisted `safe_summary` field of every `ToolResultReference`
    /// message on this thread's FULL history (not baseline-sliced — same caveat
    /// as `assert_tool_error`/`assert_tool_error_summary_contains`: safe only for
    /// single-turn harnesses today). Shared collector for [`assert_tool_error`],
    /// [`assert_no_tool_error`], and [`assert_tool_error_summary_contains`].
    ///
    /// A `ToolResultReference` message with `content: None`, or with `content`
    /// that fails to decode as a `ToolResultReferenceEnvelope`, is an `Err` —
    /// never silently skipped. Both would otherwise vanish from `summaries`
    /// and degrade into a misleading "not found; saw [...]" for the caller.
    async fn persisted_tool_error_summaries(&self) -> HarnessResult<Vec<String>> {
        let history = self
            .thread_harness
            .history(self.binding.thread_id.clone())
            .await?;
        history
            .iter()
            .filter(|message| message.kind == ironclaw_threads::MessageKind::ToolResultReference)
            .map(|message| {
                // Fail loud on a missing `content` field, and on a decode
                // error, rather than silently dropping the message
                // (.claude/rules/error-handling.md) — a malformed or
                // content-less envelope must surface as its own diagnosis,
                // not degrade into a misleading "not found" from the caller.
                let Some(content) = message.content.as_deref() else {
                    return Err("ToolResultReference message missing content".into());
                };
                serde_json::from_str::<ironclaw_threads::ToolResultReferenceEnvelope>(content)
                    .map(|envelope| envelope.safe_summary.as_str().to_string())
                    .map_err(|err| {
                        // Truncate the raw payload before interpolating it into
                        // the error: `content` can carry a `model_observation`
                        // field with large/unbounded text, which is bad for
                        // test-output size and potentially sensitive. Mirrors
                        // the truncation shape in `assert_system_prompt_contains`.
                        let truncated = match content.char_indices().nth(200) {
                            Some((cutoff, _)) => format!("{}...[truncated]", &content[..cutoff]),
                            None => content.to_string(),
                        };
                        format!(
                            "failed to decode ToolResultReferenceEnvelope: {err}; raw: {truncated}"
                        )
                        .into()
                    })
            })
            .collect()
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
        let summaries = self.persisted_tool_error_summaries().await?;
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

    /// Assert NO persisted `ToolResultReference` summary matches `class`'s
    /// prefix and contains `reason` — the inverse predicate of
    /// [`assert_tool_error`], built on the same `persisted_tool_error_summaries`
    /// collector. Use to prove a specific tool-error was NOT recorded (e.g. no
    /// leaked re-dispatch after a gate-declined short-circuit) without coupling
    /// the test to `assert_tool_error`'s own diagnostic wording.
    pub async fn assert_no_tool_error(
        &self,
        class: ToolErrorClass,
        reason: &str,
    ) -> HarnessResult<()> {
        let summaries = self.persisted_tool_error_summaries().await?;
        let prefix = class.summary_prefix();
        let matching: Vec<&String> = summaries
            .iter()
            .filter(|summary| summary.starts_with(prefix) && summary.contains(reason))
            .collect();
        if matching.is_empty() {
            return Ok(());
        }
        Err(format!(
            "expected no persisted tool-error summary of class {class:?} with reason {reason:?}; found {matching:?}"
        )
        .into())
    }

    /// Assert some persisted `ToolResultReference`'s raw `safe_summary` text
    /// contains `text` — NO class-prefix requirement. Complements
    /// [`assert_tool_error`] for `CapabilityErrorSummary`s the executor builds
    /// via `SanitizedStrategySummary::from_trusted_static` in
    /// `crates/ironclaw_agent_loop/src/executor/capabilities.rs` (filtered-surface
    /// denial, stale-surface retry, auth/approval gate-declined short-circuit) —
    /// those are fixed host-authored literals with no host-returned text to
    /// prefix, so `assert_tool_error`'s `capability_{failed,denied}_summary`
    /// prefix match can never succeed for them. Use only for known
    /// executor-synthesized literals.
    pub async fn assert_tool_error_summary_contains(&self, text: &str) -> HarnessResult<()> {
        let summaries = self.persisted_tool_error_summaries().await?;
        if summaries.iter().any(|summary| summary.contains(text)) {
            return Ok(());
        }
        Err(
            format!("no persisted tool-error summary containing {text:?}; saw {summaries:?}")
                .into(),
        )
    }

    /// Assert that any captured **network** egress request whose URL
    /// contains `url_substr` carried a header named `header_name`
    /// (case-insensitive) whose value contains `value_substr`. This is the
    /// credential-injection-on-the-wire proof for T0-SECRET-INJECT: a
    /// host-injected `Authorization: Bearer <token>` lands on the outbound
    /// request only after the egress pipeline's `apply_credential_injections`
    /// step, which the recording network egress captures.
    ///
    /// **Why the network lane, not the runtime lane:** the GitHub WASM harness
    /// (`with_github_issue_tools`) wires its recording `RuntimeHttpEgress` and
    /// then calls `try_with_host_http_egress`, which overwrites the runtime port
    /// with the host egress pipeline over the recording *network* egress. So the
    /// injected request flows through the network recorder, and the runtime-lane
    /// `assert_egress_*` family (which reads `runtime_http_requests()`) is inert
    /// for this wiring. Assert here instead.
    ///
    /// Checks only the `[baseline_network_count..]` delta so a group thread never
    /// spuriously matches a prior thread's request (R2), mirroring the runtime-lane
    /// `assert_egress_*` family's baseline discipline even though no group
    /// constructor wires `GithubIssueTools` today.
    pub async fn assert_network_egress_header_contains(
        &self,
        url_substr: &str,
        header_name: &str,
        value_substr: &str,
    ) -> HarnessResult<()> {
        let requests = self.captured_network_requests();
        let mut matching = requests
            .iter()
            .filter(|r| r.url.contains(url_substr))
            .peekable();
        if matching.peek().is_none() {
            let seen: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
            return Err(format!(
                "no captured network egress request matching url {url_substr:?}; saw {seen:?}"
            )
            .into());
        }
        let mut first_seen: Option<Vec<&str>> = None;
        for request in matching {
            if request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case(header_name) && value.contains(value_substr)
            }) {
                return Ok(());
            }
            if first_seen.is_none() {
                first_seen = Some(
                    request
                        .headers
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect(),
                );
            }
        }
        let seen = first_seen.unwrap_or_default();
        Err(format!(
            "no network egress request matching url {url_substr:?} has header {header_name:?} \
             with the expected value (redacted, not logged); header names present (first \
             matching request): {seen:?}"
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

    /// Return the parsed JSON `output` of the MOST RECENT recorded capability
    /// result for `capability_id` (baseline-sliced to this thread's turns).
    ///
    /// Unlike `assert_tool_result_contains`, this returns the value so a test can
    /// read a server-minted field — e.g. the `trigger_id` a `builtin.trigger_create`
    /// dispatch mints, which a static script cannot know ahead of time and which
    /// later `trigger_pause`/`resume`/`remove` turns must reference. Errors (never
    /// silently returns `Null`) when no result for `capability_id` was recorded.
    pub async fn tool_result_output(
        &self,
        capability_id: &str,
    ) -> HarnessResult<serde_json::Value> {
        let results = self.captured_capability_results();
        if let Some(result) = results
            .iter()
            .rev()
            .find(|result| result.capability_id.as_str() == capability_id)
        {
            return Ok(result.output.clone());
        }
        let seen: Vec<String> = results
            .iter()
            .map(|result| result.capability_id.as_str().to_string())
            .collect();
        Err(format!(
            "no recorded capability result for {capability_id:?}; saw results for {seen:?}"
        )
        .into())
    }
}
