# Ironclaw TDD Playbook for Engineers

This playbook explains which tests to write, where they belong, and how to
develop a feature or bug fix test-first without adding unnecessary process.

It is written for engineers who are new to Ironclaw. Repository and crate-local
guidance remains authoritative when it is more specific. Start with
`.claude/rules/testing.md`; for cross-layer Reborn tests, also read
`tests/integration/CLAUDE.md`.

## Core rule

Start with what the user should experience, write a test for that behavior,
and implement the smallest amount of code needed to make the test pass.

Do not write every kind of test for every change. Select tests based on the
behavior and its risks.

## The four test types

### 1. Unit or contract test

A unit or contract test proves one local rule or one crate-owned public
contract.

Use it for:

- pure logic and validation
- typed errors and state transitions
- isolated bug fixes
- public API or policy tables owned by one crate

Examples include rejecting an invalid cron expression or denying an operation
when the caller lacks permission.

Run the owning crate's tests:

```bash
cargo test -p OWNING_CRATE
```

### 2. Hermetic feature test

A hermetic feature test proves a complete Ironclaw behavior without calling a
real model or external service. The result should be repeatable and require no
API keys.

The Reborn integration harness scripts the vendor model response while keeping
the real product workflow, scheduler, agent loop, LLM decorator chain,
permissions, capabilities, and persistence in the path.

Use this for most new or changed production-wired Reborn behavior.

```bash
cargo test --test reborn_integration_SCENARIO
```

### 3. Surface test

A surface test proves behavior through the external surface affected by the
change. Choose only the relevant kind:

- **Recorded model fixture:** the model must choose a particular tool or send
  particular arguments.
- **Browser E2E:** the user can see or interact with the behavior in WebUI.
- **Backend or runtime integration:** the behavior depends on PostgreSQL,
  libSQL, Docker, WASM, MCP, or another runtime.

Many local changes do not need a surface test.

### 4. Live canary

A live canary runs a small scenario against a real model or provider. It finds
model drift, provider API changes, credential failures, and unexpected prompt
behavior.

Live canaries are supplemental. They must not be the only test for a feature,
and every pull request should not have to wait for one.

## How to choose a test

Ask these questions:

1. Can the user see or click the changed behavior in WebUI? Add a browser E2E
   test.
2. Does the model need to choose a particular tool or arguments? Add a
   recorded fixture and replay test.
3. Does the behavior cross Ironclaw components or perform a side effect? Add a
   hermetic Reborn integration test.
4. Is the change only local logic? Add a unit or contract test.
5. Does success depend on the current behavior of a real model or provider?
   Add or run a live canary after deterministic tests pass.

A change can match more than one question. For example, a WebUI approval
feature may need both a hermetic integration test and a browser test.

## Step-by-step workflow

### Step 1: Describe the user behavior

Write one short Given/When/Then example. Describe an observable result, not an
internal function call.

> Given a user who must approve file writes, when Ironclaw tries to write a
> file, then the turn waits for approval, the file does not exist before
> approval, and the file exists after approval.

### Step 2: Identify the risks

Mark the risks that apply:

- model behavior
- browser behavior
- side effect
- persistence
- security or permissions
- external provider
- cross-component behavior

These risks determine which test types are needed.

### Step 3: Choose the highest-level deterministic test

For most production-wired Reborn behavior, start with a hermetic Reborn
integration test. A live canary can be the first scenario designed, but it
should not be the first or only automated test.

### Step 4: Make the test fail

Run the test before implementing the feature. Confirm it fails because the
behavior is missing, not because the test contains a typo or broken setup.

For a bug fix, the test must reproduce the original bug.

### Step 5: Write the smallest fix

Implement only enough code to make the test pass. Avoid unrelated refactoring
and speculative features.

### Step 6: Add important edge cases

Add smaller tests when they improve diagnosis or protect an important rule:

- invalid input
- missing permission
- cancellation
- duplicate requests
- partial failure
- wrong user or tenant
- persistence after restart

Do not repeat the same happy path at every test level.

### Step 7: Run tests from fast to slow

Run:

1. unit and contract tests
2. hermetic Reborn integration tests
3. recorded fixture replay, browser E2E, or backend/runtime integration
4. live canary

This keeps the development loop fast while preserving outside-in coverage.

## Where tests belong

| Behavior | Location | Notes |
| --- | --- | --- |
| Private helper or pure local rule | `crates/<owning_crate>/src/` in `#[cfg(test)] mod tests` | Keep the test next to the implementation. |
| Public crate contract | `crates/<owning_crate>/tests/<behavior>_contract.rs` | Test through the crate's public API. |
| Whole Reborn turn or cross-component behavior | `tests/integration/` | Use the scripted-model harness and assert at a meaningful seam. |
| Model tool choice or request shape | `tests/fixtures/llm_traces/reborn_qa/` and `tests/reborn_qa_recorded_behavior.rs` | Commit only scrubbed fixtures. |
| WebUI behavior | `tests/e2e/scenarios/test_<behavior>.py` | Use the Reborn v2 fixtures for WebChat v2. |
| Database or runtime behavior | Owning crate's feature-gated integration suite, or an existing root integration suite | Cover supported production backends. |
| Real model or provider drift | `scripts/reborn_webui_v2_live_qa/` | Reuse the current live-QA lane when possible. |

## Codebase examples

### Unit or crate contract

Example:
[`crates/ironclaw_webui/tests/webui_v2_descriptors_contract.rs`](../../crates/ironclaw_webui/tests/webui_v2_descriptors_contract.rs)

This contract locks the declared WebChat v2 route surface, including method,
path, authentication, body limit, rate limit, CORS, audit class, and allowed
effect path.

The test follows this shape:

```rust
#[test]
fn route_table_has_exactly_the_expected_routes() {
    let routes = webui_v2_routes();
    let expected = expected_table();
    assert_eq!(
        routes.len(),
        expected.len(),
        "expected {} WebChat v2 routes, found {}",
        expected.len(),
        routes.len()
    );
}
```

Put a similar test inline when it covers a private helper. Put it under the
owning crate's `tests/` directory when it protects a public contract or
caller-facing behavior.

```bash
cargo test -p ironclaw_webui
```

### Hermetic Reborn integration

Simple example: [`tests/integration/greeting.rs`](../../tests/integration/greeting.rs)

This proves that a synthetic inbound message travels through product workflow,
scheduling, the agent loop, the real LLM decorator chain, and persisted thread
history. Only the vendor model response is scripted.

```rust
let harness = RebornIntegrationHarness::test_default()
    .script([RebornScriptedReply::text("Hello! How can I help?")])
    .build()
    .await
    .expect("harness builds");
harness
    .submit_turn("hi there")
    .await
    .expect("turn completes");
harness
    .assert_reply_contains("Hello! How can I help?")
    .await
    .expect("reply finalized in thread history");
```

For a distinct scenario, add `tests/integration/<scenario>.rs` and register the
flat test binary in the root `Cargo.toml` as `reborn_integration_<scenario>`.

If a scenario shares expensive setup with an existing group, add it to the
group directory and include the module from that group's `main.rs` instead of
creating another harness.

```bash
cargo test --test reborn_integration_greeting
```

### Caller-path side effect

Example:
[`tests/integration/group_approvals/scenario_gate_then_approve.rs`](../../tests/integration/group_approvals/scenario_gate_then_approve.rs)

This drives the real approval and resume path and asserts that the approved
file write actually persisted. It does not stop at `Completed` status or a mock
call count.

```rust
let (run_id, gate_ref) = h
    .submit_turn_until_blocked("write the approval file")
    .await?;
h.approve_gate(run_id, &gate_ref).await?;
h.wait_for_status(run_id, TurnStatus::Completed).await?;
h.assert_workspace_file_contains("approved.txt", "approved write")
    .await?;
```

Use this caller-path shape whenever a helper controls persistence, egress,
dispatch, approval, secrets, or another side effect.

```bash
cargo test --test reborn_group_approvals
```

### Recorded model fixture

Examples:

- fixture:
  [`tests/fixtures/llm_traces/reborn_qa/web_status_check.json`](../../tests/fixtures/llm_traces/reborn_qa/web_status_check.json)
- contract and replay:
  [`tests/reborn_qa_recorded_behavior.rs`](../../tests/reborn_qa_recorded_behavior.rs)

The contract proves that the recorded model response selected `builtin.http`
with the expected target:

```rust
let trace = load_qa_trace(WEB_STATUS_CHECK.fixture);
assert_tool_called_with(&trace, "builtin.http", &["api.github.com"]);
```

Add tool-choice and key-argument assertions to
`tests/reborn_qa_recorded_behavior.rs`. Add a replay assertion when the trace
should create or modify durable state. Live recorders stay ignored; contract
and replay tests run hermetically in CI.

```bash
scripts/ci/check-reborn-qa-fixtures.sh
cargo test --test reborn_qa_recorded_behavior
```

### Browser E2E

Example:
[`tests/e2e/scenarios/test_reborn_webui_v2_smoke.py`](../../tests/e2e/scenarios/test_reborn_webui_v2_smoke.py)

This starts the standalone Reborn server and proves that an authenticated user
reaches the chat shell while an anonymous user reaches the login screen.

```python
async def test_reborn_v2_serves_shell_and_gates_auth(
    reborn_v2_server, reborn_v2_browser
):
    authed_ctx = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720}
    )
    authed_page = await authed_ctx.new_page()
    await authed_page.goto(
        f"{reborn_v2_server}/?token={REBORN_V2_AUTH_TOKEN}"
    )
    await expect(authed_page.locator(SEL_V2["chat_composer"])).to_be_visible()
```

For Reborn WebChat v2, use the `reborn_v2_*` fixtures and `SEL_V2` selectors.
If the scenario must be part of the Reborn coverage gate, add its file or
pytest node ID to
[`tests/e2e/reborn_coverage_tests.txt`](../../tests/e2e/reborn_coverage_tests.txt).

```bash
cd tests/e2e
pytest scenarios/test_reborn_webui_v2_smoke.py
```

### Database or runtime integration

Example:
[`crates/ironclaw_hooks/tests/parity_matrix.rs`](../../crates/ironclaw_hooks/tests/parity_matrix.rs)

This is the right shape when multiple backends must implement the same
behavioral contract. Production-facing persistence behavior should cover both
libSQL and PostgreSQL unless the owning contract explicitly says otherwise.

Prefer the owning crate's `tests/` directory for one storage or runtime
contract. Use an existing feature-gated root integration suite when behavior
crosses composition layers.

Never silently return from a test because Docker or PostgreSQL is missing. Use
documented feature gates or a loud, explicit opt-out.

```bash
cargo test --features integration
```

### Live canary

Examples:

- implementation:
  [`scripts/reborn_webui_v2_live_qa/run_live_qa.py`](../../scripts/reborn_webui_v2_live_qa/run_live_qa.py)
- workflow: [`.github/workflows/live-canary.yml`](../../.github/workflows/live-canary.yml)

The existing `qa_3b_endpoint_status_live_chat` case asks Ironclaw to check
whether `near.ai` returns HTTP 200 and verifies that the current status is
reported.

Add or extend the case under `scripts/reborn_webui_v2_live_qa/`. Reuse the
current `reborn-webui-v2-live-qa` lane when possible. Change the workflow only
when a case needs new shard, secret, schedule, or lane wiring.

Authorized maintainers can run one case from a pull request:

```text
/canary cases=qa_3b_endpoint_status_live_chat
```

Keep mutations isolated and reversible, and scrub uploaded artifacts.

## Worked example

Feature: a user asks Ironclaw to create a routine that checks a website every
hour.

Possible coverage:

1. **Unit test:** reject an invalid schedule.
2. **Hermetic integration:** script the routine-creation tool call and verify
   the routine is persisted with the correct schedule.
3. **Recorded fixture:** verify a real model chooses the routine tool with the
   expected URL and schedule.
4. **Live canary:** ask the current production model to create the routine and
   verify it succeeds.

A browser test is unnecessary unless the feature changes how routines appear
or behave in WebUI.

## Pull request test card

The pull request template includes this test card. Complete every field before
requesting review; do not remove the section.

```markdown
### Test card

User behavior:

Risk areas:
- [ ] Model behavior
- [ ] Browser
- [ ] Side effect
- [ ] Persistence
- [ ] Security or permissions
- [ ] External provider
- [ ] Cross-component behavior

Tests added or updated:
- Unit or contract:
- Reborn integration:
- Recorded fixture:
- Browser E2E:
- Backend or runtime:
- Live canary:

What the tests prove:

Commands run:
```

For every unused field, write `Not applicable: <reason>`. If an expected test
layer is omitted, explain why in one sentence.

## Before creating a new test file

- Search for an existing test that already drives the same caller or workflow.
- Read the owning crate's `AGENTS.md`, `CLAUDE.md`, `CONTRACT.md`, or
  `README.md`.
- Read `tests/integration/CLAUDE.md` before changing the Reborn integration
  harness.
- Write or update the test first and confirm it fails for the expected reason.
- Assert an observable outcome, not only `Completed` status or a mock call
  count.
- Run the narrowest test during development, then expand based on risk.
- Add `cargo test -p ironclaw_architecture` when dependency or ownership edges
  change.
- Use `bash scripts/reborn-e2e-rust.sh` when a Reborn contract or whole-path
  behavior changes.

## Rules to remember

- Most production changes need a unit or contract test plus a hermetic feature
  test.
- Every reproducible bug fix needs a regression test.
- Test real outcomes: files written, records persisted, events emitted,
  requests captured, or permissions enforced.
- Test through the real caller when permissions or side effects are involved.
- Use recorded fixtures only when model behavior matters.
- Use browser tests only when browser behavior matters.
- Live canaries supplement deterministic tests; they never replace them.
- Extend an existing test when it already covers the same workflow.

## Provider fault profiles

Use `tests/e2e/provider_fault_proxy.py` when a provider operation must cross
the real Reborn extension and network path while Emulate retains authoritative
provider state. The proxy supplies reusable HTTP, malformed-response, timeout,
connection-reset, and lost-acknowledgement profiles. Its ledger records only
request metadata, body digests, and credential fingerprints.

Apply profiles by operation equivalence class instead of multiplying every
provider operation by every failure. A representative read, idempotent write,
and non-idempotent write must assert the model-visible result, proxy attempt
count, whether the provider received the request, and direct provider
readback. A lost-acknowledgement test must prove the provider committed while
the runtime did not report success, and must prove that no blind duplicate
request occurred.

Keep missing credentials, credential refresh, and account-scope behavior at
their existing auth/runtime seams when a provider proxy cannot create the
condition faithfully. Fault state must be reset independently from provider
state after every case.
