# Manifest-Declared Channel Command Allowlist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development or superpowers:executing-plans to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Make product-command exposure fail closed per channel manifest, with
bundled Slack and Telegram exposing only the existing exact `/status` command.

**Architecture:** `ironclaw_host_api::ChannelDescriptor` owns the neutral,
bounded `commands` declaration. `ironclaw_product` remains the only product
command registry and owns the concrete availability/direct-conversation
admission policy. `ironclaw_extension_host` validates a resolved manifest
against that registry and injects the policy into the generic channel graph.
Adapters only normalize ingress; no Slack- or Telegram-specific command
filtering or execution is added.

**Tech Stack:** Rust 2024 workspace, Serde/TOML, Tokio async tests,
`ironclaw_host_api`, `ironclaw_product`, `ironclaw_extension_host`, bundled
first-party extension manifests, Cargo test/clippy.

## Global Constraints

- Do not add product commands or create Slack-/Telegram-specific command
  handlers, allowlists, parsers, or dispatch branches.
- Missing `channel.commands` and `commands = []` must both expose no product
  commands.
- Declarations use exact slash-command tokens without `/`; enabling `status`
  must not implicitly enable its `progress` alias.
- The manifest allowlist is an exposure boundary, not an authorization grant.
  Existing direct-conversation policy remains, and unavailable sensitive
  commands must never reach their handlers.
- Pairing syntax such as Telegram `/start`, and interaction-resolution syntax
  such as `approve`, `deny`, and `auth deny`, are not product commands and must
  remain unaffected.
- The neutral descriptor validates syntax, bounds, and duplicates. The product
  registry validates whether a well-formed token names a real product command
  or alias.
- Rejection feedback must reveal only commands enabled for the current channel.
  An empty allowlist must not leak the global command inventory.
- Preserve existing defaults outside command exposure. Add no persistence
  migrations, dependencies, credentials, secrets, `.unwrap()`, or `.expect()`
  in production code.
- Use red-green-refactor for every behavior change: add the caller-level test,
  run it and observe the expected failure, implement the smallest change, then
  rerun it.
- Re-run architecture validation because a neutral manifest contract and
  cross-crate wiring change together.

---

## File Structure

- `crates/ironclaw_host_api/src/channel.rs`
  owns the serialized `ChannelDescriptor.commands` field and neutral
  declaration validation.
- `crates/ironclaw_product/src/commands.rs`
  owns exact-name lookup against the canonical product-command registry and
  enabled-inventory rendering.
- `crates/ironclaw_product/src/command_dispatch.rs`
  carries the exact inbound command token in the authority-bearing admission
  context.
- `crates/ironclaw_product/src/command_admission.rs`
  owns the concrete direct-conversation plus manifest-availability policy.
- `crates/ironclaw_product/src/lib.rs`
  re-exports only the registry/policy interfaces needed by the generic host.
- `crates/ironclaw_product/src/run_delivery/observer.rs`
  renders channel-scoped invalid-request feedback configured from the same
  validated manifest instead of the global inventory.
- `crates/ironclaw_product/tests/product_command_surface_contract.rs`
  proves exact-token admission context and fail-closed handler behavior.
- `crates/ironclaw_extension_host/src/channel_host.rs`
  validates the resolved manifest and injects the centralized policy.
- `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs`
  proves `/status` execution and disabled-command rejection through the real
  bundled Slack generic graph.
- `crates/ironclaw_extension_host/src/available_extensions.rs`
  proves the shipping Slack and Telegram package declarations.
- `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml` and
  `crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml`
  opt into exactly `status`.
- `docs/reborn/contracts/extensions.md`
  documents the schema, fail-closed compatibility behavior, and authorization
  boundary.

### Task 1: Add the Fail-Closed Neutral Manifest Contract

**Files:**
- Modify: `crates/ironclaw_host_api/src/channel.rs`

**Interfaces:**
- Add `ChannelDescriptor.commands: Vec<String>` with Serde defaulting to empty
  and omitting empty lists when serialized.
- Extend `ChannelDescriptor::validate()` with bounded token validation.
- Extend `ChannelDescriptorError` with one typed command-declaration error
  family.

- [ ] **Step 1: Write failing descriptor tests**

Add focused tests beside the existing `ChannelDescriptor` tests:

```rust
#[test]
fn channel_commands_are_exact_and_fail_closed_by_default() {
    let missing: ChannelDescriptor =
        toml::from_str(documented_channel_toml()).expect("documented channel parses");
    assert!(missing.commands.is_empty());

    let source = documented_channel_toml().replace(
        "conversation_model = \"continuous\"\n",
        "conversation_model = \"continuous\"\ncommands = [\"status\"]\n",
    );
    let declared: ChannelDescriptor = toml::from_str(&source).expect("commands parse");
    assert_eq!(declared.commands, ["status"]);
    assert!(declared.validate().is_ok());
}
```

Add table-driven validation cases for:

- an explicit empty list;
- duplicate `status`;
- leading `/`;
- empty and whitespace-containing names;
- uppercase names;
- a token longer than the command-name byte limit;
- more declarations than the command-count limit.

Round-trip the `["status"]` descriptor through JSON to prove the resolved
descriptor retains the declaration.

- [ ] **Step 2: Run the focused tests and observe the missing field**

Run:

```bash
cargo test -p ironclaw_host_api channel_commands_ -- --nocapture
```

Expected: compilation fails because `ChannelDescriptor.commands` does not
exist.

- [ ] **Step 3: Implement the bounded descriptor field**

Add:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub commands: Vec<String>,
```

Use explicit neutral limits for the maximum declaration count and per-token
UTF-8 byte length. Validate each token as non-empty lower-case ASCII command
grammar (`a-z`, `0-9`, `_`, and `-`), reject `/`, whitespace, control
characters, and duplicates, and return a typed `ChannelDescriptorError`.

The validator must not know which product commands exist. Registry membership
belongs to `ironclaw_product`.

- [ ] **Step 4: Run the focused and full host-API tests**

Run:

```bash
cargo test -p ironclaw_host_api channel_commands_ -- --nocapture
cargo test -p ironclaw_host_api channel -- --nocapture
```

Expected: all descriptor tests pass, including missing/empty fail-closed
behavior and strict malformed declarations.

- [ ] **Step 5: Commit the neutral contract**

```bash
git add crates/ironclaw_host_api/src/channel.rs
git commit -m "feat(channels): declare bounded manifest commands"
```

### Task 2: Make Product Admission Exact, Centralized, and Fail Closed

**Files:**
- Modify: `crates/ironclaw_product/src/commands.rs`
- Modify: `crates/ironclaw_product/src/command_dispatch.rs`
- Modify: `crates/ironclaw_product/src/command_admission.rs`
- Modify: `crates/ironclaw_product/src/lib.rs`
- Modify: `crates/ironclaw_product/tests/product_command_surface_contract.rs`

**Interfaces:**
- Add the original normalized token to `ProductCommandContext`, separate from
  `ProductCommand::name()` so aliases are not canonicalized before policy.
- Add product-owned exact registry validation for canonical names and aliases.
- Replace the unit admission policy with a constructed policy that stores the
  resolved exact allowlist.
- Add product-owned help rendering for an enabled set only.

- [ ] **Step 1: Write failing exact-token context and registry tests**

In `product_command_surface_contract.rs`, extend
`command_admission_receives_authority_context_and_action_metadata` to assert
that an inbound `progress` payload gives admission the exact token
`"progress"`, even though it parses to `ProductCommand::Status`.

In `product_commands_contract.rs`, add focused tests proving:

- `status` and `progress` are both individually recognized registry tokens;
- `notacommand` returns a typed unknown-token error;
- enabled help for `["status"]` is exactly `Available commands:\n/status`;
- enabled help for an empty list says commands are unavailable and contains no
  global command names.

- [ ] **Step 2: Run the focused tests and observe the absent APIs**

Run:

```bash
cargo test -p ironclaw_product --test product_command_surface_contract \
  command_admission_receives_authority_context_and_action_metadata -- --nocapture
cargo test -p ironclaw_product --test product_commands_contract \
  declared_command -- --nocapture
```

Expected: compilation fails because the exact requested token and scoped
registry/help APIs do not exist.

- [ ] **Step 3: Carry the exact inbound token and add registry helpers**

Populate a new `requested_command: String` (or equivalently strong product-owned
type) in `ProductCommandContext::from_envelope()` directly from
`InboundCommandPayload.command`.

In `commands.rs`, add:

- exact lookup across each `ProductCommandDescriptor.name` and its declared
  aliases;
- a typed error that retains the unknown token for startup diagnostics;
- enabled-inventory rendering that sorts/deduplicates only its supplied tokens
  and emits an explicit unavailable message for an empty list.

Do not change `ProductCommand::name()` or the command handlers.

- [ ] **Step 4: Write failing concrete-admission tests**

Add contract tests that construct the production admission policy and route
commands through `DefaultProductSurface`:

- empty allowlist rejects `/status` as durable `InvalidRequest`;
- `["status"]` admits exact `/status`;
- `["status"]` rejects `/progress`,
  `/model set-provider openai --model gpt-5`,
  `/extension_configure slack`, and `/skill_remove demo`;
- rejected commands do not invoke the recording command surface and do not
  submit a user turn;
- the disabled-command reason lists `/status` and excludes `/model`,
  `/extension_configure`, and skill commands;
- a shared-conversation `/status` remains `PolicyDenied` by the separate
  direct-conversation rule.

- [ ] **Step 5: Run the admission tests and observe current allow-all behavior**

Run:

```bash
cargo test -p ironclaw_product --test product_command_surface_contract \
  manifest_command_admission_ -- --nocapture
```

Expected: the new tests fail because the current unit
`DirectConversationCommandAdmission` admits every direct-conversation command
and has no allowlist constructor.

- [ ] **Step 6: Implement the centralized admission policy**

Make the concrete policy stateful and construct it from validated exact command
tokens. Its order is:

1. reject non-direct conversations with the existing `PolicyDenied` result;
2. compare `context.requested_command` exactly against the stored allowlist;
3. reject absent tokens permanently as `InvalidRequest`, using enabled-only
   help text;
4. otherwise return `Allowed`.

The constructor must validate every supplied token through the central product
registry, so no alternate caller can create an allowlist containing an unknown
command. A `Default` implementation, if retained for compatibility, must be
empty/fail-closed rather than allow-all.

- [ ] **Step 7: Run all product command contracts**

Run:

```bash
cargo test -p ironclaw_product --test product_command_surface_contract -- --nocapture
cargo test -p ironclaw_product --test product_commands_contract -- --nocapture
```

Expected: exact-token, sensitive-command denial, direct-conversation, lease,
and existing command dispatch contracts all pass.

- [ ] **Step 8: Commit the product policy**

```bash
git add crates/ironclaw_product/src/commands.rs \
  crates/ironclaw_product/src/command_dispatch.rs \
  crates/ironclaw_product/src/command_admission.rs \
  crates/ironclaw_product/src/lib.rs \
  crates/ironclaw_product/tests/product_command_surface_contract.rs \
  crates/ironclaw_product/tests/product_commands_contract.rs
git commit -m "feat(product): enforce channel command allowlists"
```

### Task 3: Preserve Channel-Scoped Rejection Feedback

**Files:**
- Modify: `crates/ironclaw_product/src/run_delivery/observer.rs`

**Interfaces:**
- Consume enabled-command help configured from the same validated manifest as
  command admission.
- Keep fixed host text for `PolicyDenied` shared-conversation feedback.

- [ ] **Step 1: Write a failing observer test**

Add an observer test that configures only `status` and posts a rejected command
ack with:

```rust
ProductRejection::permanent(
    ProductRejectionKind::InvalidRequest,
    "Available commands:\n/status",
)
```

Assert the delivered command feedback is exactly
`Available commands:\n/status` and does not contain `/model`,
`/extension_configure`, or another global command.

- [ ] **Step 2: Run the test and observe the global inventory leak**

Run:

```bash
cargo test -p ironclaw_product command_feedback_uses_scoped_invalid_request_reason \
  -- --nocapture
```

Expected: the current observer calls `command_help_text()` and exposes the
global inventory.

- [ ] **Step 3: Configure enabled-only invalid-request feedback**

Keep `ProductRejection.reason` opaque: `RedactedString` intentionally exposes no
inner value. Add an observer setting/builder for enabled command tokens,
default it to the empty fail-closed inventory, and change only the
`InvalidRequest` feedback arm to render that precomputed enabled-only help.
Configure the observer from the same resolved channel descriptor used to build
admission. Preserve the fixed shared-conversation message for `PolicyDenied`
and the existing behavior for all other rejection families.

Remove the now-unused global `command_help_text` import from the observer; do
not remove the global helper if other callers/tests still use it.

- [ ] **Step 4: Run observer and product regression tests**

Run:

```bash
cargo test -p ironclaw_product command_feedback_uses_scoped_invalid_request_reason \
  -- --nocapture
cargo test -p ironclaw_product --lib run_delivery::observer -- --nocapture
```

Expected: scoped feedback is delivered and existing direct-conversation,
binding, and terminal-rejection behavior remains intact.

- [ ] **Step 5: Commit the feedback boundary**

```bash
git add crates/ironclaw_product/src/run_delivery/observer.rs
git commit -m "fix(product): scope channel command feedback"
```

### Task 4: Wire the Resolved Manifest into Every Generic Channel

**Files:**
- Modify: `crates/ironclaw_extension_host/src/channel_host.rs`
- Modify: `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs`

**Interfaces:**
- Consume `source.resolved().channel.commands`.
- Construct the product-owned admission policy during generic graph assembly.
- Fail channel assembly for syntactically valid but unknown declarations.

- [ ] **Step 1: Convert the Slack generic-host E2E to the intended policy**

Update the command fixtures and caller-path expectations:

- `DM_COMMAND` sends `/status`;
- the successful test expects `product.status.command` and the rendered status
  result;
- the shared-conversation fixture sends `/status` and still receives the
  direct-conversation denial;
- unknown-command feedback contains only `/status`;
- add distinct direct-message fixtures for
  `/model set-provider openai --model gpt-5`,
  `/extension_configure slack`, and `/skill_remove demo`.

Add a table-driven E2E proving every disabled sensitive command:

- returns a durable command rejection;
- posts enabled-only `/status` help;
- invokes no command operation;
- submits no user turn.

- [ ] **Step 2: Run the E2E and observe current unrestricted behavior**

Run:

```bash
cargo test -p ironclaw_extension_host \
  channel_host::e2e_tests::dm_slash_command_executes_and_delivers_rendered_result \
  -- --nocapture
cargo test -p ironclaw_extension_host \
  channel_host::e2e_tests::disabled_dm_slash_commands_are_rejected_without_execution \
  -- --nocapture
```

Expected: `/status` may execute, but disabled commands still reach the command
surface or receive global-inventory feedback because the manifest is not wired
to admission.

- [ ] **Step 3: Write a failing unknown-declaration assembly test**

Build a test `HostedChannelSource` whose otherwise valid channel descriptor has
`commands = ["syntactically_valid_but_unknown"]`. Assert
`build_generic_graph()` returns a deterministic startup/configuration error
naming the extension and unknown command instead of silently starting.

Add a second source with a missing/empty declaration and assert its generic
graph starts but rejects `/status`.

- [ ] **Step 4: Run the assembly tests and observe missing validation**

Run:

```bash
cargo test -p ironclaw_extension_host \
  channel_host::tests::unknown_manifest_command_fails_generic_graph_assembly \
  -- --nocapture
cargo test -p ironclaw_extension_host \
  channel_host::tests::empty_manifest_commands_are_fail_closed \
  -- --nocapture
```

Expected: the unknown declaration is currently ignored and the unit direct
conversation policy still admits commands.

- [ ] **Step 5: Inject the product-owned policy**

In `build_generic_graph()`:

1. read the resolved `ChannelDescriptor.commands`;
2. construct the product-owned exact allowlist policy;
3. map unknown-token errors to a deterministic channel assembly error with
   extension context;
4. pass that policy to
   `with_product_command_admission_service()`.

Do not branch on extension id, transport, Slack, or Telegram. Do not duplicate
the product registry in `ironclaw_extension_host`.

- [ ] **Step 6: Run the focused generic-host command suite**

Run:

```bash
cargo test -p ironclaw_extension_host \
  channel_host::e2e_tests::dm_slash_command_executes_and_delivers_rendered_result \
  -- --nocapture
cargo test -p ironclaw_extension_host \
  channel_host::e2e_tests::unknown_dm_slash_command_returns_enabled_inventory_help_without_a_turn \
  -- --nocapture
cargo test -p ironclaw_extension_host \
  channel_host::e2e_tests::disabled_dm_slash_commands_are_rejected_without_execution \
  -- --nocapture
cargo test -p ironclaw_extension_host \
  channel_host::e2e_tests::shared_channel_slash_command_is_denied_with_notice \
  -- --nocapture
```

Expected: only `/status` invokes a command operation; disabled/unknown direct
commands expose only `/status`; shared-channel `/status` remains denied; no
command path submits a turn.

- [ ] **Step 7: Commit generic host enforcement**

```bash
git add crates/ironclaw_extension_host/src/channel_host.rs \
  crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs
git commit -m "feat(channels): enforce resolved command exposure"
```

### Task 5: Opt Slack and Telegram into Status Only

**Files:**
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml`
- Modify: `crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml`
- Modify: `crates/ironclaw_extension_host/src/available_extensions.rs`
- Modify if the existing production-binding assertion requires it:
  `crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs`

**Interfaces:**
- Shipping Slack and Telegram resolved manifests each expose exactly
  `["status"]`.
- No other bundled or fixture channel gains implicit command exposure.

- [ ] **Step 1: Write failing bundled-manifest assertions**

Extend `bundled_slack_package_declares_product_adapter_channel_surface` to
assert `channel.commands == ["status"]`.

Add the symmetric Telegram assertion through the same bundled inventory path.
Also assert a fixture manifest with no declaration resolves to an empty
allowlist, proving there is no global fallback.

- [ ] **Step 2: Run the assertions and observe missing declarations**

Run:

```bash
cargo test -p ironclaw_extension_host \
  bundled_slack_package_declares_product_adapter_channel_surface -- --nocapture
cargo test -p ironclaw_extension_host \
  bundled_telegram_package_declares_status_only -- --nocapture
```

Expected: Slack and Telegram currently resolve with empty command lists.

- [ ] **Step 3: Update only the two shipping manifests**

Add this inside each existing `[channel]` table:

```toml
commands = ["status"]
```

Do not add `model`, lifecycle commands, aliases, pairing commands, approval
commands, or auth commands.

- [ ] **Step 4: Update Telegram's production command-routing assertion**

The CLI test
`bundled_telegram_binding_routes_targeted_commands_through_generic_sink`
currently proves Telegram protocol normalization with `/model` fixtures. Keep
that test focused on normalization if it bypasses manifest admission, but add
or update a production-assembly assertion so its shipping resolved descriptor
declares only `status`. Do not imply that raw sink normalization alone grants
command availability.

- [ ] **Step 5: Run manifest, package, and Telegram normalization tests**

Run:

```bash
cargo test -p ironclaw_extension_host bundled_slack_package -- --nocapture
cargo test -p ironclaw_extension_host bundled_telegram_package -- --nocapture
cargo test -p ironclaw_reborn_cli \
  bundled_telegram_binding_routes_targeted_commands_through_generic_sink \
  -- --nocapture
cargo test -p ironclaw_telegram_v2_adapter command -- --nocapture
```

Expected: both bundled manifests expose exactly `/status`, undeclared fixtures
remain empty, and Telegram bot-addressing normalization still works without
owning product availability.

- [ ] **Step 6: Commit shipping manifest policy**

```bash
git add crates/ironclaw_first_party_extensions/assets/slack/manifest.toml \
  crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml \
  crates/ironclaw_extension_host/src/available_extensions.rs \
  crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs
git commit -m "feat(channels): expose status in bundled channels"
```

Only stage `native_extensions.rs` if its test actually changes.

### Task 6: Document the Contract and Verify the Security Boundary

**Files:**
- Modify: `docs/reborn/contracts/extensions.md`
- Verify: all files changed by Tasks 1-5
- Update on GitHub after verification: PR #6816 body

- [ ] **Step 1: Document the manifest field**

Add a `[channel]` example with `commands = ["status"]` and state:

- missing and empty mean no product commands;
- tokens are exact and aliases require independent declaration;
- malformed/duplicate declarations fail descriptor validation;
- unknown well-formed names fail generic graph assembly;
- availability does not grant administrator/operator authority;
- pairing and gate-resolution syntax are separate protocols;
- an older binary with strict unknown-field rejection may require restoring a
  pre-upgrade resolved-manifest snapshot when rolling back.

- [ ] **Step 2: Format and run focused crate suites**

Run:

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_host_api
cargo test -p ironclaw_product
cargo test -p ironclaw_extension_host
cargo test -p ironclaw_telegram_v2_adapter
cargo test -p ironclaw_telegram_extension
cargo test -p ironclaw_reborn_cli native_extensions -- --nocapture
```

If a broad suite is blocked by an unrelated baseline failure, record the exact
command and first meaningful error, then ensure every changed contract still
has a passing focused test.

- [ ] **Step 3: Run architecture and warning checks**

Run:

```bash
cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold
cargo clippy -p ironclaw_host_api -p ironclaw_product \
  -p ironclaw_extension_host -p ironclaw_reborn_cli \
  --all-targets --all-features -- -D warnings
git diff --check
```

Expected: no dependency-boundary failure, clippy warning, or whitespace error.

- [ ] **Step 4: Perform the required production-code safety audit**

Run targeted searches over changed production files for:

```bash
rg -n '\.unwrap\(\)|\.expect\(' \
  crates/ironclaw_host_api/src/channel.rs \
  crates/ironclaw_product/src/commands.rs \
  crates/ironclaw_product/src/command_dispatch.rs \
  crates/ironclaw_product/src/command_admission.rs \
  crates/ironclaw_product/src/run_delivery/observer.rs \
  crates/ironclaw_extension_host/src/channel_host.rs
```

Review every hit and ensure none was added to production. Also verify:

- exact inbound token is used for availability, not canonical command name;
- empty/missing manifests are fail closed;
- direct-conversation admission remains distinct;
- `/model`, `/extension_configure`, and skill mutation handlers are not invoked
  from bundled Slack/Telegram;
- enabled-only feedback cannot expose the global inventory;
- `approve`, `deny`, `auth deny`, and Telegram pairing remain outside this
  allowlist;
- no adapter-local command execution/filtering was introduced.

- [ ] **Step 5: Review the final diff and test strategy**

Run:

```bash
git status --short
git diff --stat origin/main...HEAD
git diff --check origin/main...HEAD
git diff --name-only origin/main...HEAD
```

Complete every `Test Strategy` field in the PR body. Mark this as a security /
permissions and cross-component change. Include compatibility, rollback, and
the remaining fact that manifest availability does not repair independent
action-level admin/owner authorization for commands not exposed here.

- [ ] **Step 6: Commit documentation**

```bash
git add docs/reborn/contracts/extensions.md
git commit -m "docs(extensions): define channel command exposure"
```

- [ ] **Step 7: Push and update PR #6816**

```bash
git push origin codex/generic-channel-ingress-classification
gh pr edit 6816 --body-file /tmp/pr-6816-body.md
gh pr view 6816 --json url,isDraft,headRefName,headRefOid,statusCheckRollup
```

Expected: the PR URL remains
`https://github.com/nearai/ironclaw/pull/6816`, the remote head matches local
`HEAD`, and the body contains complete validation evidence without claiming
unrun live-provider coverage.
