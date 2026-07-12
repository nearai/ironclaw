# Q-10 Slack Canary Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Make Q-10 a reliable, attributable evaluation of Slack tool quality and live-model behavior while deterministically preventing raw Slack identifiers from reaching user-facing assistant output.

**Architecture:** Correct the model-visible tool contracts, wrap the product-live model gateway with a Slack-aware assistant-output policy, and divide live-QA results into blocking contracts and nonblocking behavioral observations. Q-10 correctness cases preactivate Slack and prove the expected capability actually ran; terminal UI state and typed provider failures replace synthetic-marker timeouts.

**Tech Stack:** Rust workspace crates, Python 3 live-QA harness, React/TypeScript WebUI v2, GitHub Actions, and recorded Reborn QA traces.

## Global Constraints

- No whole-case retry may turn a failed first answer into an unqualified pass.
- Raw Slack IDs remain available in capability results and capability-call arguments; only user-facing assistant text is sanitized.
- The output guard must not modify ParentLoopOutput::CapabilityCalls, stored capability results, or Slack message bodies sent through slack.send_message.
- A sanitizer intervention remains a failed model-quality observation; it must not make 10I look naturally compliant.
- Correctness cases must prove the intended capability completed before evaluating answer text.
- Final-turn state, not exact synthetic marker spelling, is the liveness primitive.
- Provider-unavailable runs are infrastructure incidents and never count as product passes or product regressions.
- Deterministic product contracts block; live behavioral observations preserve success=false but do not determine the process exit code.
- No live secrets, raw Slack IDs, names, email addresses, local paths, or PII may enter committed fixtures.
- Follow repository TDD rules: add the caller-level regression test first, observe the expected failure, then write the minimal production change.
- Keep the pre-existing main checkout and its uncommitted Q-10/Slack changes untouched.

---

### Task 1: Correct model-visible Slack and outbound-delivery contracts

**Files:**
- Modify: crates/ironclaw_reborn_composition/src/outbound/outbound_delivery_capability_surface.rs
- Modify: crates/ironclaw_first_party_extensions/assets/slack/manifest.toml
- Modify: crates/ironclaw_first_party_extensions/assets/slack/prompts/slack/search_messages.md
- Modify: crates/ironclaw_first_party_extensions/assets/slack/prompts/slack/list_conversations.md
- Modify: crates/ironclaw_first_party_extensions/assets/slack/prompts/slack/get_conversation_history.md
- Modify: crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs
- Modify: crates/ironclaw_reborn_composition/src/runtime/local_dev/tests.rs

**Interfaces:**
- Consumes: OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION and the bundled Slack manifest catalog.
- Produces: caller-visible descriptions that distinguish delivery routing from Slack reads and direct the model to is_member, newest-first history, humanized text, and display-name fields.

- [ ] **Step 1: Write failing caller-visible tests**

Extend tests that build the real local-dev provider tool and AvailableExtensionCatalog. Require these semantics:

~~~rust
assert!(outbound.description.contains("cannot read Slack conversations"));
assert!(search.description.contains("single newest message"));
assert!(search.description.contains("get_conversation_history"));
assert!(list.description.contains("is_member"));
assert!(list.description.contains("not only"));
assert!(history.description.contains("user_display_name"));
assert!(history.description.contains("is_current_user"));
~~~

The outbound test must inspect the actual provider tool definition, not only the constant.

- [ ] **Step 2: Run RED**

~~~bash
cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta \
  slack_read_descriptions -- --nocapture
cargo test -p ironclaw_reborn_composition \
  outbound_delivery_targets_list -- --nocapture
~~~

Expected: at least the delivery-routing and newest-message assertions fail.

- [ ] **Step 3: Implement minimal description corrections**

Use the same rules in manifest and prompt docs:

- outbound delivery targets route final replies and routine/trigger results only and cannot read Slack content, membership, status, or profiles;
- search.messages is indexed search and must not answer the single newest message when a conversation is known;
- list_conversations returns visible conversations and is_member=true is authoritative;
- history prose uses humanized text, user_display_name, and is_current_user; raw IDs are only for subsequent tool calls.

Do not change Slack schemas or WASM behavior.

- [ ] **Step 4: Run GREEN and commit**

~~~bash
cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta \
  slack_read_descriptions -- --nocapture
cargo test -p ironclaw_reborn_composition \
  outbound_delivery_targets_list -- --nocapture
git add crates/ironclaw_reborn_composition/src/outbound/outbound_delivery_capability_surface.rs \
  crates/ironclaw_first_party_extensions/assets/slack \
  crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs \
  crates/ironclaw_reborn_composition/src/runtime/local_dev/tests.rs
git commit -m "fix(reborn): clarify Slack capability selection"
~~~

---

### Task 2: Add a Slack-aware user-facing model-output guard

**Files:**
- Create: crates/ironclaw_reborn_composition/src/runtime/slack_output_hygiene.rs
- Modify: crates/ironclaw_reborn_composition/src/runtime.rs
- Test: crates/ironclaw_reborn_composition/src/runtime/slack_output_hygiene.rs
- Test: crates/ironclaw_reborn_composition/src/runtime/local_dev/tests.rs
- Verify unchanged: crates/ironclaw_host_runtime/tests/github_wasm_runtime_contract.rs

**Interfaces:**
- Consumes: HostManagedModelGateway, HostManagedModelRequest, HostManagedModelResponse, HostManagedModelStreamSink, LoopCapabilityPort, and ParentLoopOutput.
- Produces: SlackOutputHygieneGateway::new(Arc<dyn HostManagedModelGateway>).
- Produces: SLACK_IDENTIFIER_REDACTION with exact value [Slack identifier redacted].

- [ ] **Step 1: Write pure failing redaction tests**

Specify a private function:

~~~rust
fn redact_slack_identifiers(text: &str) -> String;
~~~

Required examples:

~~~rust
assert_eq!(redact_slack_identifiers("user U0123ABCDE"),
           "user [Slack identifier redacted]");
assert_eq!(redact_slack_identifiers("mention <@U0123ABCDE>"),
           "mention [Slack identifier redacted]");
assert_eq!(redact_slack_identifiers("legacy W0123ABCDE."),
           "legacy [Slack identifier redacted].");
assert_eq!(redact_slack_identifiers("short U123 and word BUILDING"),
           "short U123 and word BUILDING");
let once = redact_slack_identifiers("U0123ABCDE");
assert_eq!(redact_slack_identifiers(&once), once);
~~~

Recognize bounded U/W identifiers with at least eight uppercase ASCII letters/digits and token boundaries. Keep this logic out of ironclaw_turns and out of global tool-result sanitization.

- [ ] **Step 2: Run RED**

~~~bash
cargo test -p ironclaw_reborn_composition slack_output_hygiene -- --nocapture
~~~

Expected: missing module/function failure.

- [ ] **Step 3: Implement bounded redaction, then add failing decorator tests**

Define:

~~~rust
#[derive(Clone)]
pub(super) struct SlackOutputHygieneGateway {
    inner: Arc<dyn HostManagedModelGateway>,
}

impl SlackOutputHygieneGateway {
    pub(super) fn new(inner: Arc<dyn HostManagedModelGateway>) -> Self;
}

struct SlackOutputHygieneSink {
    inner: Arc<dyn HostManagedModelStreamSink>,
}
~~~

Tests must prove:

1. Slack-context assistant text, safe reasoning, and cumulative progress use the intervention marker.
2. ParentLoopOutput::CapabilityCalls retains arguments containing user_id U0123ABCDE and text <@U0123ABCDE> byte-for-byte.
3. Non-Slack requests are unchanged.
4. resolve_for_scope wraps a resolved inner gateway.

Determine Slack context from prior tool_result_provider_call.capability_id values and, in capability-aware methods, from visible Slack tool definitions. Never rewrite hydrated tool-result content.

- [ ] **Step 4: Run RED for the decorator**

~~~bash
cargo test -p ironclaw_reborn_composition slack_output_hygiene_gateway -- --nocapture
~~~

Expected: raw IDs remain or the decorator is missing.

- [ ] **Step 5: Implement all four gateway methods and wire composition**

The progress methods wrap the sink only for Slack context. All methods sanitize returned assistant/public fields only for Slack context. Forward requests, usage, capability ports, and CapabilityCalls unchanged. Declare the private module in runtime.rs and wrap the selected gateway before local-dev result hydration.

Do not change sanitize_model_visible_text because hydration uses it on model-visible tool results.

- [ ] **Step 6: Run GREEN and commit**

~~~bash
cargo test -p ironclaw_reborn_composition slack_output_hygiene -- --nocapture
cargo test -p ironclaw_reborn_composition \
  local_dev_runtime_exposes_host_runtime_capabilities_to_model_calls -- --nocapture
cargo test -p ironclaw_host_runtime --test github_wasm_runtime_contract \
  slack_history_output_carries_display_names_alongside_raw_user_ids -- --nocapture
git add crates/ironclaw_reborn_composition/src/runtime/slack_output_hygiene.rs \
  crates/ironclaw_reborn_composition/src/runtime.rs \
  crates/ironclaw_reborn_composition/src/runtime/local_dev/tests.rs
git commit -m "fix(reborn): guard Slack IDs in assistant output"
~~~

---

### Task 3: Make terminal replies and provider errors structural

**Files:**
- Modify: crates/ironclaw_webui_v2/frontend/src/pages/chat/components/message-bubble.tsx
- Modify: crates/ironclaw_webui_v2/frontend/src/pages/chat/components/message-bubble.test.ts
- Modify: scripts/reborn_webui_v2_live_qa/run_live_qa.py
- Modify: scripts/reborn_webui_v2_live_qa/test_run_live_qa.py

**Interfaces:**
- Consumes: ChatMessage.failureCategory, failureStatus, and isFinalReply.
- Produces: data-failure-category and data-failure-status on error bubbles.
- Produces: TerminalRunFailureObservation, _observe_terminal_run_failure, and final-state-first _wait_for_assistant_reply.

- [ ] **Step 1: Write and run the failing frontend test**

Render a real ErrorChatMessage with model_unavailable/failed and assert both data attributes.

~~~bash
cd crates/ironclaw_webui_v2/frontend
node --test --import tsx src/pages/chat/components/message-bubble.test.ts
~~~

Expected: the new attributes are absent.

- [ ] **Step 2: Render the two attributes and observe GREEN**

Derive them only for CHAT_MESSAGE_ROLES.ERROR and place them on the outer message element beside data-final-reply. Preserve existing styles and retry behavior.

- [ ] **Step 3: Write failing Python terminal-state tests**

Add:

~~~python
@dataclass(frozen=True)
class TerminalRunFailureObservation:
    summary: str
    failure_category: str | None
    failure_status: str | None
~~~

Test that:

- enforce_marker=False returns immediately for data-final-reply=true even when the synthetic marker is altered;
- enforce_marker=True fails immediately on a finalized reply missing the marker;
- a new error bubble after error_count_before raises a typed terminal failure;
- stale error bubbles at or before the baseline are ignored;
- the quiet fallback remains when final metadata is absent.

The fake page must implement every selector/attribute used by production.

- [ ] **Step 4: Run RED**

~~~bash
python3 -m unittest \
  scripts.reborn_webui_v2_live_qa.test_run_live_qa.LiveQaRunnerTests.test_wait_for_assistant_reply_returns_final_reply_when_marker_is_not_enforced \
  scripts.reborn_webui_v2_live_qa.test_run_live_qa.LiveQaRunnerTests.test_wait_for_assistant_reply_raises_terminal_model_failure_without_waiting
~~~

Expected: missing API/type or timeout-path assertion failure.

- [ ] **Step 5: Implement terminal observation**

Implement _observe_terminal_run_failure with the exact parameters
(page: object, *, baseline_count: int = 0) and return type
TerminalRunFailureObservation | None. Implement _wait_for_assistant_reply with
the existing page, marker, required_text, timeout, and semantic_goal parameters,
plus error_count_before: int = 0 and enforce_marker: bool = True, retaining the
AssistantReplyWaitResult return type.

_live_chat_case records the error-bubble count before submit and persists category/status from a typed terminal exception. Q-10 will set enforce_marker=False while retaining seeded content assertions.

- [ ] **Step 6: Run GREEN and commit**

~~~bash
(cd crates/ironclaw_webui_v2/frontend && \
  node --test --import tsx src/pages/chat/components/message-bubble.test.ts)
python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
git add crates/ironclaw_webui_v2/frontend/src/pages/chat/components/message-bubble.tsx \
  crates/ironclaw_webui_v2/frontend/src/pages/chat/components/message-bubble.test.ts \
  scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
git commit -m "fix(canary): observe terminal model failures"
~~~

---

### Task 4: Add typed contract/behavioral aggregation and reporting

**Files:**
- Modify: scripts/reborn_webui_v2_live_qa/case_matrix.py
- Modify: scripts/reborn_webui_v2_live_qa/run_live_qa.py
- Modify: scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
- Modify: scripts/live-canary/notify_slack.py
- Modify: scripts/live-canary/test_notify_slack.py

**Interfaces:**
- Produces: CaseSpec.tier in contract/behavioral and CaseSpec.blocking.
- Produces: result details case_tier, blocking, failure_class, failure_category, and failure_status.
- Produces: notifier advisory counts that do not feed blocking issue creation.

- [ ] **Step 1: Write failing metadata, exit, and outage tests**

Construct:

~~~python
contract = CaseSpec(fake_case, tier="contract", blocking=True)
behavioral = CaseSpec(fake_case, tier="behavioral", blocking=False)
~~~

Assert invalid tier raises ValueError; manifest and result details carry both fields; both observed failures remain success=False; behavioral-only failure exits 0; contract failure exits 1; model_unavailable is infrastructure and short-circuits later cases as inconclusive without starting their servers.

- [ ] **Step 2: Run RED**

~~~bash
python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
~~~

Expected: tier/exit/short-circuit tests fail.

- [ ] **Step 3: Implement metadata and exit policy**

Extend CaseSpec with tier and blocking. Have _result copy the selected CaseSpec metadata and write_case_manifest emit it. Use:

~~~python
def _is_blocking_failure(result: ProbeResult) -> bool:
    return not result.success and bool(result.details.get("blocking", True))
~~~

Exit 1 only for blocking failures. Classify provider outages from structured category, append explicit inconclusive results for remaining cases, and break.

- [ ] **Step 4: Write and run failing notifier tests**

A behavioral failure must render as a warning, preserve its failure message and tool trace, not increment blocking failures, not create a canary issue, and preserve PR/SHA context.

~~~bash
python3 scripts/live-canary/test_notify_slack.py
~~~

Expected: behavioral failure is counted as blocking or warning fields are absent.

- [ ] **Step 5: Implement notifier warnings**

Extend RebornQaCaseReport/LaneReport and parser/rendering paths. Structured metadata is authoritative over optional Haiku enrichment. Issue creation and failure categorization remain limited to blocking failures.

- [ ] **Step 6: Run GREEN and commit**

~~~bash
python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
python3 scripts/live-canary/test_notify_slack.py
python3 scripts/live-canary/test_run_dispatch.py
git add scripts/reborn_webui_v2_live_qa/case_matrix.py \
  scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  scripts/live-canary/notify_slack.py scripts/live-canary/test_notify_slack.py
git commit -m "fix(canary): classify behavioral and infrastructure results"
~~~

---

### Task 5: Isolate Q-10 Slack correctness journeys

**Files:**
- Modify: scripts/reborn_webui_v2_live_qa/run_live_qa.py
- Modify: scripts/reborn_webui_v2_live_qa/case_matrix.py
- Modify: scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
- Modify: .github/workflows/live-canary.yml

**Interfaces:**
- Produces: SLACK_EXTENSION_REQUIREMENT.
- Extends: _live_chat_case with extensions and enforce_marker parameters.
- Extends: _slack_correctness_chat_reply with expected_capability.
- Produces: blocking scoped case_qa_10g_slack_last_message_sent and nonblocking case_qa_10g_slack_last_message_sent_global.

- [ ] **Step 1: Write failing preactivation and capability-evidence tests**

Drive _slack_correctness_chat_reply and prove _live_chat_case reaches _ensure_extension_authenticated_on_page with:

~~~python
SLACK_EXTENSION_REQUIREMENT = {
    "package_id": "slack",
    "display_name": "Slack",
    "required_tools": [
        "slack.list_conversations",
        "slack.get_conversation_history",
    ],
}
~~~

Add 10D tests: a correct-looking answer without a new completed slack.list_conversations call fails model_quality; a completed call plus correct membership succeeds.

- [ ] **Step 2: Write failing scoped/global 10G and 10I tests**

The blocking 10G prompt names the seeded channel and requires a new completed slack.get_conversation_history call. The global case keeps the original wording and registers tier=behavioral, blocking=False.

10I requires the display-name token and rejects raw U/W identifiers plus [Slack identifier redacted]. It registers behavioral/nonblocking and never retries.

- [ ] **Step 3: Run RED**

~~~bash
python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
~~~

Expected: preactivation, expected-capability, scoped/global, and intervention tests fail.

- [ ] **Step 4: Implement shared preactivation and evidence**

Fold extension setup into _live_chat_case and make _live_chat_with_extensions_case delegate to it. _slack_correctness_chat_reply passes the Slack requirement, sets enforce_marker=False, and compares expected capability completed counts before and after chat.

Classify missing capability as model_quality, answer/ground-truth mismatch as product, terminal provider errors as infrastructure, and invalid fixtures as precondition.

- [ ] **Step 5: Register scoped/global 10G and strict 10I**

Keep QA row 10G stable for the scoped contract; map the new global case to 10G for reporting. Add both to workflow selection and coverage tests. Mark 10I behavioral/nonblocking; Task 2 supplies the blocking safety guarantee.

- [ ] **Step 6: Run GREEN and commit**

~~~bash
python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
python3 scripts/live-canary/test_run_dispatch.py
git add scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/reborn_webui_v2_live_qa/case_matrix.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  .github/workflows/live-canary.yml
git commit -m "fix(canary): isolate Q-10 Slack journeys"
~~~

---

### Task 6: Add recorded behavior coverage, verify, and publish

**Files:**
- Modify: tests/reborn_qa_recorded_behavior.rs
- Modify only if production-equivalent wiring requires it: tests/support/reborn_parity_qa/qa_trace.rs
- Create: tests/fixtures/llm_traces/reborn_qa/slack_channel_membership.json
- Create: tests/fixtures/llm_traces/reborn_qa/slack_recent_message.json
- Create: tests/fixtures/llm_traces/reborn_qa/slack_entity_hygiene.json
- Verify/update only if required: FEATURE_PARITY.md and CHANGELOG.md

**Interfaces:**
- Consumes: record_qa_phrase, load_qa_trace, recorded_tool_calls, strip_expected_tool_results, send_qa_phrase, and RebornTraceReplayModelGateway::from_trace.
- Produces: scrubbed Q-10D/G/I tool-choice fixtures and hermetic contracts.

- [ ] **Step 1: Write failing fixture contracts before fixtures**

Require slack.list_conversations for membership, slack.get_conversation_history with synthetic conversation D0CANARY for recent-message retrieval, a final display name Canary User with no raw ID, and preserved synthetic raw ID U0CANARY in a capability call where chaining needs it.

- [ ] **Step 2: Run RED**

~~~bash
cargo test --test reborn_qa_recorded_behavior --features libsql -- --nocapture
~~~

Expected: missing fixture failures.

- [ ] **Step 3: Add scrubbed real-sequence fixtures**

Derive sequences from an attended recorder or exact-head live artifact; replace every live name/ID with synthetic values. Do not invent a tool path merely to satisfy the assertion. If replay cannot reach Slack without test-only production wiring, keep fixture-level tool-choice contracts plus the real WASM caller contract and explain the crate-tier fallback in the PR.

- [ ] **Step 4: Run scrub and recorded GREEN**

~~~bash
scripts/ci/check-reborn-qa-fixtures.sh
cargo test --test reborn_qa_recorded_behavior --features libsql -- --nocapture
~~~

- [ ] **Step 5: Run final deterministic verification**

~~~bash
python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
python3 scripts/live-canary/test_notify_slack.py
python3 scripts/live-canary/test_run_dispatch.py
(cd crates/ironclaw_webui_v2/frontend && \
  node --test --import tsx src/pages/chat/components/message-bubble.test.ts)
cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta -- --nocapture
cargo test -p ironclaw_host_runtime --test github_wasm_runtime_contract slack_ -- --nocapture
cargo fmt --all -- --check
bash scripts/check-boundaries.sh
cargo clippy -p ironclaw_reborn_composition --all-targets \
  --features slack-v2-host-beta -- -D warnings
~~~

Check FEATURE_PARITY.md, relevant specs/API docs, and CHANGELOG.md and update only if the implemented behavior changes a tracked contract/status.

- [ ] **Step 6: Commit deterministic coverage**

~~~bash
git add tests/reborn_qa_recorded_behavior.rs \
  tests/support/reborn_parity_qa/qa_trace.rs \
  tests/fixtures/llm_traces/reborn_qa
git commit -m "test(reborn): pin Q-10 Slack behavior"
~~~

Stage only files that changed.

- [ ] **Step 7: Open a draft PR and dispatch exact-head Q-10**

Push the branch and open a draft PR. Dispatch trusted main workflow code with lane=reborn-webui-v2-live-qa, the Q-10 case list including qa_10g_slack_last_message_sent_global, target_ref equal to the exact PR head, and use_target_harness=true.

The exact SHA needs an approving review from a write-capable collaborator before live secrets can run. Any new commit invalidates approval.

- [ ] **Step 8: Validate three consecutive exact-head runs**

Require three consecutive runs whose blocking Q-10 contracts pass. Provider-unavailable runs are inconclusive. Inspect artifacts to confirm 10D used slack.list_conversations, scoped 10G used slack.get_conversation_history, 10I exposed neither raw IDs nor the intervention marker when naturally compliant, behavioral failures stayed visible with success=false, and no first-attempt failure was hidden.

Only then mark the PR ready for review.
