# Telegram Extension Implementation Plan

> **Historical plan — do not execute as a current lifecycle contract.**
> This document predates the generic extension-correctness work and retains
> implementation-era names such as `extension_activate`,
> `activation_credential_requirements`, and “fake activation.” Those names are
> historical evidence, not supported product behavior. The current public
> lifecycle is derived, generically and without an Activate action:
>
> - no personal membership → `uninstalled`;
> - membership plus incomplete manifest-declared personal setup → `setup_needed`;
> - membership plus satisfied setup and proven runtime readiness → `active`.
>
> Installing Telegram creates personal membership and, while pairing is
> incomplete, returns the manifest-declared pairing requirement. Consuming the
> host-generated code/deep link completes setup, resumes blocked runs, and
> derives `active`; there is no `extension_activate` tool, route, button, or
> stored public activation state. Tenant-admin bot configuration remains a
> separate tenant-scoped authority and never installs Telegram for a user.
> Use
> `docs/superpowers/plans/2026-07-22-generic-extension-correctness-merge-readiness.md`
> and the live manifest/contracts for current implementation and release
> decisions.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the single `telegram` extension on Reborn main — admin bot setup (Channels tab), webhook-only ingress, WebGeneratedCode pairing with BlockedAuth park/resume, DM-only messaging, proactive delivery, zero tools — per `docs/superpowers/specs/2026-07-16-telegram-extension-design.md`.

**Architecture:** Clone the `crates/ironclaw_reborn_composition/src/slack/**` host-module shape as `telegram/**` behind cargo feature `telegram-v2-host-beta`, reusing the unwired `ironclaw_telegram_v2_adapter` (ProductAdapter) and the existing park/resume machinery (`RuntimeCredentialAuthRequirement` → `BlockedAuth` → `BlockedAuthResumeFanout`, all provider-string-keyed on `"telegram"`). Pairing state lives in telegram host state (filesystem-over-backend); no credential accounts are minted for pairing — the gate is synthesized from pairedness.

**Tech Stack:** Rust (axum, tokio, async_trait, secrecy, sha2, rand), React/TSX (Vite, react-query, `qrcode` npm dep), tests via the `RebornIntegrationHarness` scripted tier + crate/contract tests + arch gates.

## Global Constraints

- Names pinned (spec §1): extension id `telegram`; handles `telegram_bot_token`, `telegram_webhook_secret`; provider `telegram`; adapter id `telegram_v2`; actor kind `telegram_user`; route id `telegram.updates`; route `POST /webhooks/extensions/telegram/updates`; strategy `RebornChannelConnectStrategy::WebGeneratedCode`; feature `telegram-v2-host-beta`; env `IRONCLAW_REBORN_TELEGRAM_ENABLED`.
- Never introduce `telegram_bot`/`telegram_personal`/`_bot`/`_personal` identifiers (retired taxonomy). Telegram strings stay inside `composition/src/telegram/**`, the manifest asset, the adapter crate, serve wiring, and tests — mirror the #6116 specificity boundary.
- No `.unwrap()`/`.expect()` in production code; `thiserror` errors; `crate::` imports; prompt-like copy inline only if single-line.
- Pairing: 8-char alphabet `ABCDEFGHJKLMNPQRSTUVWXYZ23456789`, OS CSPRNG, 15-min TTL, single-use, one live code per user, rotate-on-reissue. Codes expire; gates don't.
- DM-only: non-private chats, `channel_post`, `edited_message`, bot senders ⇒ no turn, no reply. Unpaired DM ⇒ static throttled hint, never LLM.
- Honest delivery: provider error ⇒ `Failed*` statuses (adapter already maps 5xx/401/400); never optimistic Delivered.
- Both manifests' credential coherence: `telegram_webhook_secret` verifies ingress; `telegram_bot_token` is egress-only.
- Feature declared everywhere it's referenced + CI flag files updated (`scripts/ci/package-feature-flags.sh`, bucket file already lists the adapter crate).
- Every task: red test first → fail for the right reason → implement → green → `cargo fmt` → commit.

## Execution notes (single-session, owner AFK)

Owner pre-approved inline execution straight to a single PR (base `main`, origin `nearai/ironclaw`, non-draft). Priority order if time runs short: Tasks 1–10 (backend + integration tests) > Task 11 (frontend) > Task 12 (legacy purge) > Task 13 (docs/QA polish). Cut line is task-granular; cuts listed in the PR body.

---

### Task 1: Feature flag + manifest + catalog entry

**Files:**
- Create: `crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml`
- Modify: `crates/ironclaw_reborn_composition/Cargo.toml` (features block ~L74), `crates/ironclaw_reborn_cli/Cargo.toml` (~L51), `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`, `scripts/ci/package-feature-flags.sh` (L44/L55)
- Test: extend `crates/ironclaw_reborn_composition` tests (new `#[cfg(feature = "telegram-v2-host-beta")]` test in `available_extensions.rs` tests mod)

**Interfaces:**
- Produces: `telegram_manifest_toml() -> &'static str` and `telegram_package() -> Result<AvailableExtensionPackage, ProductWorkflowError>` in `available_extensions.rs`, both `#[cfg(feature = "telegram-v2-host-beta")]`; `pub(crate) const TELEGRAM_EXTENSION_ID: &str = "telegram";` there too. Catalog builder `from_first_party_assets_with_nearai_mcp_config` pushes `telegram_package()` under the feature.
- Consumes: `bundled_extension_package(id, label, manifest_toml, assets)` (available_extensions.rs:771).

Manifest content (v2 schema; the registry fixture at `ironclaw_product_adapter_registry/src/lib.rs:936` already validates this exact shape — only our route path differs):

```toml
schema_version = "reborn.extension_manifest.v2"
id = "telegram"
name = "Telegram"
version = "0.1.0"
description = "Telegram Bot API channel: DM IronClaw on Telegram after pairing your account."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "telegram_v2_host_beta"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "shared_secret_header"
header_name = "X-Telegram-Bot-Api-Secret-Token"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages", "external_final_reply_push", "delivery_status_reporting"]

[[product_adapter.inbound.required_credentials]]
handle = "telegram_bot_token"

[[product_adapter.inbound.required_credentials]]
handle = "telegram_webhook_secret"

[[product_adapter.inbound.egress]]
host = "api.telegram.org"
credential_handle = "telegram_bot_token"

[[product_adapter.inbound.host_ingress]]
credential_handles = ["telegram_webhook_secret"]

[product_adapter.inbound.host_ingress.descriptor]
route_id = "telegram.updates"
method = "post"
route_pattern = "/webhooks/extensions/telegram/updates"

[product_adapter.inbound.host_ingress.descriptor.policy]
listener_class = "public_webhook"
auth = { type = "required", schemes = ["webhook_signature"] }
scope_source = "host_resolved"
body_limit = { type = "limited", max_bytes = 1048576 }
rate_limit = { type = "limited", scope = "global", max_requests = 12000, window_seconds = 60 }
cors = "not_applicable"
websocket_origin = "not_applicable"
streaming = "none"
audit = "public_callback"
effect_path = { type = "product_workflow" }
```

Cargo features (mirror slack lines exactly):
```toml
# composition Cargo.toml [features]
telegram-v2-host-beta = ["webui-v2-beta", "dep:ironclaw_telegram_v2_adapter", "dep:ironclaw_wasm_product_adapters", "ironclaw_product/storage"]
# + [dependencies] ironclaw_telegram_v2_adapter = { path = "../ironclaw_telegram_v2_adapter", optional = true }
# cli Cargo.toml [features]
telegram-v2-host-beta = ["webui-v2-beta", "ironclaw_reborn_composition/telegram-v2-host-beta"]
```
CI: append `,telegram-v2-host-beta` to the cli (L44) and composition (L55) feature lists in `scripts/ci/package-feature-flags.sh`.

- [ ] **Step 1: Red test** in `available_extensions.rs` tests:
```rust
#[cfg(feature = "telegram-v2-host-beta")]
#[test]
fn telegram_package_is_visible_channel_with_zero_tools() {
    let package = telegram_package().expect("telegram manifest parses");
    assert_eq!(package.package_ref.id.as_str(), "telegram");
    assert!(!is_internal_extension_package_ref(&package.package_ref));
    assert!(package.surface_kinds.contains(&LifecycleExtensionSurfaceKind::ExternalChannel));
    assert!(package.package.manifest.capabilities.is_empty(), "telegram must expose zero tools");
}
```
- [ ] **Step 2:** `cargo test -p ironclaw_reborn_composition --features telegram-v2-host-beta telegram_package_is_visible` → FAIL (fn undefined)
- [ ] **Step 3:** Add manifest asset, features, `telegram_manifest_toml()` (include_str), `telegram_package()` via `bundled_extension_package("telegram", "Telegram", ...)`, catalog push, CI script lines.
- [ ] **Step 4:** test PASSES; also `cargo check -p ironclaw_reborn_composition` (feature OFF) still green.
- [ ] **Step 5:** Commit `feat(reborn): telegram manifest, feature flag, catalog entry`.

---

### Task 2: Telegram host state + setup service (getMe/setWebhook via injected Bot API port)

**Files:**
- Create: `crates/ironclaw_reborn_composition/src/telegram/mod.rs`, `telegram/telegram_setup.rs`, `telegram/telegram_host_state.rs`, `telegram/telegram_bot_api.rs`
- Modify: `crates/ironclaw_reborn_composition/src/lib.rs` (mod + cfg-gated re-exports mirroring L203-248), lib.rs mount-view fn (mirror `slack_host_state_mount_view` L708 with aliases `/tenant-shared/telegram-setup`, `/tenant-shared/telegram-pairing`, `/tenant-shared/telegram-binding`, `/tenant-shared/telegram-dm-targets`)

**Interfaces (produces):**
```rust
// telegram_bot_api.rs — the hermetic seam for api.telegram.org calls at setup time
pub(crate) struct TelegramBotIdentity { pub id: i64, pub username: String }
#[async_trait]
pub(crate) trait TelegramBotApi: Send + Sync + std::fmt::Debug {
    async fn get_me(&self, bot_token: &SecretString) -> Result<TelegramBotIdentity, TelegramBotApiError>;
    async fn set_webhook(&self, bot_token: &SecretString, url: &str, secret_token: &SecretString) -> Result<(), TelegramBotApiError>;
    async fn delete_webhook(&self, bot_token: &SecretString) -> Result<(), TelegramBotApiError>;
    async fn send_message(&self, bot_token: &SecretString, chat_id: i64, text: &str) -> Result<(), TelegramBotApiError>;
}
// production impl ReqwestTelegramBotApi (uses the http client stack composition already links)

// telegram_setup.rs — mirrors slack_setup.rs shapes (SlackInstallationSetup L35-48 etc.)
pub(crate) struct TelegramInstallationSetup {
    bot_id: i64, bot_username: String, webhook_url: String,
    bot_token_handle: SecretHandle, webhook_secret_handle: SecretHandle,
    revision: u64, updated_at: DateTime<Utc>,
}
impl TelegramInstallationSetup {
    pub(crate) fn installation_id(&self) -> Result<AdapterInstallationId, TelegramSetupError>; // "tg-bot-{bot_id}"
}
pub(crate) struct TelegramInstallationSetupUpdate { pub bot_token: Option<SecretString>, pub webhook_url_override: Option<String> }
pub(crate) struct TelegramInstallationSetupStatus { // Serialize only, redacted
    pub configured: bool, pub bot_username: Option<String>, pub bot_token_configured: bool,
    pub webhook_url: Option<String>, pub revision: Option<u64>,
}
#[async_trait] pub(crate) trait TelegramInstallationSetupStore: Send + Sync + std::fmt::Debug {
    async fn get_telegram_installation_setup(&self) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError>;
    async fn put_telegram_installation_setup(&self, setup: &TelegramInstallationSetup) -> Result<(), TelegramSetupError>;
    async fn delete_telegram_installation_setup(&self) -> Result<(), TelegramSetupError>;
}
pub(crate) struct TelegramSetupService { /* tenant_id, agent_id, project_id, operator_user_id, store, secret_store, bot_api: Arc<dyn TelegramBotApi>, public_base_url: Option<String>, save_lock */ }
impl TelegramSetupService {
    pub(crate) fn new(...) -> Self;
    pub(crate) async fn current_setup(&self) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError>;
    pub(crate) async fn status(&self) -> Result<TelegramInstallationSetupStatus, TelegramSetupError>;
    // save pipeline: resolve token (new or existing) → get_me → generate webhook secret →
    // set_webhook(url = override or {public_base}/webhooks/extensions/telegram/updates) →
    // put secrets under handles telegram_bot_token_{hash}_v{rev} / telegram_webhook_secret_{hash}_v{rev} → put record
    pub(crate) async fn save_with_previous(&self, update: TelegramInstallationSetupUpdate)
        -> Result<(Option<TelegramInstallationSetup>, TelegramInstallationSetup), TelegramSetupError>;
    pub(crate) async fn rollback_failed_activation_save(&self, saved: &TelegramInstallationSetup, previous: Option<&TelegramInstallationSetup>) -> Result<(), TelegramSetupError>;
    pub(crate) async fn clear(&self) -> Result<(), TelegramSetupError>; // delete_webhook best-effort + purge secrets + delete record
    pub(crate) async fn bot_token(&self) -> Result<Option<SecretString>, TelegramSetupError>; // lease_once+consume, for ingress/egress wiring
    pub(crate) async fn webhook_secret(&self) -> Result<Option<SecretString>, TelegramSetupError>;
}

// telegram_host_state.rs — FilesystemTelegramHostState<F: RootFilesystem> mirroring FilesystemSlackHostState (slack_host_state.rs:82)
// path consts: TELEGRAM_INSTALLATION_SETUP_PATH="/tenant-shared/telegram-setup/installation.json";
// TELEGRAM_PAIRING_ROOT="/tenant-shared/telegram-pairing"; TELEGRAM_BINDING_ROOT="/tenant-shared/telegram-binding";
// TELEGRAM_DM_TARGET_ROOT="/tenant-shared/telegram-dm-targets"
// implements: TelegramInstallationSetupStore + RebornUserIdentityLookup (slack_actor_identity.rs:28 trait, reused) +
// TelegramPairingStore (Task 3) + TelegramDmTargetStore (Task 3) + TelegramUserBindingStore (Task 3)
```
Setup errors mirror `SlackSetupError` (+`PublicUrlMissing`, `BotApi{reason: String}` variants). Missing public base URL and any `get_me`/`set_webhook` failure abort the save before persistence (fail-closed).

- [ ] **Step 1: Red tests** (in-crate `#[cfg(test)]` with an in-memory `RootFilesystem` and a `RecordingTelegramBotApi` fake): (a) save happy path persists record + both secret handles + calls get_me then set_webhook with generated secret and derived URL; (b) `get_me` error ⇒ nothing persisted; (c) missing public base URL and no override ⇒ `PublicUrlMissing`, no Bot API call; (d) second save (token rotation) bumps revision, new webhook secret, same `bot_id` keeps `installation_id`; (e) `status()` never contains token bytes; (f) `clear()` calls delete_webhook and removes record+secrets but tolerates delete_webhook failure.
- [ ] **Step 2:** run → FAIL (types undefined). **Step 3:** implement per shapes above (copy slack_setup.rs mechanics: save_lock, revision bump, `ensure_existing_secret`, `secret_handle_for_installation` with key material `b"telegram-installation-secret:v1"`). **Step 4:** green + fmt. **Step 5:** commit `feat(reborn): telegram setup service + host state`.

---

### Task 3: Pairing service (issue/rotate/consume) + identity binding + DM targets

**Files:**
- Create: `crates/ironclaw_reborn_composition/src/telegram/telegram_pairing.rs`, `telegram/telegram_actor_identity.rs`
- Modify: `telegram/telegram_host_state.rs` (store impls)

**Interfaces (produces):**
```rust
// telegram_actor_identity.rs
pub(crate) const TELEGRAM_IDENTITY_PROVIDER: &str = "telegram";
pub(crate) const TELEGRAM_V2_ADAPTER_ID: &str = "telegram_v2";
pub(crate) fn telegram_user_identity_provider_user_id(installation_id: &AdapterInstallationId, telegram_user_id: &str) -> String; // "{installation}:{tg_user}"
pub(crate) struct TelegramUserIdentityActorResolver { lookup: Arc<dyn RebornUserIdentityLookup> } // impl ProductActorUserResolver, guard: adapter_id==telegram_v2 && actor kind==TELEGRAM_USER_ACTOR_KIND; mirrors slack_actor_identity.rs:107-172 incl. epoch fast-path

// telegram_pairing.rs
pub(crate) const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub(crate) const PAIRING_CODE_LEN: usize = 8;
pub(crate) const PAIRING_TTL: Duration = Duration::from_secs(15 * 60);
pub(crate) struct TelegramPairingRecord { code: String, tenant_id: TenantId, user_id: UserId, installation_id: AdapterInstallationId, created_at: Timestamp, expires_at: Timestamp, consumed_at: Option<Timestamp> }
#[async_trait] pub(crate) trait TelegramPairingStore: Send + Sync + std::fmt::Debug {
    async fn upsert_pending_pairing(&self, record: TelegramPairingRecord) -> Result<(), TelegramPairingError>; // replaces caller's live code (rotation)
    async fn live_pairing_for_code(&self, code: &str) -> Result<Option<TelegramPairingRecord>, TelegramPairingError>; // uppercased lookup, unexpired+unconsumed
    async fn live_pairing_for_user(&self, user_id: &UserId) -> Result<Option<TelegramPairingRecord>, TelegramPairingError>;
    async fn mark_consumed(&self, code: &str) -> Result<(), TelegramPairingError>;
    async fn invalidate_for_user(&self, user_id: &UserId) -> Result<(), TelegramPairingError>;
}
#[async_trait] pub(crate) trait TelegramUserBindingStore: Send + Sync + std::fmt::Debug { // write/delete side; read side is RebornUserIdentityLookup
    async fn bind_telegram_user(&self, provider_user_id: &str, user_id: &UserId, epoch: &str) -> Result<(), TelegramBindingError>; // AlreadyBoundToOther on conflict with different user
    async fn unbind_telegram_users_for_user(&self, user_id: &UserId, installation_prefix: &str) -> Result<usize, TelegramBindingError>;
}
pub(crate) struct TelegramDmTarget { pub user_id: UserId, pub chat_id: i64 }
#[async_trait] pub(crate) trait TelegramDmTargetStore: Send + Sync + std::fmt::Debug {
    async fn upsert_dm_target(&self, installation_id: &AdapterInstallationId, target: TelegramDmTarget) -> Result<(), TelegramPairingError>;
    async fn dm_target_for_user(&self, installation_id: &AdapterInstallationId, user_id: &UserId) -> Result<Option<TelegramDmTarget>, TelegramPairingError>;
    async fn delete_dm_target_for_user(&self, installation_id: &AdapterInstallationId, user_id: &UserId) -> Result<(), TelegramPairingError>;
}
pub(crate) struct PairingIssue { pub code: String, pub deep_link: String, pub expires_at: Timestamp } // deep_link = https://t.me/{bot_username}?start={code}
pub(crate) enum PairingConsumeOutcome { Paired { user_id: UserId }, AlreadyBoundToOtherUser, ExpiredOrUnknown, AlreadyPairedSameUser { user_id: UserId } }
pub(crate) struct TelegramPairingService { /* pairing_store, binding_store, lookup, dm_target_store, setup: Arc<TelegramSetupService>, continuation: Arc<dyn RebornAuthContinuationDispatcher>, tenant_id, agent_id, project_id */ }
impl TelegramPairingService {
    pub(crate) async fn issue_or_rotate(&self, caller: &UserId) -> Result<PairingIssue, TelegramPairingError>; // errors NotConfigured when no setup
    pub(crate) async fn status_for(&self, caller: &UserId) -> Result<TelegramPairingStatus, TelegramPairingError>; // { connected: bool, pending: Option<PairingIssue> }
    pub(crate) async fn consume(&self, code: &str, telegram_user_id: &str, chat_id: i64) -> Result<PairingConsumeOutcome, TelegramPairingError>;
    pub(crate) async fn unpair(&self, caller: &UserId) -> Result<(), TelegramPairingError>; // unbind + delete dm target + invalidate code
}
```
`consume` on success: bind provider id (`{installation}:{tg_user}`) with epoch = the code string; upsert DM target; mark consumed; then dispatch:
```rust
let event = AuthContinuationEvent {
    flow_id: AuthFlowId::new(), scope: /* AuthProductScope for (tenant, user) mirroring product_auth construction */,
    continuation: AuthContinuationRef::SetupOnly,
    provider: AuthProviderId::new(TELEGRAM_IDENTITY_PROVIDER)?,
    credential_account_id: None, emitted_at: Timestamp::now(),
};
self.continuation.dispatch_auth_continuation(event).await // fan-out resumes BlockedAuth runs with provider "telegram"
```

- [ ] **Step 1: Red tests** (in-crate, fakes/in-memory fs): issue mints 8-char code from alphabet + correct deep link + TTL; second issue rotates (old dead); consume happy binds + records chat_id + marks consumed + dispatches continuation (recording dispatcher fake asserts provider=="telegram", SetupOnly); consume expired/consumed/unknown ⇒ `ExpiredOrUnknown`, no dispatch; consume when tg user bound to OTHER user ⇒ `AlreadyBoundToOtherUser`, original binding intact; same-user re-pair ⇒ `AlreadyPairedSameUser` idempotent; unpair removes binding+target and later resolve fails; case-insensitive code lookup.
- [ ] **Steps 2-4:** red → implement → green + fmt. **Step 5:** commit `feat(reborn): telegram pairing service, identity binding, dm targets`.

---

### Task 4 (superseded): historical activation-gate implementation sketch

> Current correction: installation establishes membership; the generic
> manifest-derived readiness projection parks the run on the pairing
> requirement and derives `active` when pairing completes. The symbols and
> `extension_activate` scripted call below describe the retired intermediate
> design and must not be copied into current code or fixtures.

**Files:**
- Modify: `crates/ironclaw_host_api/src/capability.rs` (~L128-144 `RuntimeCredentialAccountSetup`), `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs` (`activation_credential_requirements` L526-543; connect-strategy fn ~L3143; `channel_connection_requirement` construction ~L2190-2216), `crates/ironclaw_product/src/lifecycle.rs` (`LifecycleExtensionCredentialSetup` — add `Pairing`), `crates/ironclaw_product/src/reborn_services/types.rs` (`RebornExtensionCredentialSetup` — add `Pairing` arm), `crates/ironclaw_reborn_composition/src/extension_host/extension_credential_requirements.rs` (projection arms)
- Create: `crates/ironclaw_reborn_composition/src/telegram/telegram_channel_connection.rs` (slot + facade)
- Test: `crates/ironclaw_webui_v2/tests/webui_v2_handlers_contract.rs` (DTO), composition tests

**Interfaces:**
- Produces: `RuntimeCredentialAccountSetup::Pairing` (serde `pairing`); grep-and-extend every `match` on this enum (compiler drives; known consumers: product_auth setup projection, lifecycle DTO mapping, extensions onboarding derivation) — `Pairing` maps to `RebornExtensionCredentialSetup::Pairing` on the wire.
- Produces: `pub(crate) struct TelegramChannelConnectionSlot(Arc<OnceLock<Arc<dyn ChannelConnectionFacade>>>)` (mirror `SlackPersonalSetupServiceSlot` pattern, slack_setup.rs:717) held by `RebornLocalExtensionManagementPort` as `telegram_channel_connection: Option<TelegramChannelConnectionSlot>`; filled by Task 7 mounts.
- Modifies `activation_credential_requirements` (extension_lifecycle.rs:526): after `package_runtime_credential_auth_requirements`, if the package id is `TELEGRAM_EXTENSION_ID` (cfg-gated arm) and the slot's facade reports `caller_channel_connections[..]["telegram"] == false`, append:
```rust
RuntimeCredentialAuthRequirement {
    provider: RuntimeCredentialAccountProviderId::new("telegram")?,
    setup: RuntimeCredentialAccountSetup::Pairing,
    requester_extension: ExtensionId::new("telegram")?,
    provider_scopes: Vec::new(),
}
```
(The unchanged activate arm then parks `BlockedAuth`; the resumed re-run recomputes and finds it satisfied — self-correcting, per blocked_auth_resume.rs:156-160.)
- Modifies connect-strategy derivation (~L3143) and `channel_connection_requirement` (~L2190): telegram ⇒ `RebornChannelConnectStrategy::WebGeneratedCode` with copy `instructions = "Open the pairing panel to link your Telegram account."`, `submit_label = "Open pairing"`, `input_placeholder = ""` (cfg-gated arm; slack stays OAuth).

- [ ] **Step 1: Red integration test** `tests/integration/telegram_gate.rs` (`[[test]] name = "reborn_integration_telegram_gate"` in workspace Cargo.toml) using the extension-lifecycle group with the telegram package present and an injected unpaired connection facade:
```rust
// scripts: extension_activate tool_call + post-resume text
let h = /* group harness with extension_lifecycle tools + telegram package */;
let (run_id, gate_ref) = h.submit_turn_until_blocked("activate telegram").await?;
// assert run parked BlockedAuth with provider "telegram" in credential_requirements
```
(Exact harness wiring: extend `group_constructors.rs::extension_lifecycle()` with a `telegram_extension_lifecycle()` variant that registers the telegram package + a test `ChannelConnectionFacade` whose pairedness is a shared `Arc<AtomicBool>`.)
- [ ] **Steps 2-4:** red (no gate raised today) → implement enum variant + projections + synthesis + strategy arm → green; run `cargo test -p ironclaw_webui_v2 --test webui_v2_handlers_contract` and fix DTO expectations. **Step 5:** commit `feat(reborn): pairing setup variant + telegram activation gate parks BlockedAuth`.

---

### Task 5: Pairing completion resumes blocked runs (continuation dispatch end-to-end)

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs` (expose the composed `Arc<dyn RebornAuthContinuationDispatcher>` to telegram mounts — it already exists at factory.rs:1269-1297; add an accessor or pass-through in the runtime parts used by Task 7), `tests/integration/telegram_gate.rs` (extend)

**Interfaces:**
- Consumes: `auth_continuation_dispatcher(..)` chain (LifecycleAuthContinuationDispatcher → BlockedAuthResumeFanout → ProductAuthTurnGateResumeDispatcher); `TelegramPairingService.consume` (Task 3).
- Produces: pairing consume in a live composition resumes every `BlockedAuth` run whose `credential_requirements[].provider == "telegram"` for that tenant+user, and leaves other providers' gates parked.

- [ ] **Step 1: Red test extension** in `telegram_gate.rs`: after the Task 4 park, flip the fake facade to paired and drive `TelegramPairingService::consume` (or dispatch the continuation event directly through the group's dispatcher) → `wait_for_status(run_id, TurnStatus::Completed)`; second scenario: two telegram-blocked runs both resume; a github-blocked run stays `BlockedAuth`.
- [ ] **Steps 2-4:** wire the dispatcher handle into the pairing service construction; green. **Step 5:** commit `feat(reborn): pairing completion fans out BlockedAuth resume (provider telegram)`.

---

### Task 6: Webhook ingress — resolver, pairing-aware dispatcher, DM admission, hint throttle

**Files:**
- Create: `crates/ironclaw_reborn_composition/src/telegram/telegram_serve.rs`, `telegram/telegram_dispatch.rs`
- Test: crate tests + `tests/integration/telegram_ingress.rs` (`reborn_integration_telegram_ingress`)

**Interfaces:**
```rust
// telegram_serve.rs (mirror slack_serve.rs L43-275)
pub const TELEGRAM_UPDATES_PATH: &str = "/webhooks/extensions/telegram/updates";
const TELEGRAM_UPDATES_ROUTE_ID: &str = "telegram.updates";
pub trait TelegramInstallationResolver: Send + Sync { fn resolve_ingress<'a>(&'a self, headers: &'a HeaderMap, body: &'a [u8]) -> Pin<Box<dyn Future<Output = Result<ResolvedTelegramIngress, TelegramIngressError>> + Send + 'a>>; fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>; }
pub struct ResolvedTelegramIngress { /* installation: ResolvedTelegramInstallation (evidence + dispatcher handle) */ }
pub struct TelegramEventsRouteState { /* ingress service */ } // impl PublicRouteDrain
pub fn telegram_events_route_mount(state: TelegramEventsRouteState) -> PublicRouteMount; // descriptor projected via bundled_host_ingress_descriptors(telegram_manifest_toml()) + descriptor_for_route("telegram.updates")
// per-installation rate limit mirroring SlackInstallationRateLimiter (120 req/60s token bucket)

// telegram_dispatch.rs — the pairing-aware pre-router in front of NativeProductAdapterRunner
pub(crate) struct TelegramInboundPreRouter { pairing: Arc<TelegramPairingService>, lookup: Arc<dyn RebornUserIdentityLookup>, bot_api: Arc<dyn TelegramBotApi>, setup: Arc<TelegramSetupService>, hint_throttle: Mutex<HashMap<i64, Instant>>, runner: Arc<NativeProductAdapterRunner> }
pub(crate) enum PreRouteOutcome { HandledPairing, HandledHint, HandledSilently, ForwardToWorkflow }
impl TelegramInboundPreRouter {
    // Parses the update minimally (serde into a small TelegramUpdateLite { update_id, message?{ chat{id, r#type}, from?{id, is_bot}, text? } }):
    //  - non-"private" chat / channel_post / edited_message / from.is_bot / no message ⇒ HandledSilently (200, no reply)
    //  - text is "/start <CODE>" or a bare live code (trimmed, uppercased) ⇒ pairing.consume(..) ⇒ send confirmation/refusal via bot_api.send_message ⇒ HandledPairing
    //  - sender unpaired (lookup miss) ⇒ throttled static hint (1 per chat per 10 min) ⇒ HandledHint / HandledSilently when throttled
    //  - paired + ordinary text ⇒ ForwardToWorkflow (runner.process_verified_webhook_immediate_ack)
}
```
Static copy (single-line consts): hint `"This bot is IronClaw. Pair your account from IronClaw → Extensions → Telegram, then message me here."`; confirm `"✅ Paired to {display}. You can talk to IronClaw right here."`; expired `"That code has expired or was already used — get a fresh link from IronClaw."`; already-bound `"This Telegram account is already paired to another IronClaw user."`; already-paired `"You're already paired — just send me a message."`

- [ ] **Step 1: Red crate tests** for the pre-router (fake pairing service/lookup/bot api): each admission row above (group ignored; unpaired hint sent once then throttled; /start CODE consume path sends confirmation; bare-code path; paired forwards; bot sender ignored; non-message update ignored). **Red integration test** `telegram_ingress.rs`: POST without header ⇒ 401-class + no turn; with wrong header ⇒ rejected; with correct header + paired user message ⇒ turn runs (scripted reply) and reply rendered via recorded egress to `api.telegram.org/.../sendMessage`; duplicate `update_id` re-POST ⇒ exactly one turn.
- [ ] **Steps 2-4:** implement resolver (mirror `DynamicSlackInstallationResolver`, runner built with `SharedSecretHeaderAuth { header_name: "X-Telegram-Bot-Api-Secret-Token", expected_secret: <from setup>, subject: installation }` + `TelegramV2Adapter::new(TelegramV2AdapterConfig{ adapter_id: "telegram_v2", installation_id, group_trigger_policy: GroupTriggerPolicy{ bot_username, bot_user_id, recognized_commands: vec![] }, egress_credential_handle: <bot token handle>, auth_requirement, progress_push_enabled: false })`), mount, pre-router; green. **Step 5:** commit `feat(reborn): telegram webhook ingress + pairing-aware DM admission`.

---

### Task 7: Runtime mounts + facades + outbound targets + admin/pairing HTTP routes

**Files:**
- Create: `telegram/telegram_host_beta.rs` (+ `telegram_host_beta/runtime_setup.rs`), `telegram/telegram_channel_routes.rs`, `telegram/telegram_connectable_channel.rs`, `telegram/telegram_outbound_targets.rs`
- Modify: `crates/ironclaw_reborn_composition/src/webui/webui_serve.rs` (add `with_telegram_channel_routes` mirroring `with_slack_channel_routes` L404 — or reuse `with_protected_route_mount` if the generic mount carries state cleanly), composite facades in `crates/ironclaw_reborn_composition/src/webui/` (new `composite_channels.rs`: `CompositeConnectableChannelsFacade(Vec<Arc<dyn ConnectableChannelsProductFacade>>)` concatenating lists; `CompositeChannelConnectionFacade(Vec<Arc<dyn ChannelConnectionFacade>>)` merging maps / routing disconnect by channel key)

**Interfaces:**
```rust
pub struct TelegramHostRuntimeConfig { pub tenant_id: TenantId, pub agent_id: AgentId, pub project_id: Option<ProjectId>, pub operator_user_id: UserId } // ::new(..)
pub struct TelegramHostMounts { pub events: PublicRouteMount, pub channel_routes: TelegramChannelRouteAdminRouteConfig, /* pub(crate) facade handles, pairing service, outbound provider */ }
pub async fn build_telegram_host_runtime_mounts(runtime: &RebornRuntime, config: TelegramHostRuntimeConfig) -> Result<TelegramHostMounts, TelegramHostBuildError>;
// mirrors slack build_runtime_mounts L65-227: one FilesystemTelegramHostState fanned into Arc<dyn ...> handles;
// TelegramSetupService; DynamicTelegramInstallationResolver; DynamicTelegramChannelSetupActivation (activate "telegram" package, Discovered⇒no-op);
// TelegramOutboundTargetProvider registered via runtime.register_outbound_delivery_target_provider("telegram", ..) (targets "telegram:dm:{installation}:{user}" from TelegramDmTargetStore);
// fills the Task 4 TelegramChannelConnectionSlot with the telegram ChannelConnectionFacade

// telegram_channel_routes.rs — protected router:
//   GET/PUT  /api/webchat/v2/channels/telegram/setup      (operator-gated: ensure_authorized_operator semantics — tenant mismatch⇒404, non-operator⇒403; fields through scan_route_admin_field-equivalent safety scan; PUT = save_with_previous → activate → rollback-on-failure → redacted status)
//   DELETE   /api/webchat/v2/channels/telegram/setup      (operator: clear())
//   POST     /api/webchat/v2/channels/telegram/pairing    (ANY authenticated member: issue_or_rotate → {code, deep_link, expires_at})
//   GET      /api/webchat/v2/channels/telegram/pairing    (member: status_for → {connected, pending})
//   DELETE   /api/webchat/v2/channels/telegram/pairing    (member: unpair)
pub struct TelegramSetupSaveRequest { pub bot_token: Option<String>, pub webhook_url: Option<String> } // deny_unknown_fields
```
Connectable channels: operator gets `{channel:"telegram", strategy: AdminManagedChannels, action: {title:"Telegram bot setup", ...}}`; every member gets `{channel:"telegram", strategy: WebGeneratedCode, action: {title:"Pair Telegram", instructions:"Open Telegram via the link or QR, or send the code to the bot.", input_placeholder:"", submit_label:"Open pairing", success_message:"Telegram paired.", error_message:"Pairing failed — get a fresh code."}}` (only when setup configured; unconfigured ⇒ member entry omitted, card shows admin-required copy from the extension info path).

- [ ] **Step 1: Red tests**: axum-level route tests (mirror slack_channel_routes tests): operator authz matrix on setup routes; pairing POST as member mints code; GET reflects pending/connected; PUT setup with recording bot api → activates package (fake activation) and rolls back on activation error; secrets absent from all responses. Composite facade unit tests: two facades' channels concatenate; disconnect routes to the right one.
- [ ] **Steps 2-4:** implement; green. **Step 5:** commit `feat(reborn): telegram runtime mounts, admin+pairing routes, composite channel facades`.

---

### Task 8: Serve wiring + config section + trigger-delivery composite

**Files:**
- Create: `crates/ironclaw_reborn_cli/src/commands/serve_telegram.rs`
- Modify: `crates/ironclaw_reborn_cli/src/commands/serve.rs` (mirror slack blocks at L22-25/L221/L472-506/L612-619), `crates/ironclaw_reborn_config/src/config_file.rs` (add `TelegramSection { enabled: Option<bool> }` with `deny_unknown_fields`, next to `SlackSection` L344), `crates/ironclaw_reborn_composition/src/slack/slack_delivery.rs` consumers if a composite `PostSubmitDeliveryHook` is needed (`CompositePostSubmitDeliveryHook(Vec<Arc<dyn PostSubmitDeliveryHook>>)` in a shared module — `set_trigger_post_submit_hook` is a single OnceLock slot; when both slack+telegram are enabled serve installs the composite once)

**Interfaces:**
- Produces: `resolve_telegram_config_for_serve(section, tenant_id, agent_id, project_id, user_id) -> anyhow::Result<Option<TelegramHostRuntimeConfig>>` gated on env `IRONCLAW_REBORN_TELEGRAM_ENABLED` override else `section.enabled`; `#[cfg(not(feature))]` stub errors when enabled without the feature (mirror serve_slack.rs L90).
- Serve: `build_telegram_host_runtime_mounts` → `serve_config.with_public_route_mount(telegram_mounts.events).with_telegram_channel_routes(telegram_mounts.channel_routes)`; connectable/connection facades are exposed through the runtime-backed product surface.

- [ ] **Step 1: Red test**: config-file test for `TelegramSection` parse + env override precedence (mirror existing SlackSection tests); a compile-features matrix check: `cargo check -p ironclaw_reborn_cli` (no features), `--features telegram-v2-host-beta`, `--features slack-v2-host-beta,telegram-v2-host-beta`.
- [ ] **Steps 2-4:** implement; all three feature combos compile; green. **Step 5:** commit `feat(reborn): serve wiring + config for telegram host`.

---

### Task 9: Removal/lifecycle semantics + restart survival (integration)

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_removal_cleanup.rs` (register telegram cleanup requirement: user remove ⇒ `TelegramPairingService::unpair` for the caller), `available_extensions.rs` (`cleanup_requirements` for telegram mirroring the slack entry at L620-632)
- Test: `tests/integration/telegram_lifecycle.rs` (`reborn_integration_telegram_lifecycle`)

- [ ] **Step 1: Red integration tests**: (a) user remove unpairs only the caller (other user's binding + DM target intact; history files intact); (b) remove during pending pairing invalidates the code (consume afterwards ⇒ ExpiredOrUnknown); (c) restart survival with `.storage(StorageMode::LibSql)`: setup + binding + pending BlockedAuth run all read back through freshly reopened stores, then pairing consume still resumes the run; (d) admin `clear()` ⇒ resolver rejects next webhook (fail-closed), bindings retained.
- [ ] **Steps 2-4:** implement cleanup wiring; green. **Step 5:** commit `feat(reborn): telegram removal semantics + restart survival coverage`.

---

### Task 10: Arch gates — naming + no-legacy + boundaries

**Files:**
- Create: `crates/ironclaw_architecture/tests/telegram_extension_gates.rs`
- Modify: `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` if the new module adds edges

- [ ] **Step 1: Red gates**:
```rust
#[test] fn no_retired_taxonomy_telegram_identifiers() { /* scan crates/ + webui frontend src for "telegram_bot", "telegram_personal", "telegram_channel" as identifiers; allow zero */ }
#[test] fn telegram_strings_confined() { /* "telegram" (case-insens) in crates/** limited to: composition/src/telegram/**, extension_host/{available_extensions,extension_lifecycle,extension_removal_cleanup}.rs cfg-gated arms, ironclaw_telegram_v2_adapter, reborn_cli serve wiring, first_party assets, tests, doc comments — mirror reborn_extension_specificity boundaries */ }
#[test] fn reborn_context_free_of_v1_pairing_routes() { /* no "/api/pairing/" literals in crates/** or webui frontend src */ }
```
- [ ] **Steps 2-4:** tune allowlists to the real tree; green (`cargo test -p ironclaw_architecture`). **Step 5:** commit `test(arch): telegram naming + no-legacy gates`.

---

### Task 11: Frontend — setup panel, pairing panel (code+link+QR+renewal+poll), chat gate routing

**Files:**
- Create: `crates/ironclaw_webui_v2/frontend/src/lib/telegram-setup-api.ts`, `src/components/telegram-setup-panel.tsx`, `src/components/telegram-pairing-panel.tsx` (+ `.test.ts` files beside existing patterns)
- Modify: `frontend/package.json` (+`qrcode` dep + `@types/qrcode` dev), `src/pages/extensions/components/channels-tab.tsx`, `src/pages/chat/components/onboarding-pairing-card.tsx` (strategy `web_generated_code` → render the generic host-generated code/deep-link/QR panel), `src/pages/chat/hooks/useChannelOnboarding.ts` (treat a `BlockedAuth` gate whose requirement setup is `pairing` as the pairing card trigger), i18n keys

**Interfaces:**
- `telegram-setup-api.ts`: `getTelegramSetup() GET /api/webchat/v2/channels/telegram/setup`, `saveTelegramSetup({bot_token?, webhook_url?}) PUT`, `clearTelegramSetup() DELETE`, `startTelegramPairing() POST /api/webchat/v2/channels/telegram/pairing -> {code, deep_link, expires_at}`, `getTelegramPairing() GET -> {connected, pending}`, `disconnectTelegramPairing() DELETE`.
- `TelegramPairingPanel`: on mount `startTelegramPairing`; renders code (copyable), deep link button, QR (`qrcode.toDataURL(deep_link)` into `<img>`), `@username` copy-text, countdown from `expires_at`; expired state → "Get a new code" → re-start; polls `getTelegramPairing` every 2000 ms (the `CHAT_OAUTH_POLL_MS` convention) → on `connected` fires `notifyChannelConnected("telegram")` + react-query invalidation (`["extensions"]`, `["connectable-channels"]`).

- [ ] **Step 1: Red vitest tests**: panel renders code/link/QR and re-renders QR after renewal; countdown flips to expired state exposing renewal; poll flip to connected invokes the connection bus; setup panel never echoes a saved token (placeholder-only) and blank token means keep-existing; channels-tab branches per strategy.
- [ ] **Steps 2-4:** implement; `npm test` (or the repo's vitest invocation) green; `npm run build` green. **Step 5:** commit `feat(webui-v2): telegram setup panel + WebGeneratedCode pairing panel`.

---

### Task 12: Reborn-scoped legacy purge

**Files:**
- Rewrite: `docs/reborn/contracts/telegram-v2.md` (new contract: single extension, admin setup pipeline, pairing state machine, DM admission table, honest-delivery mapping; names its test bins + run commands), `tests/telegram_v2_default_off_integration.rs` → new-model gating test (feature posture + `REBORN_TELEGRAM_V2_ENABLED` v1-exclusivity arbitration comment refresh)
- Modify: telegram legs in `tests/reborn_qa_connect_flows.rs`, `tests/staging_regression_fixes.rs`, `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs`, `crates/ironclaw_webui_v2/tests/webui_v2_handlers_contract.rs` (WebGeneratedCode payloads), `scripts/reborn_webui_v2_live_qa/case_matrix.py` telegram cases, `scripts/ci/reborn-e2e-rust.sh` (add the telegram contract test mapping); remove dormant pasted-proof redeem plumbing.
- Do NOT touch: `channels-src/telegram/`, `tools-src/telegram/`, v1 tests, `src/**` (v1 monolith stays working; `validate_telegram_v1_v2_exclusivity` comments updated only if strictly needed)

- [ ] Steps: inventory each file's telegram leg → rewrite to new model → run each touched suite → commit `refactor(reborn): retire legacy-shaped telegram from the reborn context`.

---

### Task 13: Quality gate + PR

- [ ] `cargo fmt` all; `cargo clippy --workspace --all-targets --all-features -- -D warnings` (fix everything)
- [ ] `cargo test -p ironclaw_reborn_composition --features telegram-v2-host-beta` + all new `--test reborn_integration_telegram_*` bins + `cargo test -p ironclaw_architecture` + touched contract tests
- [ ] `bash scripts/pre-commit-safety.sh`; frontend build + vitest
- [ ] Push branch to origin as `feat/telegram-extension`; open PR to `main`: body = feature summary, the five owner decisions, seam notes (Pairing variant, synthesized gate, composite facades, OnceLock delivery-hook composite), test inventory (manual-QA plan cross-reference), explicit cut list if any, `🤖 Generated with [Claude Code](https://claude.com/claude-code)`
- [ ] Watch CI (Code Style + Tests(Reborn) aggregates BY NAME); fix red until green.

## Self-review (spec coverage)

Spec §1 naming → Global Constraints + Task 1/6; §2 admin setup → Tasks 2/7; §3 ingress/messaging → Task 6 (+adapter reuse; chunking is render-side — adapter sends single message; 4096 chunking asserted in Task 6 egress test via multiple sendMessage calls if view exceeds limit — NOTE: current `render_final_reply` sends one message; add chunk loop in `telegram_dispatch` outbound path if the adapter doesn't split; verify during Task 6 and extend `render.rs` only if required); §4 pairing → Tasks 3/5/7/11; §5 lifecycle → Tasks 4/9; §6 security → Tasks 2/6/7/10; §7 porting → constraints + Task 10 gates; §8 legacy → Task 12; §9 flag/config → Tasks 1/8; §10 testing → embedded per task; §11 map → tasks 1:1; §12 verifications → all nine resolved (see spec appendix + verification reports).
