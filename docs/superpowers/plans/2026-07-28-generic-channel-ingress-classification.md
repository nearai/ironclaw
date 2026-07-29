# Generic Channel Ingress Classification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every generic channel ingress classify auth and approval replies plus slash commands into the existing typed product workflow without per-channel wiring.

**Architecture:** Vendor adapters continue to verify and normalize protocol payloads into `NormalizedInboundMessage`. A host-API classifier converts reserved interaction or slash-command text into `ChannelInboundClassification`, and `GenericChannelInboundSink` invokes it unconditionally before `ProductSurface` admission. Telegram additionally canonicalizes recognized `/command@botname` entities while retaining its existing group-trigger policy.

**Tech Stack:** Rust 2024 workspace, Tokio async tests, `ironclaw_host_api` product-adapter contracts, `ironclaw_extension_host` generic ingress, Slack and Telegram channel adapters, Cargo test/clippy.

## Global Constraints

- Preserve the existing product workflow, authorization, approval, auth-resume, command-admission, persistence, and idempotency semantics.
- Keep vendor webhook verification, protocol trigger selection, attachments, conversation references, delivery, activation, cleanup, and preference-target codecs adapter-specific.
- Keep the public legacy Slack and Telegram parser/render exports source-compatible.
- Do not change Telegram's configured group-trigger policy or add new product commands.
- Confident malformed reserved interaction or slash syntax must become `NoOp`; ambiguous natural language must remain an ordinary user message.
- Do not add dependencies, persistence migrations, credentials, secrets, external configuration, `.unwrap()`, or `.expect()` in production code.
- Use the smallest caller-path tests that prove typed side effects and run architecture validation after the shared-contract changes.

---

## File Structure

- `crates/ironclaw_host_api/src/product_adapter/inbound.rs`
  owns the new channel-neutral classification contract and command variant.
- `crates/ironclaw_host_api/src/product_adapter/mod.rs` and
  `crates/ironclaw_product/src/lib.rs` re-export the shared classifier through
  the existing public product-adapter surfaces.
- `crates/ironclaw_extension_host/src/extension_ingress.rs` invokes shared
  classification for every normalized channel message.
- `crates/ironclaw_extension_host/src/channel_host.rs` retains only genuinely
  vendor-specific channel extras.
- `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs` proves Slack
  auth and approval routing without manually injected behavior.
- `crates/ironclaw_telegram_v2_adapter/src/payload.rs` canonicalizes Telegram's
  bot-qualified command entities before generic classification.
- `crates/ironclaw_reborn_composition/src/input.rs`,
  `crates/ironclaw_reborn_composition/src/runtime.rs`,
  `crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs`, and their
  callers remove the obsolete optional classifier plumbing.
- `tests/integration/support/harness/profiles/extension.rs`,
  `tests/integration/extension_ingress.rs`, and
  `tests/integration/extension_delivery.rs` follow the simplified production
  constructors.

### Task 1: Define the Shared Classification Contract

**Files:**
- Modify: `crates/ironclaw_host_api/src/product_adapter/inbound.rs`
- Modify: `crates/ironclaw_host_api/src/product_adapter/mod.rs`
- Modify: `crates/ironclaw_product/src/lib.rs`

**Interfaces:**
- Consumes:
  `parse_interaction_resolution_text(&str, ProductTriggerReason)`,
  `strip_wrapping_inline_code(&str)`, and
  `parse_product_slash_command(&str, ProductTriggerReason)`.
- Produces:
  `classify_channel_inbound_text(&str, ProductTriggerReason) ->
  Option<ChannelInboundClassification>` and
  `ChannelInboundClassification::Command(InboundCommandPayload)`.

- [ ] **Step 1: Write failing host-API classification tests**

Add tests beside the existing `inbound.rs` tests:

```rust
#[test]
fn channel_inbound_classifier_routes_interactions_and_commands() {
    assert!(matches!(
        classify_channel_inbound_text(
            "`auth deny gate:auth-1`",
            ProductTriggerReason::DirectChat,
        ),
        Some(ChannelInboundClassification::AuthResolution(_))
    ));
    assert!(matches!(
        classify_channel_inbound_text(
            "approve gate:approval-1",
            ProductTriggerReason::BotMention,
        ),
        Some(ChannelInboundClassification::ApprovalResolution(_))
    ));
    match classify_channel_inbound_text(
        "/model set-provider openai --model gpt-5",
        ProductTriggerReason::DirectChat,
    ) {
        Some(ChannelInboundClassification::Command(command)) => {
            assert_eq!(command.command, "model");
            assert_eq!(command.arguments, "set-provider openai --model gpt-5");
            assert_eq!(command.trigger, ProductTriggerReason::DirectChat);
        }
        other => panic!("expected command classification, got {other:?}"),
    }
}

#[test]
fn channel_inbound_classifier_preserves_natural_language_and_fails_closed() {
    for text in ["hello", "approve this design", "auth deny"] {
        assert_eq!(
            classify_channel_inbound_text(text, ProductTriggerReason::DirectChat),
            None,
            "{text:?} must remain an ordinary user message"
        );
    }
    for text in ["auth deny gate:bad\0ref", "/bad\\command"] {
        assert_eq!(
            classify_channel_inbound_text(text, ProductTriggerReason::DirectChat),
            Some(ChannelInboundClassification::NoOp),
            "{text:?} is confident reserved syntax and must fail closed"
        );
    }
}

#[test]
fn channel_command_classification_converts_to_product_payload() {
    let command = InboundCommandPayload::new(
        "model",
        "openai/gpt-5",
        ProductTriggerReason::BotCommand,
    )
    .expect("valid command");
    assert!(matches!(
        ProductInboundPayload::from(ChannelInboundClassification::Command(command)),
        ProductInboundPayload::Command(_)
    ));
}
```

- [ ] **Step 2: Run the tests and verify the contract is absent**

Run:

```bash
cargo test -p ironclaw_host_api channel_inbound_classifier -- --nocapture
```

Expected: compilation fails because `classify_channel_inbound_text` and the
`Command` classification variant do not exist.

- [ ] **Step 3: Add the command variant and classifier**

Extend the enum and conversion:

```rust
pub enum ChannelInboundClassification {
    Command(InboundCommandPayload),
    ApprovalResolution(ApprovalResolutionPayload),
    ScopedApprovalResolution(ScopedApprovalResolutionPayload),
    AuthResolution(AuthResolutionPayload),
    NoOp,
}

impl From<ChannelInboundClassification> for ProductInboundPayload {
    fn from(classification: ChannelInboundClassification) -> Self {
        match classification {
            ChannelInboundClassification::Command(payload) => Self::Command(payload),
            ChannelInboundClassification::ApprovalResolution(payload) => {
                Self::ApprovalResolution(payload)
            }
            ChannelInboundClassification::ScopedApprovalResolution(payload) => {
                Self::ScopedApprovalResolution(payload)
            }
            ChannelInboundClassification::AuthResolution(payload) => Self::AuthResolution(payload),
            ChannelInboundClassification::NoOp => Self::NoOp,
        }
    }
}
```

Add the shared classifier in `inbound.rs`:

```rust
pub fn classify_channel_inbound_text(
    text: &str,
    trigger: ProductTriggerReason,
) -> Option<ChannelInboundClassification> {
    match crate::product_adapter::interaction_commands::parse_interaction_resolution_text(
        crate::product_adapter::interaction_commands::strip_wrapping_inline_code(text),
        trigger,
    ) {
        Ok(Some(ProductInboundPayload::ApprovalResolution(payload))) => {
            return Some(ChannelInboundClassification::ApprovalResolution(payload));
        }
        Ok(Some(ProductInboundPayload::ScopedApprovalResolution(payload))) => {
            return Some(ChannelInboundClassification::ScopedApprovalResolution(payload));
        }
        Ok(Some(ProductInboundPayload::AuthResolution(payload))) => {
            return Some(ChannelInboundClassification::AuthResolution(payload));
        }
        Ok(Some(ProductInboundPayload::NoOp)) | Err(_) => {
            return Some(ChannelInboundClassification::NoOp);
        }
        Ok(Some(_)) | Ok(None) => {}
    }

    match parse_product_slash_command(text, trigger) {
        Ok(Some(command)) => Some(ChannelInboundClassification::Command(command)),
        Ok(None) => None,
        Err(_) => Some(ChannelInboundClassification::NoOp),
    }
}
```

Re-export `classify_channel_inbound_text` from
`ironclaw_host_api::product_adapter` and `ironclaw_product`.

- [ ] **Step 4: Run the focused host-API tests**

Run:

```bash
cargo test -p ironclaw_host_api channel_inbound_classifier -- --nocapture
cargo test -p ironclaw_host_api channel_command_classification_converts_to_product_payload -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 5: Commit the shared contract**

```bash
git add crates/ironclaw_host_api/src/product_adapter/inbound.rs \
  crates/ironclaw_host_api/src/product_adapter/mod.rs \
  crates/ironclaw_product/src/lib.rs
git commit -m "feat(channels): classify shared inbound commands"
```

### Task 2: Enforce Classification in the Generic Channel Sink

**Files:**
- Modify: `crates/ironclaw_extension_host/src/extension_ingress.rs`
- Modify: `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs`

**Interfaces:**
- Consumes:
  `classify_channel_inbound_text(&message.text, message.trigger)` from Task 1.
- Produces:
  an invariant that every `ChannelInboundSurfaceRequest.classification` is
  derived by the generic sink before product admission.

- [ ] **Step 1: Remove the Slack-only classifier from the production-shaped E2E harness**

Change the Slack extras registration to:

```rust
assembly
    .register_extras(
        "slack",
        ChannelExtras {
            classifier: None,
            preference_target_codec: Some(Arc::new(SlackPreferenceTargetCodec)),
            subject_route_resolver: None,
            storage_roots: None,
        },
    )
    .await;
```

Delete `slack_gate_reply_classifier` and its `InboundPayloadClassifier` import.
Do not change the existing auth-denial and approval-resolution assertions.

- [ ] **Step 2: Run an existing caller-path regression and verify it fails**

Run:

```bash
cargo test -p ironclaw_extension_host \
  slack_thread_auth_deny_with_bot_mention_cancels_auth_gate_without_agent_turn \
  -- --nocapture
```

Expected: the test fails because the denial is admitted as a user message and
the recorded auth decision is absent.

- [ ] **Step 3: Record classifications in the sink unit-test surface**

Extend `CountingSurface`:

```rust
struct CountingSurface {
    submissions: AtomicUsize,
    classifications: std::sync::Mutex<Vec<Option<ChannelInboundClassification>>>,
}

impl CountingSurface {
    fn new() -> Self {
        Self {
            submissions: AtomicUsize::new(0),
            classifications: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn classifications(&self) -> Vec<Option<ChannelInboundClassification>> {
        self.classifications
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}
```

At the start of `admit_channel_inbound`, push
`request.classification.clone()` into the recording vector. Add this test:

```rust
#[tokio::test]
async fn generic_sink_classifies_gate_replies_commands_and_plain_text() {
    let cases = [
        (
            "auth deny gate:auth-1",
            Some(ChannelInboundClassification::AuthResolution(
                AuthResolutionPayload::new(
                    "gate:auth-1",
                    AuthResolutionResult::Denied,
                )
                .expect("valid auth payload")
                .with_source_trigger(ProductTriggerReason::DirectChat),
            )),
        ),
        (
            "/model openai/gpt-5",
            Some(ChannelInboundClassification::Command(
                InboundCommandPayload::new(
                    "model",
                    "openai/gpt-5",
                    ProductTriggerReason::DirectChat,
                )
                .expect("valid command"),
            )),
        ),
        ("hello", None),
    ];

    for (text, expected) in cases {
        let (sink, surface, _) =
            pairing_sink(ChannelPairingInterception::NotHandled);
        sink.admit(admission_for(text)).await.expect("admitted");
        assert_eq!(surface.classifications(), vec![expected]);
    }
}
```

- [ ] **Step 4: Make the generic sink classify unconditionally**

Import `classify_channel_inbound_text` and replace the optional-only assignment
with:

```rust
classification: classify_channel_inbound_text(&message.text, message.trigger).or_else(|| {
    self.config
        .classifier
        .as_ref()
        .and_then(|classify| classify(&message))
}),
```

The temporary fallback preserves source compatibility until Task 3 removes the
obsolete hook. Generic classification runs first and cannot be disabled.

- [ ] **Step 5: Run sink and Slack caller-path tests**

Run:

```bash
cargo test -p ironclaw_extension_host \
  generic_sink_classifies_gate_replies_commands_and_plain_text -- --nocapture
cargo test -p ironclaw_extension_host \
  slack_thread_auth_deny_with_bot_mention_cancels_auth_gate_without_agent_turn \
  -- --nocapture
cargo test -p ironclaw_extension_host \
  slack_dm_thread_auth_deny_cancels_base_dm_auth_gate_without_agent_turn \
  -- --nocapture
```

Expected: all three tests pass without a Slack classifier installed by the E2E
harness.

- [ ] **Step 6: Commit the generic sink invariant**

```bash
git add crates/ironclaw_extension_host/src/extension_ingress.rs \
  crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs
git commit -m "fix(channels): classify interactions in generic ingress"
```

### Task 3: Remove Optional Classifier Wiring

**Files:**
- Modify: `crates/ironclaw_extension_host/src/extension_ingress.rs`
- Modify: `crates/ironclaw_extension_host/src/channel_host.rs`
- Modify: `crates/ironclaw_extension_host/src/channel_pairing/tests.rs`
- Modify: `crates/ironclaw_reborn_composition/src/input.rs`
- Modify: `crates/ironclaw_reborn_composition/src/runtime.rs`
- Modify: `crates/ironclaw_reborn_composition/src/runtime/tests/core.rs`
- Modify: `crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs`
- Modify: `crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs`
- Modify: `tests/integration/support/harness/profiles/extension.rs`
- Modify: `tests/integration/extension_ingress.rs`
- Modify: `tests/integration/extension_delivery.rs`

**Interfaces:**
- Consumes: the unconditional generic classification invariant from Task 2.
- Produces:
  `ChannelInboundSinkConfig` without `classifier`,
  `ChannelExtras` without `classifier`, and
  `ChannelExtensionBinding` without `inbound_payload_classifier`.

- [ ] **Step 1: Remove the obsolete type and sink configuration field**

Delete:

```rust
pub type InboundPayloadClassifier =
    dyn Fn(&NormalizedInboundMessage) -> Option<ChannelInboundClassification> + Send + Sync;
```

Change the sink configuration to:

```rust
pub struct ChannelInboundSinkConfig {
    pub adapter_id: ProductAdapterId,
    pub evidence: VerifiedEvidenceMint,
    pub surface: Arc<dyn ChannelInboundProductSurface>,
    pub observer: Option<Arc<dyn PostAdmissionObserver>>,
}
```

Set the request field directly:

```rust
classification: classify_channel_inbound_text(&message.text, message.trigger),
```

Remove `classifier: ...` from every `ChannelInboundSinkConfig` constructor.

- [ ] **Step 2: Remove classifier state from the generic channel host**

Change the public and stored extras:

```rust
pub struct ChannelExtras {
    pub preference_target_codec: Option<Arc<dyn PreferenceTargetCodec>>,
    pub subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
    pub storage_roots: Option<ChannelWorkflowStorageRoots>,
}

#[derive(Clone, Default)]
struct StoredChannelExtras {
    preference_target_codec: Option<Arc<dyn PreferenceTargetCodec>>,
    subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
    storage_roots: Option<ChannelWorkflowStorageRoots>,
}
```

Update `register_extras`, sink construction, module documentation, and every
`ChannelExtras` constructor to match.

- [ ] **Step 3: Remove classifier state from composition and CLI bindings**

Change `ChannelExtensionBinding` to:

```rust
#[derive(Clone)]
pub struct ChannelExtensionBinding {
    pub extension_id: String,
    pub adapter: std::sync::Arc<dyn ironclaw_product::ChannelAdapter>,
    pub preference_target_codec:
        Option<std::sync::Arc<dyn ironclaw_product::PreferenceTargetCodec>>,
}
```

Remove `inbound_payload_classifier` from bundled Slack and Telegram bindings,
test bindings, and integration harness bindings. In composition, register only
the remaining `ChannelExtras` values:

```rust
ironclaw_extension_host::channel_host::ChannelExtras {
    preference_target_codec: binding.preference_target_codec.clone(),
    subject_route_resolver: None,
    storage_roots: None,
}
```

- [ ] **Step 4: Prove no optional classifier seam remains**

Run:

```bash
rg -n "InboundPayloadClassifier|inbound_payload_classifier|classifier:" \
  crates tests --glob '*.rs'
```

Expected: no matches for the removed channel-ingress hook. Matches unrelated to
channel ingress, if introduced concurrently upstream, must be inspected rather
than mechanically removed.

- [ ] **Step 5: Compile the affected constructor surfaces**

Run:

```bash
cargo test -p ironclaw_extension_host \
  generic_sink_classifies_gate_replies_commands_and_plain_text -- --nocapture
cargo test -p ironclaw_reborn_composition \
  persistent_grantee_resolver_maps_outbound_delivery_target_set_to_synthetic_provider \
  -- --nocapture
cargo test -p ironclaw \
  bundled_channel_bindings_carry_their_production_extras -- --nocapture
cargo test --test extension_ingress --no-run
cargo test --test extension_delivery --no-run
```

Expected: all focused tests pass and both integration targets compile.

- [ ] **Step 6: Commit the wiring removal**

```bash
git add crates/ironclaw_extension_host/src/extension_ingress.rs \
  crates/ironclaw_extension_host/src/channel_host.rs \
  crates/ironclaw_extension_host/src/channel_pairing/tests.rs \
  crates/ironclaw_reborn_composition/src/input.rs \
  crates/ironclaw_reborn_composition/src/runtime.rs \
  crates/ironclaw_reborn_composition/src/runtime/tests/core.rs \
  crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs \
  crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs \
  tests/integration/support/harness/profiles/extension.rs \
  tests/integration/extension_ingress.rs \
  tests/integration/extension_delivery.rs
git commit -m "refactor(channels): remove per-adapter classifier wiring"
```

### Task 4: Canonicalize Telegram Bot Commands

**Files:**
- Modify: `crates/ironclaw_telegram_v2_adapter/src/payload.rs`

**Interfaces:**
- Consumes:
  Telegram `bot_command` entities, `GroupTriggerPolicy`, and the existing
  `extract_first_bot_command`.
- Produces:
  normalized command text in the exact generic form
  `/<lowercase-command>[ <arguments>]`.

- [ ] **Step 1: Write failing Telegram normalization tests**

Add tests in the `payload.rs` test module:

```rust
#[test]
fn normalized_bot_qualified_command_uses_generic_command_text() {
    let payload = include_bytes!("../tests/fixtures/group_command.json");
    let event =
        normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("recognized group command must be forwarded");
    };
    assert_eq!(message.text, "/help");
    assert_eq!(message.trigger, ProductTriggerReason::BotCommand);
}

#[test]
fn normalized_bot_command_preserves_arguments() {
    let payload = br#"{
        "update_id": 501,
        "message": {
            "message_id": 71,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": -42, "type": "supergroup"},
            "text": "/help@ironclaw_bot verbose now",
            "entities": [{"type": "bot_command", "offset": 0, "length": 18}]
        }
    }"#;
    let event =
        normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("recognized group command must be forwarded");
    };
    assert_eq!(message.text, "/help verbose now");
}

#[test]
fn command_for_another_bot_remains_ignored_in_groups() {
    let payload = br#"{
        "update_id": 502,
        "message": {
            "message_id": 72,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": -42, "type": "supergroup"},
            "text": "/help@other_bot",
            "entities": [{"type": "bot_command", "offset": 0, "length": 15}]
        }
    }"#;
    assert!(matches!(
        normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes"),
        TelegramInboundEvent::Ignore
    ));
}
```

- [ ] **Step 2: Run the Telegram tests and verify canonicalization is absent**

Run:

```bash
cargo test -p ironclaw_telegram_v2_adapter normalized_bot -- --nocapture
```

Expected: the first two tests fail because normalized text still contains the
Telegram bot suffix.

- [ ] **Step 3: Canonicalize recognized entities during normalization**

Add:

```rust
fn normalize_forwarded_text(
    message: &TelegramMessage,
    policy: &GroupTriggerPolicy,
) -> String {
    if let Some((command, arguments)) = extract_first_bot_command(message, policy) {
        if arguments.is_empty() {
            return format!("/{command}");
        }
        return format!("/{command} {arguments}");
    }

    strip_leading_mention(
        message
            .text
            .clone()
            .or_else(|| message.caption.clone())
            .unwrap_or_default(),
        policy,
    )
}
```

Replace the current `strip_leading_mention(...)` expression in
`normalize_telegram_update` with:

```rust
let text = normalize_forwarded_text(&message, group_trigger_policy);
```

Do not change `classify_trigger`, entity slicing, bot-target checks, or
`GroupTriggerPolicy`.

- [ ] **Step 4: Run Telegram protocol and adapter tests**

Run:

```bash
cargo test -p ironclaw_telegram_v2_adapter normalized_bot -- --nocapture
cargo test -p ironclaw_telegram_v2_adapter command_for_another_bot -- --nocapture
cargo test -p ironclaw_telegram_extension private_chat_update_normalizes_to_one_message -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 5: Commit Telegram canonicalization**

```bash
git add crates/ironclaw_telegram_v2_adapter/src/payload.rs
git commit -m "fix(telegram): preserve commands through normalization"
```

### Task 5: Verify the End-to-End Contract and Prepare the PR

**Files:**
- Modify only if verification reveals a defect:
  files already listed in Tasks 1-4
- Inspect:
  `.github/pull_request_template.md`

**Interfaces:**
- Consumes: all implementation commits.
- Produces: formatted, lint-clean, architecture-valid changes and a complete
  draft PR test strategy.

- [ ] **Step 1: Format and inspect the exact diff**

Run:

```bash
cargo fmt --all
git diff --check origin/main...HEAD
git status --short
git diff --stat origin/main...HEAD
git diff -- crates/ironclaw_host_api/src/product_adapter/inbound.rs \
  crates/ironclaw_extension_host/src/extension_ingress.rs \
  crates/ironclaw_extension_host/src/channel_host.rs \
  crates/ironclaw_telegram_v2_adapter/src/payload.rs
```

Expected: formatting succeeds, no whitespace errors appear, and only scoped
design, plan, contract, host, adapter, composition, CLI, and test files changed.

- [ ] **Step 2: Run focused crate tests**

Run:

```bash
cargo test -p ironclaw_host_api
cargo test -p ironclaw_extension_host
cargo test -p ironclaw_telegram_v2_adapter
cargo test -p ironclaw_telegram_extension
cargo test -p ironclaw_reborn_composition
cargo test -p ironclaw bundled_channel_bindings_carry_their_production_extras
```

Expected: all commands pass.

- [ ] **Step 3: Run architecture and integration checks**

Run:

```bash
cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold
cargo test --test extension_ingress
cargo test --test extension_delivery
```

Expected: all commands pass.

- [ ] **Step 4: Run targeted clippy with warnings denied**

Run:

```bash
cargo clippy -p ironclaw_host_api --all-targets -- -D warnings
cargo clippy -p ironclaw_extension_host --all-targets -- -D warnings
cargo clippy -p ironclaw_telegram_v2_adapter --all-targets -- -D warnings
cargo clippy -p ironclaw_telegram_extension --all-targets -- -D warnings
cargo clippy -p ironclaw_reborn_composition --all-targets -- -D warnings
cargo clippy -p ironclaw --all-targets -- -D warnings
```

Expected: all commands pass with zero warnings.

- [ ] **Step 5: Run the repository safety audit**

Run:

```bash
rg -n "\\.unwrap\\(|\\.expect\\(" \
  crates/ironclaw_host_api/src/product_adapter/inbound.rs \
  crates/ironclaw_extension_host/src/extension_ingress.rs \
  crates/ironclaw_extension_host/src/channel_host.rs \
  crates/ironclaw_telegram_v2_adapter/src/payload.rs
rg -n "InboundPayloadClassifier|inbound_payload_classifier|classifier:" \
  crates tests --glob '*.rs'
git diff --name-only origin/main...HEAD
```

Expected: any `.unwrap()` or `.expect()` matches are test-only, the obsolete
classifier hook has no matches, and the changed-file list is scoped.

- [ ] **Step 6: Commit formatting or verification fixes**

If formatting changed tracked files, stage only those scoped files and commit:

```bash
git add crates/ironclaw_host_api/src/product_adapter/inbound.rs \
  crates/ironclaw_host_api/src/product_adapter/mod.rs \
  crates/ironclaw_product/src/lib.rs \
  crates/ironclaw_extension_host/src/extension_ingress.rs \
  crates/ironclaw_extension_host/src/channel_host.rs \
  crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs \
  crates/ironclaw_extension_host/src/channel_pairing/tests.rs \
  crates/ironclaw_telegram_v2_adapter/src/payload.rs \
  crates/ironclaw_reborn_composition/src/input.rs \
  crates/ironclaw_reborn_composition/src/runtime.rs \
  crates/ironclaw_reborn_composition/src/runtime/tests/core.rs \
  crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs \
  crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs \
  tests/integration/support/harness/profiles/extension.rs \
  tests/integration/extension_ingress.rs \
  tests/integration/extension_delivery.rs
git commit -m "test(channels): verify generic ingress routing"
```

If no tracked files changed, do not create an empty commit.

- [ ] **Step 7: Push and open a draft PR**

Read `.github/pull_request_template.md`, then push:

```bash
git push -u origin codex/generic-channel-ingress-classification
```

Open a draft PR whose body:

- explains the QA auth-denial regression and generic ingress root cause;
- describes command routing and Telegram canonicalization;
- inventories removed versus intentionally vendor-specific duplication;
- includes compatibility, rollback, and residual Telegram group-policy risk;
- completes every `Test Strategy` tier with evidence or
  `Not applicable: <reason>`.

Expected: GitHub returns a real draft PR URL.
