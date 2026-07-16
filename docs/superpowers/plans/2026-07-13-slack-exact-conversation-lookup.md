# Slack Exact Conversation Lookup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate QA-10F's wrong-DM/wrong-mention flake by giving the Slack extension an exact conversation-ID lookup and requiring that lookup in the live contract.

**Architecture:** Add `slack.get_conversation_info` entirely inside the first-party Slack extension. It calls Slack's `conversations.info` endpoint for a known conversation ID and returns the DM counterpart's authoritative `user` field, while `slack.list_conversations` remains the discovery tool for unknown IDs. The core Reborn runtime stays provider-neutral.

**Tech Stack:** Rust/WASI component, Reborn extension manifests and JSON Schema, Rust host-runtime contract tests, Python live-QA harness.

## Global Constraints

- Keep Slack-specific behavior inside `crates/ironclaw_first_party_extensions/assets/slack/` and extension asset packaging.
- Preserve existing Slack OAuth scopes; `conversations.info` uses the already granted conversation-read scopes.
- Add a caller-level test that dispatches the real bundled WASM through `HostRuntime::invoke_capability` and asserts the egress URL plus returned counterpart.
- Keep QA-10F's real Slack side-effect/readback assertion and additionally require a completed exact lookup.
- Rebuild and commit `wasm/slack_user_tool.wasm` from the changed source.

---

### Task 1: Pin the exact lookup contract red

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`
- Modify: `crates/ironclaw_host_runtime/tests/github_wasm_runtime_contract.rs`
- Modify: `scripts/reborn_webui_v2_live_qa/test_run_live_qa.py`
- Modify: `tests/reborn_qa_recorded_behavior.rs`
- Create: `tests/fixtures/llm_traces/reborn_qa/slack_mention_encoding.json`

**Interfaces:**
- Consumes: existing Slack extension package loader and `UrlKeyedSlackEgress` caller harness.
- Produces: failing assertions for `slack.get_conversation_info`, `conversations.info?channel=...`, and QA-10F's required capability.

- [ ] **Step 1: Extend the model-visible manifest contract test**

Add assertions that the package exposes `slack.get_conversation_info`, describes it as the exact known-ID path, and tells the model that a DM response's `user` is the authoritative mention target. Change the existing `slack.send_message` assertion from `slack.list_conversations` to the new exact lookup for known DM IDs.

- [ ] **Step 2: Add the caller-level host-runtime test**

Dispatch:

```rust
wasm_runtime_request_for_scope(
    CapabilityId::new("slack.get_conversation_info").unwrap(),
    scope,
    json!({"channel": "D0FIRAT"}),
)
```

Script `conversations.info?channel=D0FIRAT` to return a DM with `user: U0BBB`, script `users.info?user=U0BBB`, then assert the completed output has `conversation.id == "D0FIRAT"`, `conversation.user == "U0BBB"`, and `conversation.user_display_name == "Ada Lovelace"`. Assert the recorded egress includes the exact encoded conversation lookup.

- [ ] **Step 3: Pin QA-10F to the new capability**

Update the existing Python source/argument contract so `case_qa_10f_slack_mention_encoding` must pass `expected_capability="slack.get_conversation_info"` to `_slack_correctness_chat_reply`.

Add a hermetic recorded-QA contract whose synthetic trace pins `slack.get_conversation_info(channel=D0CANARY) → slack.send_message(channel=D0CANARY, text containing <@U0CANARY>)` and rejects list scanning for the known ID.

- [ ] **Step 4: Run the red tests**

Run:

```bash
cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta slack_send_message_description_states_host_owned_final_reply_delivery
cargo test -p ironclaw_host_runtime --test github_wasm_runtime_contract slack_get_conversation_info_resolves_exact_dm_counterpart
python3 -m unittest scripts.reborn_webui_v2_live_qa.test_run_live_qa.RebornWebUiV2LiveQaRunnerTests.test_blocking_qa_10_cases_declare_intended_slack_capability
```

Expected: all three fail because the capability and QA requirement do not exist yet.

### Task 2: Implement the extension-owned exact conversation capability

**Files:**
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml`
- Create: `crates/ironclaw_first_party_extensions/assets/slack/prompts/slack/get_conversation_info.md`
- Create: `crates/ironclaw_first_party_extensions/assets/slack/schemas/slack/get_conversation_info.input.v1.json`
- Create: `crates/ironclaw_first_party_extensions/assets/slack/schemas/slack/get_conversation_info.output.v1.json`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/prompts/slack/send_message.md`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/schemas/slack/send_message.input.v1.json`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/wasm-src/src/types.rs`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/wasm-src/src/api.rs`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/wasm-src/src/lib.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`

**Interfaces:**
- Consumes: Slack Web API `GET conversations.info?channel=<id>` and the existing `Conversation` response type/name-enrichment helper.
- Produces: `slack.get_conversation_info` with input `{ channel: String }` and output `{ ok: true, conversation: Conversation }`.

- [ ] **Step 1: Declare and package the capability**

Add the manifest capability using the same read-only OAuth requirements as `slack.list_conversations`. Package its two schemas and prompt in `slack_assets()`.

- [ ] **Step 2: Add the typed action and result**

Add:

```rust
GetConversationInfo { channel: String }
```

to `SlackUserAction`, plus:

```rust
pub struct GetConversationInfoResult {
    pub ok: bool,
    pub conversation: Conversation,
}
```

- [ ] **Step 3: Implement the exact API call**

Add `api::get_conversation_info(channel: &str) -> Result<GetConversationInfoResult, String>`. It must call `conversations.info?channel={url_encode(channel)}`, map the returned `channel` object through the same `Conversation` shape used by list results, resolve a DM counterpart display name best-effort, and return Slack's existing structured errors unchanged.

- [ ] **Step 4: Wire dispatch and guidance**

Map `slack.get_conversation_info` to `get_conversation_info` in `action_from_context`, dispatch it in `execute_inner`, and update `send_message` guidance: known conversation ID uses exact lookup; unknown conversation uses list discovery. Keep the raw-ID user-facing prohibition.

- [ ] **Step 5: Run the Rust tests green**

Run the two Rust commands from Task 1. Expected: pass.

### Task 3: Update QA-10F and the distributable WASM

**Files:**
- Modify: `scripts/reborn_webui_v2_live_qa/run_live_qa.py`
- Modify: `scripts/reborn_webui_v2_live_qa/test_run_live_qa.py`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/wasm/slack_user_tool.wasm`

**Interfaces:**
- Consumes: the new exact lookup capability and existing real Slack readback classifier.
- Produces: a blocking QA-10F contract that requires exact lookup before accepting the verified side effect.

- [ ] **Step 1: Require exact lookup in QA-10F**

Set `expected_capability="slack.get_conversation_info"`. Preserve the prompt's exact conversation ID, encoded-mention requirement, author identity check, and post-run history readback.

- [ ] **Step 2: Rebuild the Slack WASM asset**

Run:

```bash
cargo component build --release --target wasm32-wasip2 --manifest-path crates/ironclaw_first_party_extensions/assets/slack/wasm-src/Cargo.toml
cp crates/ironclaw_first_party_extensions/assets/slack/wasm-src/target/wasm32-wasip2/release/slack_user_tool.wasm crates/ironclaw_first_party_extensions/assets/slack/wasm/slack_user_tool.wasm
```

Expected: the committed module changes and the host-runtime test executes the new dispatch path.

- [ ] **Step 3: Run the Python QA unit suite**

Run:

```bash
python3 -m unittest scripts.reborn_webui_v2_live_qa.test_run_live_qa
```

Expected: pass.

### Task 4: Verify, publish, and live-validate

**Files:**
- Check: `FEATURE_PARITY.md`
- Check: `CHANGELOG.md`
- Check: `docs/superpowers/specs/2026-07-12-q10-slack-canary-reliability-design.md`

**Interfaces:**
- Consumes: the completed source, schemas, test contracts, and WASM artifact.
- Produces: a review-ready PR with local and targeted live evidence.

- [ ] **Step 1: Run focused verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_first_party_extensions
cargo test -p ironclaw_reborn_composition --features slack-v2-host-beta slack_
cargo test -p ironclaw_host_runtime --test github_wasm_runtime_contract slack_
python3 -m unittest scripts.reborn_webui_v2_live_qa.test_run_live_qa
bash scripts/ci/check-reborn-qa-fixtures.sh
bash scripts/check-boundaries.sh
git diff --check
```

Expected: all commands pass. Update parity/changelog/spec only if their tracked status or documented contract is now inaccurate.

- [ ] **Step 2: Commit and push**

Commit with a `fix:` subject that names Slack exact conversation lookup, then push `codex/fix-slack-exact-conversation`.

- [ ] **Step 3: Open the PR and request review**

Open a focused PR against `main`, include the 2048-byte preview/same-name-DM root cause, explain that the fix is extension-owned, list verification, and request a full CodeRabbit review.

- [ ] **Step 4: Run targeted QA-10F validation**

Dispatch only `qa_10f_slack_mention_encoding` against the exact PR head using the live-canary workflow. Require a green result whose evidence includes a completed `slack.get_conversation_info` call and the correctly authored message in the requested DM. Do not run the full canary unless targeted evidence or CI exposes a broader interaction.

- [ ] **Step 5: Triage CI and review feedback**

Resolve actionable review comments, rerun only affected fast checks, and leave the PR non-draft, current with `main`, and free of failing or pending required checks.
