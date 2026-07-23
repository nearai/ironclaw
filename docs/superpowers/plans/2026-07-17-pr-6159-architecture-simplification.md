# PR 6159 Architecture Simplification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve PR #6159's Telegram behavior while moving delivery and Telegram runtime policy to their proper owners, deleting test-only abstraction layers and mirror DTOs, and pinning the resulting architecture with executable ratchets.

**Architecture:** `ironclaw_channel_delivery` becomes the product-neutral delivery engine and `ironclaw_product` owns reusable auth/approval projection and extension account-setup contracts. `ironclaw_telegram_extension` owns one concrete filesystem-backed host state, its concrete Bot API/egress clients, revision-aware runtime construction, and focused setup/pairing/ingress/delivery modules; composition supplies already-built neutral services and mounts/registers the returned facades only.

**Tech Stack:** Rust 2024, Tokio, Axum, `async-trait`, `ScopedFilesystem<dyn RootFilesystem>`, host-mediated HTTP egress, Cargo workspace tests, Clippy, repository architecture tests.

## Global Constraints

- Preserve every route, request/response JSON field, manifest field, secret handle, persisted record/path, identity key, webhook behavior, pairing semantic, delivery-status mapping, and feature flag from PR #6159.
- Keep token material inside `SecretStore` and `HostRuntimeHttpEgressPort`; never construct a URL containing raw Telegram credentials.
- Production code may not use `.unwrap()` or `.expect()` and workspace Clippy must finish with zero warnings.
- Composition may assemble and register services but may not own generic delivery algorithms, Telegram revision caching, Telegram dispatcher decoration, or Telegram-triggered delivery behavior.
- Retain dynamic dispatch only for cross-owner ports or true production strategies: channel delivery protocol, Reborn identity lookup, outbound target provider, revision workflow builder, setup activation, webhook dispatcher/decorators, and product-auth ports.
- All Telegram persistence tests use `FilesystemTelegramHostState` over `InMemoryBackend`; failure tests inject at filesystem, secret-store, or mediated HTTP seams.
- Touched production Telegram source files must contain fewer than 1,000 physical lines.
- Do not rebase the PR, alter schemas/wire contracts, perform a full Unified Extension Runtime migration, or clean unrelated code.

## Material Review-Closure Checklist

This checklist is the acceptance record for the full CodeRabbit pass. An item
is complete only when the production path and a meaningful caller/seam test are
both present where the finding requires behavioral proof.

- [x] Terminal delivery-honesty absence proof drains the runtime before its final assertion.
- [x] Setup persistence uses bounded CAS with conflict and rollback-race coverage.
- [x] Recorded Bot API requests fail loudly on malformed JSON.
- [x] Account-status failures retain their internal cause while exposing sanitized copy.
- [x] Auth-challenge tests assert every forwarded authority argument.
- [x] Bot API mediation uses the canonical Telegram extension id.
- [x] Unconfigured/broken dynamic trigger hooks persist terminal outcomes.
- [x] Ingress builds identity, secret, adapter, and workflow from one atomic setup snapshot.
- [x] Pairing codes are bound to the authenticated bot installation.
- [x] Pairing continuation work is durable; ingress awaits generic fan-out
      acceptance and transient failures rely on provider redelivery.
- [x] Unpair cleanup keeps durable metadata until actor and DM cleanup finish.
- [x] Product connection status never exposes backend diagnostics.
- [x] Pairing uses one validated `PairingCode` contract type.
- [x] Clear and activation rollback use durable, restart-safe lifecycle intents.
- [x] Binding mutation uses bounded CAS and compensates partial index failures.
- [x] Per-user binding indexes preserve concurrent provider identities.
- [ ] Pairing-code rotation and invalidation use bounded CAS rather than a
      process-local lock, with stale code records non-authoritative.
- [x] Trigger delivery tasks are bounded, lifecycle-owned, and drained on shutdown.
- [x] Trigger outcome-store failures propagate to the managed task owner.
- [x] Per-trigger target authority rejects same-scope target substitution.
- [x] Approval-store outages remain actionable transient failures, not empty prompts.
- [x] Architecture gates recognize visibility-qualified in-memory store declarations.
- [x] Each channel owns a distinct notification projection namespace.
- [x] Busy-hint dedup commits only after successful egress.
- [x] Internal background-delivery diagnostics remain at debug level.
- [x] Delivery requires provider-issued posted-message evidence.
- [x] Webhook override precedence is documented and tested alongside the default path.
- [x] Invalid multi-filter Cargo commands are split into runnable commands.
- [x] CI documentation names the actual embedded frontend asset path.
- [x] WebUI predecessor documentation names `ironclaw_webui_v2` correctly.
- [x] Disconnect invalidates stale pairing polls before they can reconnect the UI.
- [x] Setup UI adopts newer pristine server revisions without overwriting dirty edits.
- [ ] Every review fix passes the necessity audit: a reachable production failure,
      no existing simpler mechanism, no mirror DTO/local store, and no speculative
      abstraction or persisted field.
- [ ] Full local validation stack is green on the final diff.
- [ ] GitHub CI is green on the pushed head.
- [ ] Every review thread has an evidence-backed reply and is resolved.

## Locked File Structure

```text
crates/ironclaw_channel_delivery/
  Cargo.toml
  AGENTS.md
  src/lib.rs
  src/services.rs
  src/observer.rs
  src/actionable.rs
  src/routing.rs
  src/hooks.rs
  src/triggered.rs
  src/tests.rs

crates/ironclaw_product/src/
  auth_prompt.rs
  approval_prompt.rs
  extension_account_setup.rs

crates/ironclaw_telegram_extension/src/
  setup/{mod.rs,service.rs,status.rs,compensation.rs}
  pairing/{mod.rs,code.rs,service.rs,status.rs}
  ingress/{mod.rs,resolver.rs,route.rs,dispatch.rs}
  delivery/{mod.rs,protocol.rs,targets.rs,triggered.rs}
  state/{mod.rs,records.rs,setup.rs,pairing.rs,bindings.rs,dm_targets.rs}
  host/{mod.rs,builder.rs,revision.rs}
  bot_api.rs
  egress.rs
  channel_routes.rs
  test_support.rs
```

The existing `telegram_*` public module names are re-exported only where downstream crates still consume them. No compatibility module contains behavior.

---

### Task 1: Add all eight architecture ratchets in their red state

**Files:**
- Modify: `crates/ironclaw_architecture/tests/telegram_extension_gates.rs`
- Modify: `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`
- Modify: `crates/ironclaw_architecture/tests/reborn_composition_boundaries.rs`

**Interfaces:**
- Consumes: repository paths and workspace Cargo metadata.
- Produces: exact-path/symbol ratchets named below; later tasks turn each failure green without weakening its assertion.

- [x] **Step 1: Add exact symbol/path scanners**

Add constants and tests with these exact target sets:

```rust
const DELETED_TELEGRAM_SYMBOLS: &[&str] = &[
    "TelegramInstallationSetupStore",
    "TelegramPairingStore",
    "TelegramUserBindingStore",
    "TelegramDmTargetStore",
    "TelegramEgressCredentialProvider",
    "TelegramInstallationResolver",
    "TelegramBotApi",
    "TelegramPairingStatusResponse",
    "ResolvedTelegramIngress",
];

const TELEGRAM_PRODUCTION_LINE_BUDGET: usize = 999;

#[test]
fn generic_channel_delivery_is_not_owned_by_composition() { /* assert old path absent */ }

#[test]
fn generic_extension_lifecycle_has_no_telegram_knowledge() { /* scan production prefix before cfg(test) */ }

#[test]
fn deleted_telegram_abstractions_and_dtos_stay_deleted() { /* scan Telegram production sources */ }

#[test]
fn telegram_tests_use_the_real_filesystem_state() { /* reject struct names matching InMemory*Store */ }

#[test]
fn telegram_production_files_meet_the_line_budget() { /* enumerate touched .rs files */ }

#[test]
fn telegram_composition_is_assembly_only() { /* enforce symbol denylist and 450-line production budget */ }
```

Add `ironclaw_channel_delivery` to `SUBSTRATE_CRATES`, and add a `BoundaryRule` forbidding dependencies on `ironclaw_reborn_composition`, `ironclaw_reborn_cli`, `ironclaw_webui_v2`, `ironclaw_slack_v2_adapter`, and `ironclaw_telegram_extension`.

- [x] **Step 2: Run each ratchet and record the intended baseline failure**

Run:

```bash
cargo test -p ironclaw_architecture --test telegram_extension_gates
cargo test -p ironclaw_architecture --test reborn_dependency_boundaries
cargo test -p ironclaw_architecture --test reborn_composition_boundaries
```

Observed: the new Telegram gate tests fail on the old composition delivery path, lifecycle Telegram symbols, deleted-symbol set, in-memory store fakes, oversized files, and composition behavior. The dependency tests pass because their rules activate only for workspace crates; adding `ironclaw_channel_delivery` in Task 3 activates its already-declared rule.

- [x] **Step 3: Commit the red ratchets**

```bash
git add crates/ironclaw_architecture/tests
git commit -m "test(reborn): pin PR 6159 simplification boundaries"
```

---

### Task 2: Move reusable auth and approval projection contracts to product workflow

**Files:**
- Create: `crates/ironclaw_product/src/auth_prompt.rs`
- Create: `crates/ironclaw_product/src/approval_prompt.rs`
- Modify: `crates/ironclaw_product/src/lib.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/api/auth_prompt.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/api/mod.rs`
- Modify: `crates/ironclaw_reborn_composition/src/projection/turn_events.rs`
- Modify: `crates/ironclaw_reborn_composition/src/lib.rs`
- Modify: composition projection and product-auth tests importing these types.

**Interfaces:**
- Consumes: `AuthPromptView`, `ApprovalPromptContextView`, `ApprovalRequestStore`, `TurnScope`, and existing product-auth implementations.
- Produces:

```rust
pub struct AuthChallengeView { /* existing redacted fields unchanged */ }

#[async_trait]
pub trait AuthChallengeProvider: Send + Sync { /* existing method unchanged */ }

#[async_trait]
pub trait BlockedAuthFlowCanceller: Send + Sync { /* existing method unchanged */ }

pub async fn enrich_auth_prompt_view(
    view: AuthPromptView,
    fallback_owner_user_id: &UserId,
    scope: &TurnScope,
    credential_requirements: &[RuntimeCredentialAuthRequirement],
    auth_challenges: Option<&dyn AuthChallengeProvider>,
) -> Result<AuthPromptView, ProductAdapterError>;

#[derive(Debug, Default)]
pub struct ApprovalPromptLookup {
    pub context: Option<ApprovalPromptContextView>,
    pub invocation_id: Option<InvocationId>,
}

pub async fn approval_prompt_lookup(
    approval_requests: Option<&dyn ApprovalRequestStore>,
    gate_ref: &GateRef,
    owner_user_id: &UserId,
    turn_scope: &TurnScope,
) -> ApprovalPromptLookup;
```

- [x] **Step 1: Add product-workflow tests for auth and approval projection**

Move the existing requirement-fallback and approval-context cases to module tests beside the new owners. Add a direct assertion that `enrich_auth_prompt_view` enriches an existing `AuthPromptView` and does not require a request DTO.

- [x] **Step 2: Run the new focused tests and verify unresolved imports**

Run:

```bash
cargo test -p ironclaw_product auth_prompt
cargo test -p ironclaw_product approval_prompt
```

Expected: compilation fails because the new modules/exports do not exist.

- [x] **Step 3: Move the implementations and remove the crossing DTO**

Move the existing bodies without changing error mapping. Delete `BlockedAuthPromptRequest`; callers construct their existing base `AuthPromptView` and call `enrich_auth_prompt_view`. Move the complete approval lookup/action/scope/detail rendering helper family so WebUI projection and channel delivery read the same implementation.

- [x] **Step 4: Rewire composition and run owner/caller tests**

Run:

```bash
cargo test -p ironclaw_product auth_prompt
cargo test -p ironclaw_product approval_prompt
cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,slack-v2-host-beta,telegram-v2-host-beta,libsql --lib projection
```

Expected: all selected tests pass with composition implementing and re-exporting the product-workflow ports.

- [x] **Step 5: Commit the contract move**

```bash
git add crates/ironclaw_product crates/ironclaw_reborn_composition
git commit -m "refactor(reborn): move prompt projection contracts below composition"
```

---

### Task 3: Extract and split the generic channel delivery engine

**Files:**
- Create: `crates/ironclaw_channel_delivery/Cargo.toml`
- Create: `crates/ironclaw_channel_delivery/AGENTS.md`
- Create: `crates/ironclaw_channel_delivery/src/{lib,services,observer,actionable,routing,hooks,triggered,tests}.rs`
- Modify: root `Cargo.toml`
- Modify: `crates/ironclaw_reborn_composition/Cargo.toml`
- Modify: `crates/ironclaw_reborn_composition/src/outbound/mod.rs`
- Delete: `crates/ironclaw_reborn_composition/src/outbound/channel_delivery.rs`
- Modify: all Slack, Telegram, runtime, and test imports of `crate::outbound::channel_delivery`.

**Interfaces:**
- Consumes: product-workflow prompt helpers, `ChannelDeliveryProtocol`, outbound stores/policy, product adapter/egress, thread/turn services, and trigger events.
- Produces the existing public behavior under `ironclaw_channel_delivery::{FinalReplyDeliveryObserver, FinalReplyDeliveryServices, FinalReplyDeliverySettings, PostSubmitDeliveryHook, NoopPostSubmitDeliveryHook, CompositePostSubmitDeliveryHook, TriggeredRunDeliveryDriver}`.

- [x] **Step 1: Create the crate manifest and a failing public API compile test**

The manifest has layer `products`, no default features, and only neutral production dependencies: `async-trait`, `chrono`, `ironclaw_channel_host`, `ironclaw_conversations`, `ironclaw_host_api`, `ironclaw_outbound`, `ironclaw_product_adapters`, `ironclaw_product`, `ironclaw_run_state`, `ironclaw_threads`, `ironclaw_triggers`, `ironclaw_turns`, `ironclaw_wasm_product_adapters`, `tokio`, and `tracing`. The two additional authority/conversation crates are required by the preserved gate-route and fallback-agent signatures; neither owns a concrete channel or composition policy.

Add a crate test that constructs `FinalReplyDeliverySettings::default()` and asserts all four bounds are non-zero. Run `cargo test -p ironclaw_channel_delivery`; expected compilation failure because the exported types are absent.

- [x] **Step 2: Move services, observer, actionable, routing, hooks, and triggered code**

Use the locked module responsibilities. Replace composition-qualified prompt/projection calls with product-workflow imports. Make `CompositePostSubmitDeliveryHook` public because runtime assembly is now a downstream consumer. Replace production `NonZeroUsize::new(...).expect(...)` defaults with safe `NonZeroUsize` constants/fallbacks that contain no unwrap/expect.

- [x] **Step 3: Preserve tests at the new owner**

Move generic unit tests into `src/tests.rs`. Replace composition-owned Slack fixtures with a local recording `ChannelDeliveryProtocol`, local target provider, and host-mediated egress recorder. Keep composition Slack/Telegram whole-path tests as downstream contract tests, not as the engine's unit-test dependency.

- [x] **Step 4: Rewire all consumers and delete the old production file**

Re-export only factory-local target registry items from composition `outbound`; import delivery engine types directly from `ironclaw_channel_delivery`. Confirm:

```bash
test ! -e crates/ironclaw_reborn_composition/src/outbound/channel_delivery.rs
rg -n "outbound::channel_delivery" crates
```

Expected: first command succeeds and second command prints no production references.

- [x] **Step 5: Run crate, architecture, Slack, and Telegram delivery tests**

```bash
cargo test -p ironclaw_channel_delivery
cargo test -p ironclaw_architecture --test reborn_dependency_boundaries
cargo test -p ironclaw_architecture --test reborn_composition_boundaries
cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,slack-v2-host-beta,telegram-v2-host-beta,libsql --lib channel_delivery
```

Expected: all pass; generic code contains no `slack` or `telegram` branch.

Observed: `cargo test -p ironclaw_channel_delivery` passed 87 unit tests plus the public API test; targeted Clippy passed with `-D warnings`; both dependency/composition boundary suites passed; the all-feature composition target compiled successfully with 1,529 unrelated tests filtered by the historical `channel_delivery` filter.

- [x] **Step 6: Commit the delivery owner**

```bash
git add Cargo.toml Cargo.lock crates/ironclaw_channel_delivery crates/ironclaw_reborn_composition crates/ironclaw_architecture
git commit -m "refactor(reborn): extract generic channel delivery engine"
```

---

### Task 4: Replace the Telegram lifecycle slot with an ExtensionId-keyed registry

**Files:**
- Create: `crates/ironclaw_product/src/extension_account_setup.rs`
- Modify: `crates/ironclaw_product/src/lib.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Modify: Telegram host assembly and lifecycle tests.
- Delete: `crates/ironclaw_channel_host/src/paired_status.rs`
- Modify: `crates/ironclaw_channel_host/src/lib.rs`

**Interfaces:**
- Produces:

```rust
#[async_trait]
pub trait AccountConnectionStatusSource: Send + Sync + std::fmt::Debug {
    async fn connected(&self, user_id: &UserId) -> Result<bool, AccountConnectionStatusError>;
}

#[derive(Debug, Clone)]
pub struct ExtensionAccountSetupDescriptor {
    pub extension_id: ExtensionId,
    pub auth_requirement: RuntimeCredentialAuthRequirement,
    pub connection_requirement: ChannelConnectionRequirement,
    pub connection_success_message: String,
}

#[derive(Clone, Default)]
pub struct ExtensionAccountSetupRegistry;

impl ExtensionAccountSetupRegistry {
    pub fn declare(&self, descriptor: ExtensionAccountSetupDescriptor) -> bool;
    pub fn connect(
        &self,
        extension_id: &ExtensionId,
        source: Arc<dyn AccountConnectionStatusSource>,
    ) -> bool;
    pub fn descriptor(&self, extension_id: &ExtensionId) -> Option<ExtensionAccountSetupDescriptor>;
    pub async fn missing_requirement(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<Option<RuntimeCredentialAuthRequirement>, ExtensionAccountSetupError>;
}
```

`ExtensionAccountSetupError` has exact variants `HostUnavailable { extension_id }` and `StatusUnavailable { extension_id }` so lifecycle preserves invalid-binding versus transient mapping.

- [x] **Step 1: Add registry state-transition tests**

Test undeclared extension, declared-but-unconnected fail-closed, connected/disconnected users, duplicate declaration, duplicate connection, and status outage. Run `cargo test -p ironclaw_product extension_account_setup`; expected compile failure.

- [x] **Step 2: Implement the registry with a bounded owner-controlled map**

Use `Arc<RwLock<BTreeMap<ExtensionId, Entry>>>`; each entry contains one immutable descriptor plus an `OnceLock<Arc<dyn AccountConnectionStatusSource>>`. Do not add an extension-specific enum or string branch.

- [x] **Step 3: Make lifecycle generic**

Replace `telegram_paired_source` with `account_setups: ExtensionAccountSetupRegistry`. `activation_credential_requirements` calls `missing_requirement(&extension_id, caller)`. Connection projection and success copy consult the descriptor by extension id, then fall back to the existing generic behavior. Remove every production occurrence of `telegram` from `extension_lifecycle.rs`; move Telegram copy and requirement construction into the Telegram crate.

- [x] **Step 4: Register Telegram's descriptor and status source in host assembly**

Telegram exports `telegram_account_setup_descriptor() -> Result<ExtensionAccountSetupDescriptor, TelegramHostBuildError>` and implements `AccountConnectionStatusSource` for `TelegramPairingService`. Composition declares the descriptor while constructing local extension management and connects the pairing service when Telegram mounts are built.

- [x] **Step 5: Delete the old channel-host slot and run lifecycle regressions**

```bash
cargo test -p ironclaw_product extension_account_setup
cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,telegram-v2-host-beta,libsql --lib extension_lifecycle
cargo test -p ironclaw_architecture --test telegram_extension_gates generic_extension_lifecycle_has_no_telegram_knowledge
```

Expected: all pass, including fail-closed and transient-error caller tests.

- [x] **Step 6: Commit the registry**

```bash
git add crates/ironclaw_product crates/ironclaw_channel_host crates/ironclaw_reborn_composition crates/ironclaw_telegram_extension
git commit -m "refactor(reborn): generalize extension account setup gating"
```

---

### Task 5: Collapse Telegram persistence onto one concrete filesystem state

**Files:**
- Create: `crates/ironclaw_telegram_extension/src/state/{mod,records,setup,pairing,bindings,dm_targets}.rs`
- Create: `crates/ironclaw_telegram_extension/src/test_support.rs`
- Delete: `crates/ironclaw_telegram_extension/src/telegram_host_state.rs`
- Modify: setup, pairing, dispatch, serve/ingress, and outbound target services/tests.
- Modify: composition factory's Telegram filesystem field to erase `RootFilesystem` before entering the Telegram owner.

**Interfaces:**
- Produces:

```rust
#[derive(Clone)]
pub struct FilesystemTelegramHostState {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
    locks: Arc<KeyedAsyncLocks>,
}

impl FilesystemTelegramHostState {
    pub fn new(
        filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self;
    // Existing setup, pairing, binding, and DM-target methods become inherent async methods.
}
```

- [x] **Step 1: Rewrite state tests against production state plus InMemoryBackend**

Add `test_support::telegram_state()` that erases `Arc<InMemoryBackend>` to `Arc<dyn RootFilesystem>` before constructing `ScopedFilesystem`. Move serialization, CAS single-claim, rotation, binding scope, and DM-target tests to the split state modules.

- [x] **Step 2: Run the focused state tests and observe trait-dependent compile failures**

Run: `cargo test -p ironclaw_telegram_extension state`

Expected: compile failures identify services still accepting `dyn Telegram*Store`.

- [x] **Step 3: Convert all four store trait implementations into inherent methods**

Delete `TelegramInstallationSetupStore`, `TelegramPairingStore`, `TelegramUserBindingStore`, and `TelegramDmTargetStore`. Give `TelegramSetupService`, `TelegramPairingService`, and `TelegramOutboundTargetProvider` one shared `Arc<FilesystemTelegramHostState>` and preserve existing lock/CAS/error behavior.

- [x] **Step 4: Replace domain store fakes with lower-seam failure injection**

Implement a test-only `RootFilesystem` decorator that delegates all methods and can fail selected read/write/delete/CAS calls or hold a read barrier. Rewrite setup rollback, pairing outage, and concurrent claim tests to use the concrete state over that decorator.

- [x] **Step 5: Run state, pairing, setup, and architecture tests**

```bash
cargo test -p ironclaw_telegram_extension state
cargo test -p ironclaw_telegram_extension telegram_setup
cargo test -p ironclaw_telegram_extension telegram_pairing
cargo test -p ironclaw_architecture --test telegram_extension_gates telegram_tests_use_the_real_filesystem_state
```

Expected: all pass and `rg -n "(TelegramInstallationSetupStore|TelegramPairingStore|TelegramUserBindingStore|TelegramDmTargetStore|struct InMemory.*Store)" crates/ironclaw_telegram_extension` prints nothing.

Observed: all eight concrete state tests, all 100 Telegram crate tests, the real-state
architecture ratchet, Telegram/channel-host/conversation/product-workflow targeted Clippy,
and the Telegram composition feature check pass. The deleted-trait/test-store scan prints
nothing. Failure and contention coverage now injects read, write, delete, versioned-write,
and read-barrier behavior below the concrete state at the `RootFilesystem` seam.

- [x] **Step 6: Commit concrete state**

```bash
git add crates/ironclaw_telegram_extension crates/ironclaw_reborn_composition
git commit -m "refactor(telegram): use one concrete filesystem host state"
```

---

### Task 6: Make Telegram Bot API, egress credentials, and installation resolution concrete

**Files:**
- Move/modify: `crates/ironclaw_telegram_extension/src/telegram_bot_api.rs` to `src/bot_api.rs`
- Move/modify: `crates/ironclaw_telegram_extension/src/telegram_egress.rs` to `src/egress.rs`
- Modify: `crates/ironclaw_telegram_extension/src/ingress/resolver.rs`
- Modify: setup, ingress, dispatch, and egress tests.

**Interfaces:**
- Produces concrete `HostEgressTelegramBotApi`, `TelegramProtocolHttpEgress`, and `DynamicTelegramInstallationResolver` values. The only retained lower seams are `HostRuntimeHttpEgressPort` and `TelegramRevisionWorkflowBuilder`.

- [x] **Step 1: Convert Bot API setup tests to mediated HTTP recordings**

Use the host-runtime test egress double to assert method names, placeholder URLs, credential target, request JSON, success envelopes, provider rejection, malformed envelopes, and compensation ordering. Run the selected setup/Bot API tests and verify they fail while setup still requires `Arc<dyn TelegramBotApi>`.

- [x] **Step 2: Delete `TelegramBotApi` and call inherent client methods**

Change `TelegramSetupService.bot_api` to `Arc<HostEgressTelegramBotApi>`. Retain `get_me`, `set_webhook`, `delete_webhook`, and `send_message` as inherent async methods; never expose token bytes or weaken network policy.

- [x] **Step 3: Delete the egress credential-provider trait and wrapper**

Change `TelegramProtocolHttpEgress.credentials` to `Arc<TelegramSetupService>` and call a crate-visible concrete token-resolution method that validates the opaque handle. Delete `TelegramEgressCredentialProvider` and `SetupServiceTelegramEgressCredentialProvider`; egress tests persist a setup/token in the real filesystem/secret backends.

- [x] **Step 4: Delete installation resolver trait and ingress wrapper DTO**

Change `TelegramIngressService` and route state to hold `Arc<DynamicTelegramInstallationResolver>`. Delete `TelegramInstallationResolver` and `ResolvedTelegramIngress`; return/use `ResolvedTelegramInstallation` directly. Resolver tests inject only `TelegramRevisionWorkflowBuilder` and concrete state/setup services.

- [x] **Step 5: Run security and deleted-symbol regressions**

```bash
cargo test -p ironclaw_telegram_extension bot_api
cargo test -p ironclaw_telegram_extension egress
cargo test -p ironclaw_telegram_extension ingress
cargo test -p ironclaw_architecture --test telegram_extension_gates deleted_telegram_abstractions_and_dtos_stay_deleted
```

Expected: all pass; `rg -n "(trait TelegramBotApi|TelegramEgressCredentialProvider|TelegramInstallationResolver|ResolvedTelegramIngress)" crates` prints nothing.

Observed: all 101 Telegram tests pass, including concrete mediated-request shape, provider
rejection, malformed envelope, compensation, egress injection/retry, revision replacement, and
route tests. Targeted Telegram Clippy and the Telegram composition feature check pass. The exact
client/provider/resolver symbol scan prints nothing; the combined architecture ratchet now reports
only `TelegramPairingStatusResponse`, intentionally removed by Task 7.

- [x] **Step 6: Commit concrete clients/resolver**

```bash
git add crates/ironclaw_telegram_extension crates/ironclaw_reborn_composition
git commit -m "refactor(telegram): remove test-only client and resolver traits"
```

---

### Task 7: Remove mirror DTOs and split Telegram modules below 1,000 lines

**Files:**
- Create/move files in the locked `setup`, `pairing`, `ingress`, `delivery`, and `state` layouts.
- Rename: `telegram_channel_routes.rs` to `channel_routes.rs`.
- Modify: `crates/ironclaw_telegram_extension/src/lib.rs` and all downstream imports.
- Delete all superseded `telegram_*.rs` monoliths once consumers use the focused owners.

**Interfaces:**
- Preserves public behavior types `TelegramInstallationSetup`, `TelegramInstallationSetupUpdate`, `TelegramInstallationSetupStatus`, `TelegramPairingStatus`, `PairingIssue`, `PairingConsumeOutcome`, `ResolvedTelegramInstallation`, `TelegramDeliveryProtocol`, and `TelegramOutboundTargetProvider`.

- [x] **Step 1: Add route JSON compatibility assertions before deleting the DTO**

In `channel_routes.rs` handler tests, serialize `TelegramPairingStatus` for connected and pending cases and compare exact JSON objects to the existing route response. Run the two tests and verify they pass against the current mirror response.

- [x] **Step 2: Return the owned projection directly**

Delete `TelegramPairingStatusResponse`; make `pairing_status_handler` return `Json<TelegramPairingStatus>`. Remove unused `agent_id`, `project_id`, `bot_api`, and other public accessors proven to have no production caller by `rg`.

- [x] **Step 3: Split each module by the locked responsibility**

Move record definitions without changing serde names. Keep service orchestration in `service.rs`, code mint/validation in `code.rs`, HTTP parsing/routing in `route.rs`, revision resolution/cache in `resolver.rs`, protocol rendering in `protocol.rs`, target authority in `targets.rs`, and persistence operations in state-specific modules.

- [x] **Step 4: Narrow visibility and preserve deliberate re-exports**

Use `pub(crate)` for cross-module-only helpers. `lib.rs` exports focused modules; temporary old-path re-exports are allowed only when a real downstream crate still imports the symbol and are deleted after that consumer migrates in this task.

- [x] **Step 5: Enforce line and DTO ratchets**

```bash
cargo test -p ironclaw_telegram_extension
cargo test -p ironclaw_architecture --test telegram_extension_gates telegram_production_files_meet_the_line_budget
cargo test -p ironclaw_architecture --test telegram_extension_gates deleted_telegram_abstractions_and_dtos_stay_deleted
```

Expected: all pass; every touched production Telegram `.rs` file is at most 999 lines.

Observed: all 104 Telegram tests pass; targeted Clippy is warning-free; the focused line-budget
and deleted-symbol architecture ratchets pass. The Telegram composition feature compiles after all
downstream imports moved to the focused namespaces. The full Telegram architecture test now fails
only on the intentionally red Task 8 composition-ownership ratchet.

- [x] **Step 6: Commit the focused layout**

```bash
git add crates/ironclaw_telegram_extension crates/ironclaw_reborn_composition crates/ironclaw_architecture
git commit -m "refactor(telegram): split host by domain responsibility"
```

---

### Task 8: Move Telegram revision and triggered-delivery behavior out of composition

**Files:**
- Create: `crates/ironclaw_telegram_extension/src/host/{mod,builder,revision}.rs`
- Create/modify: `crates/ironclaw_telegram_extension/src/delivery/triggered.rs`
- Modify: `crates/ironclaw_telegram_extension/Cargo.toml`
- Reduce: `crates/ironclaw_reborn_composition/src/telegram/telegram_host_beta.rs`
- Modify: composition Telegram host tests.

**Interfaces:**
- Produces:

```rust
pub struct TelegramHostConfig { /* same five identity/public-origin fields */ }

pub struct TelegramHostInput {
    pub config: TelegramHostConfig,
    pub state: Arc<FilesystemTelegramHostState>,
    pub secret_store: Arc<dyn SecretStore>,
    pub host_egress: HostRuntimeHttpEgressPort,
    pub continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    pub conversation_bindings: Arc<dyn ironclaw_conversations::ConversationBindingService>,
    pub actor_pairings: Arc<dyn ConversationActorPairingService>,
    pub idempotency_ledger: Arc<dyn IdempotencyLedger>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub approval_interactions: Arc<dyn ApprovalInteractionService>,
    pub auth_interactions: Arc<dyn AuthInteractionService>,
    pub delivery_services: TelegramDeliveryServicePorts,
    pub setup_activation: Option<Arc<dyn TelegramChannelSetupActivation>>,
}

pub struct TelegramHostParts {
    pub updates: TelegramUpdatesRouteState,
    pub channel_routes: TelegramChannelRouteConfig,
    pub connectable: Arc<dyn ConnectableChannelsProductFacade>,
    pub channel_connection: Arc<dyn ChannelConnectionFacade>,
    pub outbound_targets: Arc<dyn OutboundDeliveryTargetProvider>,
    pub trigger_hook: Arc<dyn PostSubmitDeliveryHook>,
    pub account_status: Arc<dyn AccountConnectionStatusSource>,
}

pub async fn build_telegram_host(input: TelegramHostInput) -> Result<TelegramHostParts, TelegramHostBuildError>;
```

- [x] **Step 1: Add Telegram-owner revision/hook tests**

Move the same-revision cache, newer-revision replacement, unconfigured skip, first-configure, bot-swap, target-provider key stability, and trigger-hook behavior tests into `host/revision.rs` and `delivery/triggered.rs`. Run them; expected unresolved owner types.

- [x] **Step 2: Move workflow construction and cache behavior**

Move `TelegramRevisionWorkflowParts`, adapter construction, revision workflow implementation, dynamic trigger driver/cache, Telegram no-op binding/sink implementations, egress scope, and provider-key hashing into the Telegram crate. Depend on `ironclaw_channel_delivery`, `ironclaw_outbound`, `ironclaw_run_state`, `ironclaw_threads`, and `ironclaw_triggers`; do not add a composition dependency.

- [x] **Step 3: Implement the facade-shaped Telegram builder**

Construct state-dependent setup/pairing/egress/resolver/route/facade/provider/hook parts in `build_telegram_host`. Return only facade/port shapes; do not accept `RebornRuntime` or global listener/mount types.

- [x] **Step 4: Reduce composition to extraction, mounting, and registration**

Composition constructs the scoped filesystem, conversation/idempotency services, product-auth/approval ports, and optional setup activation adapter; calls `build_telegram_host`; wraps routes as `PublicRouteMount`/`ProtectedRouteMount`; registers target provider, account status, and keyed trigger hook. It contains no `TelegramRevisionWorkflowParts`, `DynamicTelegramTriggeredRunDeliveryHook`, cached driver, adapter construction, or Telegram no-op implementation.

- [x] **Step 5: Run owner and composition assembly tests**

```bash
cargo test -p ironclaw_telegram_extension host
cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,telegram-v2-host-beta,libsql --lib telegram
cargo test -p ironclaw_architecture --test telegram_extension_gates telegram_composition_is_assembly_only
```

Expected: all pass and composition's production Telegram module stays within the ratcheted assembly budget.

Observed: 108 Telegram owner tests pass, including revision-cache reuse/replacement/stale-race,
first-configure, bot-swap, and provider-key stability. Eleven Telegram-filtered composition tests
pass with the full feature set. Targeted Telegram and composition Clippy are warning-free, all nine
Telegram architecture ratchets pass, and composition's production adapter is 286 lines.

- [x] **Step 6: Commit behavior ownership**

```bash
git add crates/ironclaw_telegram_extension crates/ironclaw_reborn_composition crates/ironclaw_architecture
git commit -m "refactor(telegram): own revision and delivery runtime behavior"
```

---

### Task 9: Update contracts, crate guidance, and the material checklist

**Files:**
- Modify: `crates/AGENTS.md`
- Modify: `crates/ironclaw_channel_host/AGENTS.md`
- Create/update: `crates/ironclaw_channel_delivery/AGENTS.md`
- Modify: `crates/ironclaw_telegram_extension/AGENTS.md`
- Modify: `docs/reborn/contracts/telegram-v2.md`
- Modify: `docs/superpowers/specs/2026-07-17-pr-6159-architecture-simplification-design.md`
- Modify: this plan.

**Interfaces:**
- Produces: documented ownership and evidence-linked A1-A12 checklist; no behavior change.

- [x] **Step 1: Update ownership guidance**

Document channel-host contracts versus delivery-engine behavior, concrete Telegram state, generic account-setup registry, Telegram builder ownership, and composition's mount/registration role. Keep the HTTP/persistence/security contract text unchanged.

- [x] **Step 2: Mark only materially complete acceptance rows**

For A1-A8, replace `[ ]` with `[x]` only after the corresponding focused tests and ratchets pass. Add the validating command/commit beside each checked row. Leave A9-A12 unchecked until Task 10.

- [x] **Step 3: Verify documentation names and old paths**

Run:

```bash
rg -n "outbound/channel_delivery.rs|TelegramInstallationSetupStore|TelegramPairingStore|TelegramUserBindingStore|TelegramDmTargetStore|TelegramEgressCredentialProvider|TelegramInstallationResolver|TelegramPairingStatusResponse|ResolvedTelegramIngress" docs crates --glob '*.md'
```

Expected: only historical explanation in the approved design/plan and explicit deleted-symbol ratchet documentation remain.

Observed: active crate guidance and the Telegram contract contain only current owner paths.
Deleted names remain solely in the dated original/current implementation plans, the approved
architecture design's deletion rationale, and the explicit architecture ratchet documentation.

- [x] **Step 4: Commit guidance/checklist updates**

```bash
git add crates/AGENTS.md crates/ironclaw_channel_host/AGENTS.md crates/ironclaw_channel_delivery/AGENTS.md crates/ironclaw_telegram_extension/AGENTS.md docs
git commit -m "docs(reborn): record channel and Telegram architecture owners"
```

---

### Task 10: Run the complete regression, quality, and scope audit

**Files:**
- Modify only files required to fix defects exposed by these commands.
- Update: design and plan A9-A12 evidence after every gate is green.

**Interfaces:**
- Produces: merge-ready evidence without claiming unavailable checks.

- [x] **Step 1: Run the contract regression matrix**

```bash
cargo test -p ironclaw_channel_delivery
cargo test -p ironclaw_channel_host --features webhook-serve
cargo test -p ironclaw_telegram_extension
cargo test -p ironclaw_telegram_v2_adapter --lib
cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,slack-v2-host-beta,telegram-v2-host-beta,libsql --lib
cargo test -p ironclaw_architecture
cargo test --test telegram_v2_default_off_integration
bash scripts/reborn-e2e-rust.sh
```

Expected: all commands pass. If an external service is genuinely required, capture the exact unavailable dependency and do not mark its row complete.

Observed on final source: `ironclaw_channel_delivery` passes 87 unit tests plus its public-API
contract, `ironclaw_channel_host` passes 3 webhook-host tests, `ironclaw_telegram_extension`
passes 108 tests, `ironclaw_telegram_v2_adapter` passes 60 tests, the full-feature composition
suite passes 1,528 tests, the architecture crate passes 8 composition + 34 dependency + 9
Telegram tests, and the default-off integration passes 10 tests. `scripts/reborn-e2e-rust.sh`
passes all 50 deterministic contract binaries. No external service was required or skipped.

- [x] **Step 2: Run targeted and workspace Clippy**

```bash
cargo clippy -p ironclaw_channel_delivery --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_channel_host --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_telegram_extension --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: zero warnings.

Observed: all four targeted commands and `cargo clippy --workspace --all-targets
--all-features -- -D warnings` exit successfully with zero Rust/Clippy warnings. Cargo emits
only the repository's pre-existing `net.retries` configuration-key notice, outside Clippy.

- [x] **Step 3: Run source-safety and sibling-pattern audits**

```bash
rg -n "\.unwrap\(|\.expect\(" crates/ironclaw_channel_delivery/src crates/ironclaw_telegram_extension/src
rg -n "Telegram(InstallationSetupStore|PairingStore|UserBindingStore|DmTargetStore|EgressCredentialProvider|InstallationResolver|BotApi)|TelegramPairingStatusResponse|ResolvedTelegramIngress" crates
rg -n "struct InMemory.*Store" crates/ironclaw_telegram_extension
git diff --check origin/pr-6159...HEAD
scripts/pre-commit-safety.sh
```

Expected: production unwrap/expect scan is empty, deleted-symbol and test-store scans are empty outside ratchet text, diff check and safety script pass.

Observed: the production-aware panic scan is empty; raw matches are confined to test files or
`#[cfg(test)]` modules. Deleted Telegram names occur only in the architecture ratchet itself,
the Telegram crate has no `InMemory*Store`, the old composition delivery file is absent, and
both working-tree and baseline diff checks pass. Because `origin/pr-6159` was deleted during
implementation, `pre-commit-safety.sh` was run with that upstream ref temporarily restored to
the locked baseline `0575d381...`; it passes, including composition at 22.21% of production LOC
and 1,109 non-exempt composition `Arc<dyn>` uses against the 1,156 ceiling. The audit also
replaced both new SHA-256 byte-prefix slices with character-safe prefix extraction.

- [x] **Step 4: Audit exact scope and compatibility**

Inspect `git diff --stat origin/pr-6159...HEAD`, `git diff --name-status origin/pr-6159...HEAD`, public serde definitions, route constants, manifest files, secret-handle constants, and persistence path constants. Confirm no rebase, schema/wire/route/manifest/secret/persistence change, unrelated cleanup, or full URT migration entered the diff.

Observed against locked baseline `0575d381...`: 102 files are changed by the documented owner
moves and module splits. The Telegram manifest asset is byte-identical; route IDs/paths, webhook
header, adapter/provider IDs, credential-handle strings, six filesystem roots, feature gates,
and exact connected/pending status JSON remain unchanged. No migration, manifest asset, static
frontend, persisted schema, or wire definition changed. The only new runtime identifiers are
the provider-neutral extension registration key and a stable internal outbound-provider cache
key. The branch is a descendant of the PR baseline; it contains no rebase or full URT migration.

- [x] **Step 5: Complete A9-A12 and commit final evidence**

Check A9 for Telegram contract suites, A10 for shared Slack/Telegram delivery, A11 for quality gates, and A12 for scope audit. Record exact commands and outcomes in the approved design and this plan.

```bash
git add docs/superpowers/specs/2026-07-17-pr-6159-architecture-simplification-design.md docs/superpowers/plans/2026-07-17-pr-6159-architecture-simplification.md
git commit -m "docs(reborn): complete PR 6159 architecture evidence"
```

## Material Acceptance Matrix

- [x] **A1 — Delivery ownership:** Task 3; `cargo test -p ironclaw_channel_delivery`, targeted Clippy, both Reborn boundary suites, and the deleted composition path ratchet pass.
- [x] **A2 — Concrete Telegram state:** Task 5; eight concrete-state tests, the 100-test
  Telegram suite, targeted Clippy, the deleted-trait/test-store scan, real-state ratchet, and
  Telegram composition feature check pass.
- [x] **A3 — Concrete Bot API:** Task 6; 101 Telegram tests, mediated request/security tests,
  targeted Clippy, exact client/provider/resolver scan, and composition feature check pass.
- [x] **A4 — DTO cleanup:** Tasks 2, 6, and 7; exact JSON tests, 104-test Telegram suite,
  and deleted-symbol/accessor audit (`f1668d763`).
- [x] **A5 — Generic lifecycle:** Task 4; six registry transition tests, 101 focused
  lifecycle tests, Telegram descriptor and idempotent mount regressions, the lifecycle source
  ratchet, and targeted product-workflow/Telegram/composition Clippy all pass.
- [x] **A6 — Telegram-owned runtime behavior:** Task 8; 108 owner tests, 11
  Telegram-filtered composition tests, targeted Clippy, and assembly-only ratchet (`e1f660030`).
- [x] **A7 — Focused files:** Tasks 5, 7, and 8; 999-line physical-file ratchet.
- [x] **A8 — Ratchets:** Task 1, turned green by Tasks 3-8; all nine Telegram architecture
  tests pass.
- [x] **A9 — Contract preservation:** 108 Telegram owner tests, 60 adapter tests, 10
  default-off tests, the full composition suite, and exact route JSON/manifest projections pass.
- [x] **A10 — Cross-channel preservation:** 87 shared-engine tests plus its public API test and
  all 1,528 Slack/Telegram-enabled composition tests pass; generic behavior dispatches through
  `ChannelDeliveryProtocol` rather than a channel-name branch while legacy identity-key text is
  deliberately preserved for compatibility.
- [x] **A11 — Quality gates:** all targeted and workspace Clippy, 51 architecture tests, 50
  Reborn E2E binaries, production safety scans, and baseline-scoped pre-commit safety pass.
- [x] **A12 — Scope audit:** the final baseline diff, public constants, exact JSON tests,
  manifest asset, secret handles, persistence roots, docs, and crate guidance are verified.
