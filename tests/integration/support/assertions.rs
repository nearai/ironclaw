//! Egress + tool-result + model-prompt assertions for [`RebornIntegrationHarness`].
//!
//! These read the captured Tier-2 `RuntimeHttpEgressRequest`s and recorded
//! capability results through the `pub(super)` accessors on the harness
//! (`captured_egress_requests` / `captured_capability_results`) rather than
//! re-reaching internals.
//!
//! The egress-assertion group (`assert_egress_count` / `assert_egress_url_order`
//! / `assert_egress_method_order` / `assert_egress_body_contains`) all assert
//! over the SAME captured `RecordingRuntimeHttpEgress` request log — there is
//! one runtime-lane egress-assertion API, not a parallel one. The one
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

use ironclaw_reborn_config::BudgetDefaults;
use ironclaw_resources::ResourceGovernor;
use rust_decimal::Decimal;

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

    /// Assert exactly `expected` requests were captured on the **network**
    /// egress lane (`captured_network_requests`) -- the lane
    /// `GithubIssueTools`-backed harnesses actually dispatch through (see
    /// `assert_network_egress_header_contains`'s docs). Sibling of
    /// `assert_egress_count`, which reads the runtime-egress lane instead;
    /// use this one for github/network-lane call-count proofs (e.g. that a
    /// cancelled or failed-resume run triggered no further dispatch).
    pub async fn assert_network_egress_count(&self, expected: usize) -> HarnessResult<()> {
        let actual = self.captured_network_requests().len();
        if actual == expected {
            return Ok(());
        }
        Err(format!("expected {expected} captured network egress request(s), saw {actual}").into())
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

    /// Assert that ANY captured egress request whose URL contains `url_substr`
    /// carried a body containing `body_substr` — checks every matching request,
    /// not just the first. Needed for a multi-request handshake where every leg
    /// shares the same URL (e.g. web-access's Exa MCP `initialize` /
    /// `notifications/initialized` / `tools/call` sequence, C-WEBACCESS) and
    /// only one leg's body carries the substring under test. Prefer
    /// [`assert_egress_body_contains`] when `url_substr` is expected to match
    /// exactly one request — its first-match semantics catch a false pass that
    /// this looser check would miss if a later, unrelated same-URL request also
    /// happened to satisfy `body_substr`.
    pub async fn assert_egress_body_contains_any(
        &self,
        url_substr: &str,
        body_substr: &str,
    ) -> HarnessResult<()> {
        let requests = self.captured_egress_requests();
        let matching: Vec<_> = requests
            .iter()
            .filter(|r| r.url.contains(url_substr))
            .collect();
        if matching.is_empty() {
            let seen: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
            return Err(format!(
                "no captured egress request matching url {url_substr:?}; saw {seen:?}"
            )
            .into());
        }
        if matching
            .iter()
            .any(|request| String::from_utf8_lossy(&request.body).contains(body_substr))
        {
            return Ok(());
        }
        let bodies: Vec<String> = matching
            .iter()
            .map(|request| String::from_utf8_lossy(&request.body).into_owned())
            .collect();
        Err(format!(
            "no egress request to {url_substr:?} had a body containing {body_substr:?}; saw {bodies:?}"
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

    /// Assert that some SINGLE model request this thread sent to the scripted
    /// provider contains EVERY needle in `needles` (all in one request, not
    /// spread across several). This is the multi-turn "sees prior context"
    /// proof: pass a needle unique to an earlier turn plus one unique to the
    /// current turn — only the current turn's request carries BOTH, because the
    /// earlier turn's own request predates the later text. Scanning with the
    /// single-needle [`assert_model_request_contains`] cannot express this (each
    /// needle would trivially match its own originating request), so a genuine
    /// context-carryover regression (the loop rebuilding the request without
    /// prior history) would slip through it but not through this.
    pub async fn assert_model_request_contains_all(&self, needles: &[&str]) -> HarnessResult<()> {
        let requests = self.scripted_llm.captured_requests();
        for messages in &requests {
            let rendered = serde_json::to_string(messages)
                .map_err(|e| format!("serialize captured model request: {e}"))?;
            if needles.iter().all(|needle| rendered.contains(needle)) {
                return Ok(());
            }
        }
        Err(format!(
            "no single model request contained all of {needles:?}; captured {} request(s)",
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
                // Fail loud, per .claude/rules/error-handling.md — see doc
                // comment above for why this must not silently skip.
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

    /// Assert the in-memory `TurnEventSink` installed via `.with_turn_event_sink()`
    /// (C-TRACECAP) recorded at least one event of `kind`. Proves
    /// `subscribe_best_effort` actually fired the sink for a real turn, not just
    /// that the harness wired the field.
    pub async fn assert_turn_event_recorded(
        &self,
        kind: ironclaw_turns::TurnEventKind,
    ) -> HarnessResult<()> {
        let events = self.recorded_turn_events();
        if events.iter().any(|event| event.kind == kind) {
            return Ok(());
        }
        let seen: Vec<_> = events.iter().map(|event| &event.kind).collect();
        Err(format!("no recorded turn event of kind {kind:?}; saw {seen:?}").into())
    }

    /// Assert a captured model request carried a multimodal `data:` image part
    /// holding exactly `bytes` under `mime_type` (C-ATTACH) — proves the landed
    /// attachment round-tripped intact (lander → project filesystem →
    /// `attachment_read_port` → base64) and reached the model as
    /// `ContentPart::ImageUrl`, not just the textual `<attachments>` pointer.
    pub async fn assert_model_saw_image_attachment(
        &self,
        mime_type: &str,
        bytes: &[u8],
    ) -> HarnessResult<()> {
        use base64::Engine;
        let urls = self.captured_image_data_urls();
        let expected = format!(
            "data:{mime_type};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        if urls.iter().any(|url| url == &expected) {
            return Ok(());
        }
        // Redacted: the full base64-encoded attachment bytes must never land in
        // CI logs on assertion failure (a future test using a sensitive/screenshot
        // fixture would otherwise leak its contents). Report mime type, byte
        // length, and a short digest instead — enough to distinguish "wrong bytes"
        // from "no image part at all" without reproducing the content.
        let seen: Vec<String> = urls.iter().map(|url| redact_data_url(url)).collect();
        Err(format!(
            "no captured image data: URL matching {}; saw {} image part(s): {seen:?}",
            redact_data_url(&expected),
            seen.len()
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

    /// C-BUDGET liveness assertion: the group's wired `model_budget_accountant`
    /// seeded the run owner's daily USD cap on the turn's first model call.
    ///
    /// Reads the in-memory `ResourceGovernor` retained behind the production
    /// `build_default_budget_accountant` accountant (wired via
    /// `with_budget_accounting()` / `budget_accounting()`). Before any turn the
    /// run-owner account does not exist; after a completed turn the accountant's
    /// `pre_model_call` has fired through the real coordinator → loop → model-port
    /// path and its compiled-default seeding policy has installed the daily cap.
    /// Asserting the cap equals the compiled default (`$5.00`) proves the value
    /// came from the production helper's `BudgetDefaults`, not an incidental path.
    ///
    /// This is wiring-liveness only — budget SEMANTICS (thresholds, gates,
    /// `BudgetEvent` cascade) are covered at crate tier (`budget_e2e.rs`).
    pub async fn assert_budget_user_cap_seeded(&self) -> HarnessResult<()> {
        let governor = self._shared.budget_governor.as_ref().ok_or(
            "harness was not built with budget accounting wired (call with_budget_accounting)",
        )?;
        let account = self
            ._shared
            .budget_account
            .as_ref()
            .ok_or("budget-accounting harness is missing its run-owner account")?;
        let snapshot = governor
            .account_snapshot(account)
            .map_err(|e| format!("budget account snapshot failed: {e}"))?
            .ok_or(
                "budget accountant never seeded the run owner's account \
                 (pre_model_call did not fire through the wired accountant)",
            )?;
        let limits = snapshot
            .limits
            .ok_or("budget account exists but carries no seeded limits")?;
        let expected = Decimal::from_f64_retain(BudgetDefaults::compiled_defaults().user_daily_usd)
            .unwrap_or_default();
        if limits.max_usd == Some(expected) {
            return Ok(());
        }
        Err(format!(
            "expected seeded user daily cap {expected:?} (compiled default), saw {:?}",
            limits.max_usd
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

/// Redact a `data:<mime>;base64,<bytes>` URL for safe inclusion in an assertion
/// failure message — never prints the base64 payload itself (which is the raw
/// attachment content) or even a prefix of it. Reports the mime type, decoded
/// byte length, and a short SHA-256 prefix, which is enough to tell "wrong
/// bytes" apart from "no image part at all" without reconstructing the content.
fn redact_data_url(url: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let Some(rest) = url.strip_prefix("data:") else {
        return "<non-data: URL>".to_string();
    };
    let Some((mime, b64)) = rest.split_once(";base64,") else {
        return format!(
            "data:{}...<unparseable, redacted>",
            rest.chars().take(40).collect::<String>()
        );
    };
    match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(bytes) => {
            let digest_hex: String = Sha256::digest(&bytes)
                .iter()
                .take(4)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!(
                "data:{mime};base64=<redacted, {} byte(s), sha256={digest_hex}>",
                bytes.len(),
            )
        }
        Err(_) => format!("data:{mime};base64=<redacted, undecodable>"),
    }
}

/// Multi-turn baseline-sliced variants of the thread-history assertions
/// (`assert_tool_error*`, conversation-history containment).
///
/// The full-history family above is safe only for single-turn harnesses (see
/// the `assert_tool_error` doc). These `*_since` methods close that gap using
/// the same `[baseline..]` slice idiom as the egress assertions, but over
/// thread-history message COUNT: capture [`history_len`] at the start of a
/// turn, then assert `*_since(baseline, ..)` after it.
impl RebornIntegrationHarness {
    /// Full persisted thread-history for this harness's thread, in sequence
    /// order. The baseline-sliced `*_since` assertions read this; kept private
    /// so tests assert through the typed helpers rather than raw records.
    async fn persisted_history(&self) -> HarnessResult<Vec<ironclaw_threads::ThreadMessageRecord>> {
        Ok(self
            .thread_harness
            .history(self.binding.thread_id.clone())
            .await?)
    }

    /// Number of persisted thread-history messages right now. Capture this at
    /// the START of a turn to scope a subsequent `*_since` assertion to only the
    /// messages that turn appends — the thread-history analogue of the
    /// `baseline_egress_count` snapshot the egress assertions take at
    /// construction, but caller-controlled so it can be re-taken per turn on a
    /// multi-turn harness.
    pub async fn history_len(&self) -> HarnessResult<usize> {
        Ok(self.persisted_history().await?.len())
    }

    /// Slice `history[baseline..]`, failing loud on an out-of-range `baseline` —
    /// a baseline greater than the current history length is always a caller bug
    /// (a stale or foreign-thread value; history never shrinks). Degrading it to
    /// an empty slice would turn that bug into a misleading "not found" (or a
    /// vacuous `assert_no_*` pass), violating the support tree's fail-loud
    /// contract.
    fn history_slice(
        history: &[ironclaw_threads::ThreadMessageRecord],
        baseline: usize,
    ) -> HarnessResult<&[ironclaw_threads::ThreadMessageRecord]> {
        history.get(baseline..).ok_or_else(|| {
            format!(
                "history baseline {baseline} exceeds current history length {} — stale or \
                 foreign-thread baseline",
                history.len()
            )
            .into()
        })
    }

    /// `persisted_tool_error_summaries`, but over only the `[baseline..]` slice —
    /// shared collector for the `*_since` tool-error assertions. Same fail-loud
    /// decode contract as the full-history collector above.
    async fn persisted_tool_error_summaries_since(
        &self,
        baseline: usize,
    ) -> HarnessResult<Vec<String>> {
        let history = self.persisted_history().await?;
        Self::history_slice(&history, baseline)?
            .iter()
            .filter(|message| message.kind == ironclaw_threads::MessageKind::ToolResultReference)
            .map(|message| {
                let Some(content) = message.content.as_deref() else {
                    return Err("ToolResultReference message missing content".into());
                };
                serde_json::from_str::<ironclaw_threads::ToolResultReferenceEnvelope>(content)
                    .map(|envelope| envelope.safe_summary.as_str().to_string())
                    .map_err(|err| {
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

    /// [`assert_tool_error`], scoped to the thread-history messages appended
    /// SINCE `baseline` (a value from [`history_len`] captured at the start of
    /// the turn under test). Use on a multi-turn harness where the full-history
    /// `assert_tool_error` would also see prior turns' tool errors.
    pub async fn assert_tool_error_since(
        &self,
        baseline: usize,
        class: ToolErrorClass,
        reason: &str,
    ) -> HarnessResult<()> {
        let summaries = self.persisted_tool_error_summaries_since(baseline).await?;
        let prefix = class.summary_prefix();
        if summaries
            .iter()
            .any(|summary| summary.starts_with(prefix) && summary.contains(reason))
        {
            return Ok(());
        }
        Err(format!(
            "no tool-error summary of class {class:?} with reason {reason:?} since baseline {baseline}; saw {summaries:?}"
        )
        .into())
    }

    /// [`assert_no_tool_error`], scoped to the `[baseline..]` slice — passes when
    /// NO tool error of `class` carrying `reason` was persisted since `baseline`.
    /// The multi-turn proof that a prior turn's error does NOT leak into the turn
    /// under test.
    pub async fn assert_no_tool_error_since(
        &self,
        baseline: usize,
        class: ToolErrorClass,
        reason: &str,
    ) -> HarnessResult<()> {
        let summaries = self.persisted_tool_error_summaries_since(baseline).await?;
        let prefix = class.summary_prefix();
        let matching: Vec<&String> = summaries
            .iter()
            .filter(|summary| summary.starts_with(prefix) && summary.contains(reason))
            .collect();
        if matching.is_empty() {
            return Ok(());
        }
        Err(format!(
            "expected no tool-error summary of class {class:?} with reason {reason:?} since baseline {baseline}; found {matching:?}"
        )
        .into())
    }

    /// [`assert_tool_error_summary_contains`], scoped to the `[baseline..]`
    /// slice — a raw `safe_summary` substring check with no class-prefix
    /// requirement, for the executor-synthesized literals documented on the
    /// full-history sibling, usable across turns.
    pub async fn assert_tool_error_summary_contains_since(
        &self,
        baseline: usize,
        text: &str,
    ) -> HarnessResult<()> {
        let summaries = self.persisted_tool_error_summaries_since(baseline).await?;
        if summaries.iter().any(|summary| summary.contains(text)) {
            return Ok(());
        }
        Err(format!(
            "no tool-error summary containing {text:?} since baseline {baseline}; saw {summaries:?}"
        )
        .into())
    }

    /// Shared implementation for the conversation-history containment asserts:
    /// scans the `[baseline..]` slice of thread history for a message whose
    /// `content` contains `needle`, optionally restricted to a single
    /// [`MessageKind`](ironclaw_threads::MessageKind) (role).
    async fn conversation_history_contains_impl(
        &self,
        baseline: usize,
        kind: Option<ironclaw_threads::MessageKind>,
        needle: &str,
    ) -> HarnessResult<()> {
        let history = self.persisted_history().await?;
        let slice = Self::history_slice(&history, baseline)?;
        let matched = slice
            .iter()
            .filter(|message| kind.is_none_or(|k| message.kind == k))
            .any(|message| {
                message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains(needle))
            });
        if matched {
            return Ok(());
        }
        let seen: Vec<String> = slice
            .iter()
            .filter(|message| kind.is_none_or(|k| message.kind == k))
            .map(|message| {
                let body = message.content.as_deref().unwrap_or("<no-content>");
                let body = match body.char_indices().nth(80) {
                    Some((cutoff, _)) => format!("{}...", &body[..cutoff]),
                    None => body.to_string(),
                };
                format!("{:?}:{body:?}", message.kind)
            })
            .collect();
        let scope = match kind {
            Some(k) => format!("{k:?}-role message"),
            None => "message".to_string(),
        };
        Err(format!(
            "no conversation-history {scope} containing {needle:?} since baseline {baseline}; saw {seen:?}"
        )
        .into())
    }

    /// Assert some persisted thread-history message's `content` contains
    /// `needle`, across the FULL history and ANY role. The general
    /// conversation-history containment check — the persisted-transcript
    /// analogue of [`assert_system_prompt_contains`] (which only reads
    /// System-role model REQUESTS, not persisted history). Reads user prompts,
    /// assistant replies, summaries, etc.
    pub async fn assert_conversation_history_contains(&self, needle: &str) -> HarnessResult<()> {
        self.conversation_history_contains_impl(0, None, needle)
            .await
    }

    /// [`assert_conversation_history_contains`], scoped to the `[baseline..]`
    /// slice (a [`history_len`] value from the start of the turn under test) —
    /// the multi-turn variant.
    pub async fn assert_conversation_history_contains_since(
        &self,
        baseline: usize,
        needle: &str,
    ) -> HarnessResult<()> {
        self.conversation_history_contains_impl(baseline, None, needle)
            .await
    }

    /// [`assert_conversation_history_contains`], restricted to messages of a
    /// single [`MessageKind`](ironclaw_threads::MessageKind) (role) across the
    /// full history — e.g. assert a `User` prompt or an `Assistant` reply landed
    /// in the transcript without matching the same text in another role.
    pub async fn assert_conversation_history_role_contains(
        &self,
        kind: ironclaw_threads::MessageKind,
        needle: &str,
    ) -> HarnessResult<()> {
        self.conversation_history_contains_impl(0, Some(kind), needle)
            .await
    }
}
