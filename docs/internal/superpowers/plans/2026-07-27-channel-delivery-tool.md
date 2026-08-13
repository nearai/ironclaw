# Channel Delivery Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `builtin.outbound_deliver` (explicit, bot-identity, evidence-bearing channel delivery callable mid-run), replace automation delivery heuristics with tool calls + a notification-channel set, and delete the superseded routing machinery — per the approved spec `docs/superpowers/specs/2026-07-27-channel-delivery-tool-design.md`.

**Architecture:** Two lanes. Lane 1: final replies always land in the run's own conversation (existing `RunDeliveryObserver` / WebUI thread / per-fire trigger thread — untouched). Lane 2: a first-party capability handler (`ironclaw_host_runtime`) holds an `Arc<dyn ModelChannelDelivery>` port (defined in `ironclaw_outbound`), implemented in `ironclaw_extension_host` over the existing `DeliveryCoordinator` with a new policy-class intent, wired by composition. Background-run notifications fan out over a stored set of catalog target ids.

**Tech Stack:** Rust workspace (`crates/`), React/TS WebUI (`crates/ironclaw_webui/frontend`), Reborn integration harness (`tests/integration/`), Python/Playwright e2e (`tests/e2e/`).

## Global Constraints

- All spec section references (§N) are to `docs/superpowers/specs/2026-07-27-channel-delivery-tool-design.md`. Read it before starting any task.
- Anchors were verified on `main` `aa8b748c3` (2026-07-27). **Read every named file before editing** — line numbers drift.
- TDD per task: failing test → minimal implementation → green → commit. Every behavior change lands with its test in the same commit.
- Zero clippy warnings, both lanes: `cargo clippy --all --tests --examples -- -D warnings` AND `... --all-features -- -D warnings`.
- No `.unwrap()`/`.expect()` in production code; no `unwrap_or_default()`/`.ok()?` on fallible boundary calls without `// silent-ok:` (`.claude/rules/error-handling.md`).
- Newtypes per `.claude/rules/types.md`; shared types live with their contract owner (`.claude/rules/type-placement.md`).
- Prompt text lives in `.md` assets loaded via `include_str!`, never Rust string constants (repo rule) — single-line tool descriptions are fine inline.
- Architecture gates run after every task that touches ids, deps, or ratchets: `cargo test -p ironclaw_architecture`.
- Deep integration binaries need `RUST_MIN_STACK=16777216`; Docker-backed legs need `DOCKER_HOST=unix:///$HOME/.colima/default/docker.sock`.
- The frozen builtin-id list (`crates/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`, exact-ordered `assert_eq!`) must be updated in the SAME commit as any tool add/remove.
- Commit messages: conventional (`feat:`/`test:`/`refactor:`/`docs:`), each ending with the Claude Co-Authored-By trailer.
- PR train: Tasks 1–10 → PR 1 (tool + notifications, additive); Tasks 11–13 → PR 2 (deletions + migrations); Tasks 14–18 → PR 3 (guidance + docs + fixtures). Each PR green on the full gate.
- **Spec deviation note (approved shape):** the spec's "FailureNotice fans out" is implemented as a new policy-class intent `DeliveryIntent::BackgroundRunNotice` (the existing `FailureNotice` stays notice-class for source-routed interactive failures). Everything else follows the spec literally.

## File Structure (created / heavily modified)

```
crates/ironclaw_outbound/src/
  model_channel_delivery.rs        # NEW: port + request/evidence/error types (Task 2)
  communication_preferences.rs     # notification_targets field + migration helper (Task 7)
  delivery_coordinator (consumer)  # intent enum lives in ironclaw_product (Task 1)
crates/ironclaw_extension_host/src/
  model_channel_delivery.rs        # NEW: production impl over DeliveryCoordinator (Task 3)
  channel_triggered_delivery.rs    # routing block deleted; notifier wiring only (Tasks 9, 13)
crates/ironclaw_host_runtime/src/first_party_tools/
  outbound_deliver.rs              # NEW: the tool (Task 4)
  outbound_delivery.rs             # DELETED (Task 11)
crates/ironclaw_product/src/
  delivery_coordinator.rs          # +ModelDelivery, +BackgroundRunNotice, −TriggeredDelivery
  run_delivery/triggered.rs        # background-run notifier rework (Task 9)
  reborn_services/outbound_preferences.rs        # list-valued notification channels (Task 8)
  reborn_services/outbound_delivery_capability_surface.rs  # notification_channels_set (Task 8)
crates/ironclaw_turns/prompts/delivery.md        # NEW: the one guidance block (Task 14)
crates/ironclaw_webui/frontend/src/pages/automations/
  components/notification-channels-panel.tsx     # RENAMED from automation-delivery-defaults-panel.tsx (Task 10)
tests/integration/delivery_user_journeys.rs      # rebuilt from retired stub (Tasks 5,6,9,12)
docs/internal/reborn/extension-runtime/{overview,checklist}.md      # §5.4 rewrite (Task 17)
.claude/rules/{tools,tool-evidence}.md           # ported from pushy-today (Task 17)
```

---

### Task 1: `DeliveryIntent::ModelDelivery` + push-kind plumbing

**Files:**
- Modify: `crates/ironclaw_product/src/delivery_coordinator.rs` (intent enum, `runs_outbound_policy`, `as_str`)
- Modify: `crates/ironclaw_outbound/src/types.rs` (`OutboundPushKind`), `crates/ironclaw_outbound/src/delivery_resolution.rs` (`CommunicationDeliveryKind`, kind mapping)
- Test: `crates/ironclaw_product/tests/outbound_delivery_contract.rs`

**Interfaces:**
- Produces: `DeliveryIntent::ModelDelivery` (policy-class, `as_str() == "model-delivery"`); `OutboundPushKind::ModelDelivery`; `CommunicationDeliveryKind::ModelDelivery`. `DeliveryIntent::TriggeredDelivery` is NOT removed here (Task 9 removes it with its last consumer).
- Consumes: existing enums at the named files.

- [ ] **Step 1: Read the anchors.** `delivery_coordinator.rs` (whole file), the `CommunicationDeliveryKind` and `OutboundPushKind` definitions (`rg -n "enum CommunicationDeliveryKind|enum OutboundPushKind" crates/ironclaw_outbound/src/`), and every exhaustive `match` on them (`rg -n "CommunicationDeliveryKind::|OutboundPushKind::" crates/ | grep -v tests`). List the matches you must extend.
- [ ] **Step 2: Write the failing test** in `outbound_delivery_contract.rs`, alongside `coordinator_deliver_rejects_notice_class_intents`:

```rust
#[test]
fn model_delivery_is_policy_class() {
    use ironclaw_product::DeliveryIntent;
    assert!(DeliveryIntent::ModelDelivery.runs_outbound_policy());
    assert!(!DeliveryIntent::ModelDelivery.is_notice_class());
}
```

Also extend the existing async test `coordinator_notice_rejects_policy_class_intents` list with `DeliveryIntent::ModelDelivery`.
- [ ] **Step 3:** `cargo test -p ironclaw_product --test outbound_delivery_contract model_delivery_is_policy_class` → FAIL (no variant).
- [ ] **Step 4: Implement.** Add `ModelDelivery` to `DeliveryIntent` (doc: `/// A model-initiated explicit delivery via builtin.outbound_deliver.`), include it in `runs_outbound_policy()`'s `matches!`, add `Self::ModelDelivery => "model-delivery"` to `as_str`. Add `ModelDelivery` variants to `OutboundPushKind` and `CommunicationDeliveryKind` (both `#[serde(rename_all = "snake_case")]` wire enums — additive, no aliases needed), and extend the `CommunicationDeliveryKind → OutboundPushKind` mapping plus `resolve_triggered_explicit_target`'s match arm (`ModelDelivery` behaves like `FinalReply`: pass-through `ordinary_notification_target`). Fix every exhaustive match the compiler flags.
- [ ] **Step 5:** `cargo test -p ironclaw_product --test outbound_delivery_contract && cargo test -p ironclaw_outbound` → PASS. `cargo clippy -p ironclaw_product -p ironclaw_outbound --all-targets --all-features -- -D warnings`.
- [ ] **Step 6: Commit** `feat(outbound): add model-delivery intent and delivery kinds`

### Task 2: `ModelChannelDelivery` port in `ironclaw_outbound`

**Files:**
- Create: `crates/ironclaw_outbound/src/model_channel_delivery.rs`
- Modify: `crates/ironclaw_outbound/src/lib.rs` (module + re-exports beside `RouteCurrentRunFinalReply`)
- Test: unit tests inline in the new file

**Interfaces:**
- Produces (exact, consumed by Tasks 3–5):

```rust
use async_trait::async_trait;
use ironclaw_host_api::{ResourceScope, RunId, UserId};

use crate::{DeliveryFailureKind, OutboundDeliveryTargetId, OutboundDeliveryTargetSummary};

/// Per-run ceiling on explicit model deliveries (spec §5).
pub const MODEL_DELIVERY_PER_RUN_CAP: u32 = 16;
/// Input content ceiling, matched by the tool input schema (spec §5).
pub const MODEL_DELIVERY_MAX_CONTENT_BYTES: usize = 32_768;

/// Trusted request from the first-party capability lane (mirrors
/// `RouteCurrentRunFinalReplyRequest`'s shape and trust posture).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChannelDeliveryRequest {
    pub scope: ResourceScope,
    pub run_id: RunId,
    pub authenticated_actor_user_id: UserId,
    pub target_id: OutboundDeliveryTargetId,
    pub content: String,
}
// NOTE (ruled 2026-07-27): no separate invocation field — `scope` is a
// `ResourceScope`, which already carries the typed `invocation_id:
// InvocationId` for this tool call; Task 3 derives the attempt's
// projection ref from it (`model-delivery:{run_id}:{scope.invocation_id}`).

/// Provider-issued evidence for one delivered call (spec §5: never claim
/// what the vendor did not confirm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChannelDeliveryEvidence {
    pub target: OutboundDeliveryTargetSummary,
    pub provider_message_refs: Vec<String>,
}

/// Redacted failure classes crossing the capability-to-product port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ModelChannelDeliveryError {
    #[error("outbound delivery target is unavailable")]
    TargetUnavailable,
    #[error("target is this run's own origin conversation")]
    OriginConversationTarget,
    #[error("per-run delivery cap reached")]
    DeliveryCapExceeded,
    #[error("delivery content exceeds the size bound")]
    ContentTooLarge,
    #[error("outbound policy rejected the delivery")]
    Rejected,
    #[error("delivery failed")]
    Failed { kind: DeliveryFailureKind },
    #[error("delivery is not permitted")]
    AccessDenied,
    #[error("delivery service is unavailable")]
    Unavailable,
    #[error("delivery failed internally")]
    Internal,
}

#[async_trait]
pub trait ModelChannelDelivery: Send + Sync {
    async fn deliver_for_model(
        &self,
        request: ModelChannelDeliveryRequest,
    ) -> Result<ModelChannelDeliveryEvidence, ModelChannelDeliveryError>;
}
```

- [ ] **Step 1: Write failing unit tests** (inline `#[cfg(test)]`): `content_bound_constant_matches_schema_bound` (`assert_eq!(MODEL_DELIVERY_MAX_CONTENT_BYTES, 32_768)`) and a `Failed { kind }` Display smoke test. (The port is types-only; tests pin the constants the schema task reuses.)
- [ ] **Step 2:** `cargo test -p ironclaw_outbound model_channel_delivery` → FAIL (module missing).
- [ ] **Step 3:** Create the file with the exact block above, wire `pub mod model_channel_delivery;` + re-export the six names from `lib.rs` beside the `RouteCurrentRunFinalReply` exports.
- [ ] **Step 4:** `cargo test -p ironclaw_outbound && cargo clippy -p ironclaw_outbound --all-targets --all-features -- -D warnings` → PASS. Then `cargo test -p ironclaw_architecture` (new public types; struct-ratchet may need its recorded update — follow the test's own failure instructions if it fires; the port trait is justified as dependency inversion, same as `RouteCurrentRunFinalReply` — state that in the commit body).
- [ ] **Step 5: Commit** `feat(outbound): ModelChannelDelivery port for the delivery tool`

### Task 3: Production implementation in `ironclaw_extension_host`

**Files:**
- Create: `crates/ironclaw_extension_host/src/model_channel_delivery.rs`
- Modify: `crates/ironclaw_extension_host/src/lib.rs` (module + export)
- Test: inline `#[cfg(test)]` with fakes (this crate already fakes registry/coordinator seams in `channel_triggered_delivery.rs` tests — mirror those)

**Interfaces:**
- Consumes: Task 2 port; `OutboundDeliveryTargetRegistry::resolve_outbound_delivery_target` (`crates/ironclaw_outbound/src/delivery_targets.rs`); `DeliveryCoordinator::deliver` + `CoordinatedDeliveryRequest`/`Outcome` (`crates/ironclaw_product/src/delivery_coordinator.rs`); `RunNotificationOrigin::RunScopedTarget` + `CommunicationDeliveryKind::ModelDelivery` (Task 1); `TurnCoordinator::get_run_state` for the origin check.
- Produces: `pub struct ExtensionHostModelChannelDelivery` with

```rust
impl ExtensionHostModelChannelDelivery {
    pub fn new(deps: ModelChannelDeliveryDeps) -> Self { /* … */ }
}
pub struct ModelChannelDeliveryDeps {
    pub registry: Arc<OutboundDeliveryTargetRegistry>,
    pub coordinator: Arc<DeliveryCoordinator>,
    pub outbound_store: Arc<dyn OutboundStateStorePort>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub target_resolver: Arc<dyn ProductOutboundTargetResolver>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    // …plus exactly the validator/store handles `RunDeliveryServices`
    // (crates/ironclaw_product/src/run_delivery.rs) threads into
    // `OutboundPolicyService` for its FinalReply deliveries — copy that
    // field list verbatim in Step 1; do not invent a different shape.
}
```

`impl ModelChannelDelivery for ExtensionHostModelChannelDelivery`. Behavior contract (each bullet = one test):
  1. `content.len() > MODEL_DELIVERY_MAX_CONTENT_BYTES` → `ContentTooLarge` (byte length; no slicing).
  2. Unknown/foreign target (registry returns `None` — registry already owner-filters) → `TargetUnavailable`.
  3. Same-origin check: build `TurnScope` from `scope` + run lookup exactly the way `factory.rs`'s retired `resolve_current_run_delivery_target` did, call `get_run_state`, compare `run_state.reply_target_binding_ref` (when `Some`) against the resolved entry's `RunFinalReplyDestination::External { reply_target_binding_ref }` → equal ⇒ `OriginConversationTarget`. A run-state read error → `Unavailable` (fail closed, log cause at debug).
  4. Per-run cap: `Mutex<(VecDeque<RunId>, HashMap<RunId, u32>)>` FIFO-bounded to 256 runs (copy the `HintSeenSet` pattern from `crates/ironclaw_product/src/run_delivery.rs`); count ≥ `MODEL_DELIVERY_PER_RUN_CAP` ⇒ `DeliveryCapExceeded`; increment only after passing checks.
  5. Happy path: build `PrepareCommunicationDeliveryRequest` with a `RunNotification` resolution request whose origin is `RunNotificationOrigin::RunScopedTarget { target: <entry binding ref> }` and kind `ModelDelivery`, `projection_ref: ProjectionUpdateRef::new(format!("model-delivery:{run_id}:{invocation_id}", invocation_id = request.scope.invocation_id))` — the invocation identity comes from the request's `ResourceScope`, which already carries the typed `invocation_id` for this tool call (ruled 2026-07-27; no separate string field exists). Drive `coordinator.deliver(...)` mirroring the observer's construction of `OutboundPolicyService` and parts (`vec![OutboundPart::Text(request.content)]`, `thread_anchor: None`, `require_direct_message_target: false`, `intent: DeliveryIntent::ModelDelivery`, `extension_id` = the entry's channel extension id).
  6. Outcome mapping: `Delivered { vendor_message_refs, .. }` → `Ok(evidence)`; `Rejected` → `Err(Rejected)`; `Failed { failure_kind }` → `Err(Failed{kind})`; `NoDelivery` → `Err(Internal)` (cannot happen with an explicit candidate — log loud); coordinator `ChannelUnavailable` error → `Err(Failed { kind: TransportUnavailable })`; other coordinator errors → `Err(Internal)` with cause logged.

- [ ] **Step 1: Read** `channel_triggered_delivery.rs` (fake patterns + how it obtains `OutboundPolicyService`), `run_delivery/observer.rs`'s FinalReply deliver call (the exact `OutboundPolicyService` construction to mirror), and `delivery_targets.rs` registry API.
- [ ] **Step 2: Write failing tests** for contracts 1–6 above using the crate's existing fake registry/coordinator seams (name them `deliver_for_model_rejects_oversized_content`, `deliver_for_model_rejects_unknown_target`, `deliver_for_model_denies_origin_conversation_target`, `deliver_for_model_enforces_per_run_cap`, `deliver_for_model_returns_provider_evidence`, `deliver_for_model_maps_terminal_failure_kinds`).
- [ ] **Step 3:** `cargo test -p ironclaw_extension_host model_channel_delivery` → FAIL.
- [ ] **Step 4:** Implement per the contract. No `unwrap`; every silent fallback carries `// silent-ok:`.
- [ ] **Step 5:** Tests + clippy green; `cargo test -p ironclaw_architecture` (new dependency edges extension_host→outbound/product already exist; the boundary test must stay green — if it flags the edge, STOP and re-read `telegram_extension_gates.rs`: this crate is the pinned home).
- [ ] **Step 6: Commit** `feat(extension-host): model channel delivery over the coordinator`

### Task 4: The tool — `builtin.outbound_deliver`

**Files:**
- Create: `crates/ironclaw_host_runtime/src/first_party_tools/outbound_deliver.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs` (module decl, manifest list, registry insert + register fn), `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs`, `crates/ironclaw_host_runtime/src/lib.rs` (export register fn)
- Test: `crates/ironclaw_host_runtime/tests/first_party_builtin_tools.rs` (frozen list → 32 ids this task), `crates/ironclaw_host_runtime/tests/tool_surface_contract.rs`

**Interfaces:**
- Consumes: Task 2 port (`Arc<dyn ModelChannelDelivery>`), `first_party_capability_manifest(...)` + `resource_profile()` (mod.rs), `FirstPartyCapabilityRequest/Result/Error` (`first_party.rs`).
- Produces: `pub const OUTBOUND_DELIVER_CAPABILITY_ID: &str = "builtin.outbound_deliver";`, `pub fn register_outbound_deliver_first_party_handler(registry: &mut FirstPartyCapabilityRegistry, delivery: Arc<dyn ModelChannelDelivery>) -> Result<(), HostApiError>` (mirrors the route_current registration pair, including a fail-closed `UnavailableModelChannelDelivery` default returning `Unavailable`).
- Tool description (verbatim, single-line const `DESCRIPTION`):

```text
Deliver content to one connected channel destination from the assistant (bot) identity. Use this when the user wants content on another surface ("send me X on slack", a routine prompt's delivery step, a registered shared channel). target_id must be an exact id from builtin__outbound_delivery_targets_list; call once per destination. The result carries provider message references as delivery evidence — report outcomes honestly from it. Your final reply still lands in this conversation automatically: never deliver to the conversation you are replying in, and never use an integration send-message tool (which acts as the user) to deliver your own output.
```

- Input schema (schemas.rs, beside the route_current schema): object, `additionalProperties: false`, required `["target_id", "content"]`; `target_id`: string 1..512, description "Opaque target id returned by builtin__outbound_delivery_targets_list."; `content`: string 1..32768, description "Markdown content to deliver. The channel renders and splits it.". Output schema: object `{delivered: {const: true}, target_id: string, channel: string, display_name: string, provider_message_refs: {type: array, items: string}}`.
- Handler mapping (each = a test): parse via `#[serde(deny_unknown_fields)]` struct; missing `run_id` → `OperationFailed` safe summary "explicit channel delivery requires an active run"; missing actor → `PolicyDenied`; port errors → `TargetUnavailable`→ input issue on `target_id` (copy `invalid_target_input` shape); `OriginConversationTarget` → `FirstPartyCapabilityError::with_safe_summary(RuntimeDispatchErrorKind::PolicyDenied, "target is this conversation - your reply already lands here")`; `DeliveryCapExceeded`/`ContentTooLarge`/`Rejected`/`Failed{..}` → `OperationFailed` with kind-named safe summaries (no model-controlled interpolation — `LoopSafeSummary` forbids `/<>[]{}`); `AccessDenied` → `PolicyDenied`; `Unavailable|Internal` → `Backend`. Success → `FirstPartyCapabilityResult::new(json!({...}), ResourceUsage::default())` plus `display_preview` with `output_summary: format!("Delivered to {}", display_name)` via the existing `CapabilityDisplayOutputPreview` field.

- [ ] **Step 1: Write failing tests**: in `first_party_builtin_tools.rs` add `outbound_deliver` to the exact-ordered id list (32 entries this task; keep alphabetical position consistent with the list's existing ordering convention), to the `PermissionMode::Allow` set, and to the `GatedUnlessGranted` grouping assertion at the route_current cluster; in `tool_surface_contract.rs` add description-substring assertions for "builtin__outbound_delivery_targets_list", "provider message references", and "never deliver to the conversation you are replying in".
- [ ] **Step 2:** Run both test files → FAIL.
- [ ] **Step 3:** Implement `outbound_deliver.rs` (manifest via `first_party_capability_manifest(OUTBOUND_DELIVER_CAPABILITY_ID, DESCRIPTION, vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite], PermissionMode::Allow, resource_profile())` — the seed matrix already yields `loop_run: GatedUnlessGranted` for a non-allowlisted id), schemas, handler, registration + fail-closed default in `mod.rs`, `lib.rs` export.
- [ ] **Step 4:** `cargo test -p ironclaw_host_runtime` + clippy + `cargo test -p ironclaw_architecture` (origin-gate ratchet: new id must NOT be in the 17-seed; matrix well-formedness holds automatically) → PASS.
- [ ] **Step 5: Commit** `feat(host-runtime): builtin.outbound_deliver capability`

### Task 5: Composition wiring + first journey (WebUI → Slack, evidence at both seams)

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs` (build `ExtensionHostModelChannelDelivery` from the same deps that build the triggered-delivery hook/coordinator; call `register_outbound_deliver_first_party_handler` where `production_first_party_registry_with_trigger_create_hook` assembles the registry), `tests/integration/support/harness/profiles/outbound.rs` + `extension.rs` (register the new capability in harness profiles, beside the existing ROUTE_CURRENT registrations)
- Test: `tests/integration/delivery_user_journeys.rs` (replace the retired stub — new suite header documenting the two assertion seams, journey 1), Cargo target already registered (`reborn_integration_delivery_user_journeys`)

**Interfaces:**
- Consumes: Tasks 3–4 constructors/register fn; harness: `RebornIntegrationHarness`, `RebornScriptedReply`, `assert_delivered_attempt` pattern from `tests/integration/extension_delivery.rs`.
- Produces: journey test `webui_send_me_on_slack_delivers_via_bot_with_evidence` — scripted trace calls `builtin.outbound_deliver` with the Slack DM target id, asserts: (a) wire recorder saw `chat.postMessage` under the **bot** credential handle; (b) outbound store has a terminal `Delivered` attempt with kind `ModelDelivery`; (c) the tool RESULT payload in the run timeline contains `provider_message_refs` matching the recorded vendor response; (d) the final assistant message is an ack, rendered in the WebUI thread only (no second external attempt).

- [ ] **Step 1: Read** the retired stub, `extension_delivery.rs`'s `slack_final_reply_flows_through_the_real_delivery_coordinator` (harness setup + both seams), and `git show pushy-today:tests/integration/delivery_user_journeys.rs` for journey structure conventions (pattern donor only — its receipt/guard content is superseded).
- [ ] **Step 2:** Write the failing journey (harness profile without the new registration → tool unknown) → run `RUST_MIN_STACK=16777216 cargo test --test reborn_integration_delivery_user_journeys` → FAIL for the right reason.
- [ ] **Step 3:** Wire composition + harness profiles. Re-run → PASS.
- [ ] **Step 4:** `cargo test -p ironclaw_reborn_composition` + clippy + architecture gate → PASS.
- [ ] **Step 5: Commit** `feat(composition): wire outbound_deliver + first delivery journey`

### Task 6: Journeys 2–4 (cross-channel ack, same-origin deny, partial failure)

**Files:**
- Test: `tests/integration/delivery_user_journeys.rs`

**Interfaces:** consumes Task 5's suite scaffolding. Produces three journeys (spec §13.2–4):
- `slack_origin_delivers_to_telegram_and_acks_in_slack`: Slack-ingress scripted run delivers to the Telegram DM target; assert Telegram wire call + `Delivered` attempt, AND the lane-1 origin echo lands the final reply in the Slack conversation (existing observer seam), exactly one attempt per surface.
- `deliver_to_origin_conversation_is_denied_and_model_replies`: scripted trace targets the origin's own target id → tool error class PolicyDenied with the "already lands here" summary (assert via `ToolErrorClass` support), zero delivery attempts, run completes with a normal reply.
- `partial_failure_reports_per_call_honestly`: two deliver calls, second target's scripted vendor API returns a permanent error → first call's result has refs, second call's tool result is a Failed outcome naming the sanitized kind; attempt rows `Delivered` + `Failed{Rejected}`; trace pins the model's final text acknowledging the failed leg (scripted).
- `undeliverable_destination_is_refused_without_tool_calls` (spec §13.8): scripted trace for "email me the report" against a catalog with no Gmail surface → zero `outbound_deliver` calls, zero `trigger_create` calls, zero delivery attempts, and a scripted explanatory reply — pinned at the attempt-ledger and trace seams.

- [ ] **Step 1:** Write all three failing (they exercise wiring that exists after Task 5; failures must be assertion-level, not harness-level).
- [ ] **Step 2:** Fix any behavior gaps they expose (expected: none beyond mapping polish).
- [ ] **Step 3:** Suite + clippy green.
- [ ] **Step 4: Commit** `test(integration): cross-channel, same-origin, and partial-failure delivery journeys`

### Task 7: Notification-target set on the preference record

**Files:**
- Modify: `crates/ironclaw_outbound/src/communication_preferences.rs` (record field + helper), `crates/ironclaw_outbound/src/outbound_state_store.rs` + `store.rs` if serialization touches them
- Test: `crates/ironclaw_outbound/tests/outbound_state_store_contract.rs` (conformance — runs against both backends)

**Interfaces:**
- Produces: `CommunicationPreferenceRecord.notification_targets: Vec<OutboundDeliveryTargetId>` (`#[serde(default)]`; cap enforced at write seam, not serde), constant `pub const NOTIFICATION_TARGETS_CAP: usize = 8;`, and

```rust
impl CommunicationPreferenceRecord {
    /// Effective notification targets: the stored set, or the legacy
    /// single-slot default migrated read-side (spec §7). Ref→id migration
    /// needs the registry, so it takes the resolved entries.
    pub fn effective_notification_target_ids(
        &self,
        resolve_legacy: impl FnOnce(&ReplyTargetBindingRef) -> Option<OutboundDeliveryTargetId>,
    ) -> Vec<OutboundDeliveryTargetId> {
        if !self.notification_targets.is_empty() {
            return self.notification_targets.clone();
        }
        self.final_reply_target
            .as_ref()
            .and_then(resolve_legacy)
            .into_iter()
            .collect()
    }
}
```

Legacy fields (`final_reply_target`, `progress_target`, `approval_prompt_target`, `auth_prompt_target`) stay on the struct this task (deleted in Task 13 — the custom `Deserialize` impl keeps tolerating them forever).

- [ ] **Step 1: Write failing conformance tests**: `notification_targets_round_trip_and_default_empty` (write record with 2 ids, reopen store, read back; legacy-shaped JSON without the field deserializes to empty vec) and a unit test `legacy_single_slot_migrates_via_resolver` for the helper (closure returns a fixed id; empty stored set → one-element result; non-empty stored set → stored wins).
- [ ] **Step 2:** Run → FAIL. Implement (extend the existing custom `Deserialize` `Wire` struct with `#[serde(default)] notification_targets`).
- [ ] **Step 3:** `cargo test -p ironclaw_outbound` green; run the Docker-backed conformance leg: `DOCKER_HOST=unix:///$HOME/.colima/default/docker.sock cargo test -p ironclaw_outbound --features integration --test outbound_state_store_contract` (check the file's own header for the exact feature name before running).
- [ ] **Step 4: Commit** `feat(outbound): notification-target set with legacy single-slot migration`

### Task 8: `builtin.notification_channels_set` + service reshape (backend)

**Files:**
- Modify: `crates/ironclaw_product/src/reborn_services/outbound_delivery_capability_surface.rs` (new id/description/schema/parse beside targets_list; the old `target_set` surface stays until Task 11), `crates/ironclaw_product/src/reborn_services/outbound_preferences.rs` (new `set_notification_channels` + `get` reshape), `crates/ironclaw_product/src/reborn_services/types.rs` (DTOs), `crates/ironclaw_reborn_composition/src/runtime/local_dev/outbound_delivery.rs` (new synthetic handler cloned from the set handler's gate dance)
- Test: `tests/integration/outbound_target.rs` (extend — this file already owns the set/list seam), crate tests beside the service

**Interfaces:**
- Produces: capability id `builtin.notification_channels_set`, `PermissionMode::Ask`, effects `[DispatchCapability, ExternalWrite]`, `ConcurrencyHint::Exclusive`, full approval-gate dance copied from the existing set handler. Input schema: `{target_ids: {type: array, maxItems: 8, items: {type: string, minLength: 1, maxLength: 512}}}`, `additionalProperties: false`, required `["target_ids"]`. Description (verbatim const):

```text
Set the channels where IronClaw notifies this user about background runs (approval gates, re-authorization, failures) — full replace of the current set. Pass target ids from builtin__outbound_delivery_targets_list; an empty list means notifications stay in the web app only. This does not route replies or routine results — deliver those explicitly with builtin__outbound_deliver.
```

- Service: `RebornSetNotificationChannelsRequest { target_ids: Vec<String> }` → validates each id via the owner-scoped registry (`resolve_outbound_delivery_target`), dedups preserving order, caps at `NOTIFICATION_TARGETS_CAP` (excess → validation error), CAS-writes `notification_targets` (and clears the four legacy slots), returns `RebornNotificationChannelsResponse { channels: Vec<RebornOutboundDeliveryTargetOption> }`. GET side: `get_notification_channels` resolving stored ids to options with `available|unavailable` status.

- [ ] **Step 1: Failing tests** in `outbound_target.rs`: `notification_channels_set_replaces_and_reads_back` (set two ids → read-back through the service seam shows both; set empty → read-back empty; recorded write proves the seam, per that file's no-fabricated-success convention) and `notification_channels_set_rejects_foreign_and_overflow_ids`.
- [ ] **Step 2:** Run → FAIL. Implement surface + service + synthetic handler + composition registration next to the existing pair.
- [ ] **Step 3:** Integration suite + `cargo test -p ironclaw_product` + clippy + architecture gate green.
- [ ] **Step 4: Commit** `feat(product): notification_channels_set capability and service`

### Task 9: Background-run notifier rework (fan-out; result push dies)

**Files:**
- Modify: `crates/ironclaw_product/src/delivery_coordinator.rs` (add `BackgroundRunNotice` policy-class intent, `as_str` `"background-run-notice"`; DELETE `TriggeredDelivery`), `crates/ironclaw_product/src/run_delivery/triggered.rs`, `crates/ironclaw_product/src/run_delivery.rs` (services carry the registry handle for target resolution), `crates/ironclaw_extension_host/src/channel_triggered_delivery.rs` (hook now resolves the notification set instead of routing), `crates/ironclaw_outbound/src/resolution_engine.rs` (delete `resolve_triggered_target` / `resolve_triggered_explicit_target` / `Triggered{,WithTarget,FromSourceRoute}` origins — `RunScopedTarget` + `LiveSourceRoute` + `SystemEvent` remain)
- Test: `crates/ironclaw_product/tests/run_delivery_contract.rs`, `tests/integration/delivery_user_journeys.rs` (journeys 6–7), `tests/integration/group_triggers/scenario_triggered_gate.rs` (retarget assertions)

**Interfaces:**
- Consumes: Task 7 record/helper; `RunNotificationOrigin::RunScopedTarget`.
- Produces: `TriggeredRunDeliveryDriver` behavior contract:
  - `Completed` → **no delivery** (run-history recording unchanged).
  - `BlockedApproval` → one `GatePrompt` coordinated delivery **per** effective notification target (RunScopedTarget origin per target's binding ref, resolved through the registry at fire time; vanished targets skipped with a debug log).
  - `BlockedAuth` + URL → `AuthPrompt` only to targets whose catalog entry is a personal DM (`require_direct_message_target: true` stays as defense-in-depth); non-DM targets get a `BackgroundRunNotice` "A routine needs re-authorization - open the IronClaw app to continue." **The run is no longer cancelled when no DM-capable target exists** (spec §7): it parks, panel hold badge is the surface.
  - `BlockedAuth` manual-token → cancel (unchanged) + `BackgroundRunNotice` with the existing `AUTH_UNAVAILABLE_MESSAGE` to all targets.
  - Run failure → `BackgroundRunNotice` failure text to all targets.
  - Empty effective set → zero external attempts for every arm.
  - Gate routes: `record_gate_route_if_needed` called per delivered prompt conversation (it already records per-conversation).

- [ ] **Step 1: Read** `triggered.rs` fully plus `run_delivery_contract.rs`'s triggered arms (`triggered_final_reply_reaches_the_preference_target_with_footer`, `triggered_final_reply_honors_per_trigger_target_without_global_default`, `triggered_oauth_prompt_to_non_dm_target_cancels_and_notifies`, `triggered_project_scoped_fire_is_denied_without_delivery`).
- [ ] **Step 2: Rewrite those contract tests to the new behavior FIRST** (failing): `triggered_completed_run_delivers_nothing_external`, `triggered_gate_prompt_fans_out_to_every_notification_target`, `triggered_auth_prompt_reaches_only_dm_targets_and_run_stays_parked`, `triggered_manual_token_auth_cancels_and_notifies_all_targets`, `triggered_failure_notifies_all_targets`, `triggered_empty_notification_set_delivers_nothing`. Delete the superseded tests in the same commit (behavior intentionally changed — say so in the commit body; do NOT weaken assertions to keep them green).
- [ ] **Step 3:** Run → FAIL. Implement the rework: intent changes (compiler drives the `TriggeredDelivery` removal through `triggered.rs`, `channel_triggered_delivery.rs` outcome records, and any exhaustive matches), fan-out loop, origin/kind plumbing, resolution-engine deletions.
- [ ] **Step 4:** Add integration journeys 5–7 (spec §13), all via `submit_triggered_turn_scripted` from `tests/integration/support/triggered_submit.rs`: `routine_fire_delivers_via_tool_without_stored_target` (scripted fire calls `outbound_deliver` → bot wire call + `Delivered` attempt, no `delivery_target_id` anywhere), `conditional_fire_with_no_delivery_call_produces_zero_attempts` (scripted fire completes without the tool → attempt ledger empty, reply only in the fire thread), `blocked_fire_fans_out_and_first_approve_wins` (two notification channels; both receive the gate prompt; approve via one → run resumes; second approve → already-resolved outcome), and `empty_notification_set_keeps_blocked_fire_in_app_only` (zero attempts; `builtin.trigger_list` / automations list shows the hold).
- [ ] **Step 5:** Full crate + integration + clippy + architecture gates green (`RUST_MIN_STACK=16777216` for the deep suites).
- [ ] **Step 6: Commit** `feat(product): background-run notification fan-out; triggered result push removed`

### Task 10: WebUI — notification-channels panel

**Files:**
- Rename+rewrite: `crates/ironclaw_webui/frontend/src/pages/automations/components/automation-delivery-defaults-panel.tsx` → `notification-channels-panel.tsx` (radio → checkbox list over the same target options; Save = full replace; empty selection allowed with helper text "Notifications stay in the web app")
- Modify: `hooks/useOutboundDeliveryDefaults.ts` → `useNotificationChannels.ts`, `automations-page.tsx`, `crates/ironclaw_webui/frontend/src/lib/api.ts` (`GET/POST /api/webchat/v2/outbound/notification-channels`), `crates/ironclaw_webui/src/webui_v2/descriptors.rs` + `handlers.rs` + `router.rs` (new routes calling Task 8's service; old preference routes stay until Task 11), i18n `en.ts`, `crates/ironclaw_webui/src/webui_v2/static_assets/assets.rs` (embedded-asset pins)
- Test: frontend unit (`pnpm test` in `crates/ironclaw_webui/frontend`), `tests/e2e/scenarios/test_reborn_webui_v2_automation_trace_outbound_api.py` (extend the served-API test for the new routes), e2e manifest row

**Interfaces:** consumes Task 8 DTOs verbatim (`RebornNotificationChannelsResponse.channels[]` with `target_id`, `display_name`, `channel`, `status`).

- [ ] **Step 1:** Failing frontend test: checkbox multi-select renders options, toggles, posts `{target_ids: [...]}`, renders empty-state helper.
- [ ] **Step 2:** Implement panel/hook/api/routes/i18n/assets pins.
- [ ] **Step 3:** `pnpm lint && pnpm test` in the frontend; `cargo test -p ironclaw_webui`; extend the Playwright served-API scenario (auth required + shape) and add its `tests/e2e/reborn_coverage_tests.txt` row per `tests/e2e/CLAUDE.md`.
- [ ] **Step 4: Commit** `feat(webui): notification channels multi-select`

### Task 11: Delete the route_current stack + web_app pseudo-target + old set tool

**Files:**
- Delete: `crates/ironclaw_host_runtime/src/first_party_tools/outbound_delivery.rs`; `crates/ironclaw_outbound/src/run_final_reply_handoff.rs`
- Modify: `first_party_tools/mod.rs` (module, manifest entry, `UnavailableRunFinalReplyRouter`, register fn; id list back to 31), `schemas.rs` (route_current schemas), `crates/ironclaw_outbound/src/run_final_reply_target.rs` (delete `RouteCurrentRunFinalReply*` + `RunFinalReplyTargetRecord`/`Request`; keep `RunFinalReplyDestination` for now — Task 13 finishes it), `store.rs`/`outbound_state_store.rs` (`put/load_run_final_reply_target`, handoff methods + their contract tests), `crates/ironclaw_reborn_composition/src/factory.rs` (web_app provider registration + `host_owned_outbound_delivery_target_registry`), `outbound_delivery_capability_surface.rs` + `local_dev/outbound_delivery.rs` (delete `target_set` surface/handler), harness profiles (drop ROUTE_CURRENT registrations), `tests/reborn_trace_first_party_tool_coverage.rs` (swap the stale route_current row for `outbound_deliver` pointing at the live journeys), `local_dev/tests.rs` + `runtime/tests/outbound_delivery.rs` (delete/retarget selection tests)
- Test: run every touched crate's FULL suite `--no-fail-fast` (review-discipline rule for layer removal: unfiltered output, every failure adjudicated)

**Interfaces:** none produced; consumers were removed in Tasks 4–10 (verify: `rg -n "route_current|RouteCurrentRunFinalReply|RunFinalReplyTargetRecord|run_final_reply_handoff|WEB_APP_OUTBOUND_DELIVERY_TARGET_ID|outbound_delivery_target_set" crates/ tests/` must return only lines this task deletes).

- [ ] **Step 1:** Run the `rg` sweep above; list every hit; any hit NOT in this task's file list → STOP and report before deleting.
- [ ] **Step 2:** Failing-first where behavior remains: update `first_party_builtin_tools.rs` to the final 31-id list (out: `outbound_delivery_target_route_current`, `outbound_delivery_target_set` [it was never in this list — verify; it is a synthetic capability, not a builtin — adjust only what the list actually contains]; in: `outbound_deliver` from Task 4) → FAIL → delete code → PASS.
- [ ] **Step 3:** Full unfiltered suites for `ironclaw_host_runtime`, `ironclaw_outbound`, `ironclaw_product`, `ironclaw_reborn_composition`, integration `outbound_target.rs` (drop set-tool tests, keep list + notification tests) — every surfaced failure adjudicated per `.claude/rules/review-discipline.md` ("Removing a redundant layer un-masks behavior").
- [ ] **Step 4: Commit** `refactor(outbound)!: delete route_current stack, web_app pseudo-target, target_set`

### Task 12: Trigger `delivery_target_id` removal + prompt migration

**Files:**
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/trigger_management.rs` (input field, create path steps, output field, error arm, `TriggerCreateHook::{resolve,resolve_implicit,validate}_delivery_target` — delete from trait + impls; **rewrite `TRIGGER_CREATE_DESCRIPTION` and the schema `prompt`-field text now**, see below), `schemas.rs` (trigger_create schema), `crates/ironclaw_triggers/src/lib.rs` (`TriggerRecord.delivery_target` becomes read-tolerated: keep field `#[serde(default)]`, stop populating `TriggerFire.delivery_target` — delete that field; delete `parse_trigger_delivery_target_id`, `DeliveryTargetInvalid` arm), `postgres.rs`/`libsql.rs` (stop writing the column; keep reading), `crates/ironclaw_reborn_composition/src/factory.rs` (`LocalRuntimeTriggerCreatorPairingHook` delivery half deleted; add the startup migration), `crates/ironclaw_reborn_composition/src/automation/` (poller passes no target)
- Create: migration fn in `crates/ironclaw_reborn_composition/src/automation/trigger_delivery_migration.rs`
- Test: `crates/ironclaw_host_runtime/src/first_party_tools/trigger_management/tests.rs` + `tool_surface_contract.rs` (rewrite pinned substrings), `tests/integration/group_triggers/` (`scenario_delivery_target_fail_closed.rs` and `scenario_external_source_trigger_captures_delivery.rs` are superseded — replace with `scenario_trigger_create_has_no_delivery_target_field.rs` asserting the schema omits it and a stored-target-era record still fires), integration journey 10 (migration)

**Interfaces:**
- New `TRIGGER_CREATE_DESCRIPTION` (verbatim; guidance block in Task 14 stays the single deep teacher — this stays short):

```text
Create a scheduled routine. The prompt is the full task each fire performs, written for a future run with no memory of this conversation. If the user wants results delivered to a channel, write that as an explicit step in the prompt naming the destination (e.g. "then deliver the summary to my Slack DM with builtin__outbound_deliver") — pick destinations from builtin__outbound_delivery_targets_list while the user is present. Each fire's final reply is recorded in the routine's own run thread automatically; a fire that makes no delivery call delivers nothing externally.
```

- Migration `migrate_trigger_delivery_targets(repo, registry) -> Result<usize, TriggerError>`: for each trigger with `delivery_target: Some(id)` — resolve id via the creator-scoped registry; resolvable → append `"\n\nDeliver the result to {display_name} using builtin__outbound_deliver (target id: {id})."` to the prompt and clear the field via `upsert_trigger`; unresolvable → clear only. Idempotent (field `None` after first pass). Called from serve boot after the registry exists, before the poller starts; failures log loud and do not block boot (`// silent-ok: migration retries next boot, fire path ignores stored targets`).

- [ ] **Step 1:** Rewrite the pinned description tests to the new wording (failing), including deleting the nine old substring assertions and the `delivery_target_id` schema assertions in `tool_surface_contract.rs`.
- [ ] **Step 2:** Migration unit tests (failing): resolvable → prompt appended + cleared; unresolvable → cleared only; no-target → untouched; second run → no-op.
- [ ] **Step 3:** Implement all removals + migration; integration journey 10 (`stored_delivery_target_trigger_is_migrated_to_prompt`) via the harness boot path.
- [ ] **Step 4:** Full unfiltered suites for `ironclaw_triggers`, `ironclaw_host_runtime`, `ironclaw_reborn_composition`, `group_triggers` integration group; adjudicate every failure.
- [ ] **Step 5: Commit** `refactor(triggers)!: delivery_target_id removed; stored targets migrate into prompts`

### Task 13: Final deletions + retired-vocabulary pins

**Files:**
- Modify: `crates/ironclaw_extension_host/src/channel_triggered_delivery.rs` (delete `resolve_per_trigger_target`, `route_extension`, `stored_preference_target`, codec-selection block — the hook shrinks to notifier wiring), `crates/ironclaw_outbound/src/communication_preferences.rs` (delete the four legacy slot fields from the STRUCT — the custom `Deserialize` keeps accepting them from old rows and discards; delete `PreferenceTargetKind` slot lookups), `resolution_engine.rs` (remaining slot machinery), `run_final_reply_target.rs` (`RunFinalReplyDestination`: delete `WebApp` variant; if single-variant now, replace the catalog `destination` field with the sealed `ReplyTargetBindingRef` directly and delete the enum — follow `rg -n "RunFinalReplyDestination" crates/ tests/` to completion), `crates/ironclaw_architecture/tests/reborn_retired_taxonomy.rs` (add `RETIRED_TERMS`: `outbound_delivery_target_route_current`, `outbound_delivery_target_set`, `delivery_target_id`, `TriggeredDelivery`, `resolve_per_trigger_target`, `stored_preference_target` — with sanctioned-path entries for the migration file and this spec/plan under `docs/`)
- Test: architecture suite + full unfiltered crate suites

- [ ] **Step 1:** Add the retired-taxonomy pins FIRST → run → FAIL listing every remaining occurrence → that list is the deletion worklist.
- [ ] **Step 2:** Delete until the ratchet is green; adjudicate every surfaced behavior per review-discipline.
- [ ] **Step 3:** `cargo test -p ironclaw_architecture` + full suites for the three touched crates + the whole integration tier green.
- [ ] **Step 4: Commit** `refactor(outbound)!: delete slot machinery + pin retired delivery vocabulary`

### Task 14: The one guidance block + runtime-context rework

**Files:**
- Create: `crates/ironclaw_turns/prompts/delivery.md`
- Modify: `crates/ironclaw_turns/src/run_profile/runtime_context.rs` (render the block when delivery tools are visible; DELETE the five-branch delivery-target line + the ScheduledTrigger no-target warning; new origin-line wording), `crates/ironclaw_turns/src/run_profile/instruction_bundle.rs` (include_str! wiring, mirror `capability_surface_usage_policy.md`), `outbound_delivery_capability_surface.rs` (`TARGETS_LIST` description: drop "route only final replies and routine/trigger results", now "List destinations for builtin__outbound_deliver and for notification channels…")
- Test: `runtime_context.rs` inline tests (rewrite the ~12 pinned assertions), `tests/integration/comm_context.rs` (slice still renders)

**Interfaces:** `delivery.md` content (verbatim):

```markdown
## Delivery

Your reply already lands where this conversation lives — the web app thread,
the channel conversation, or this routine's run thread. Never re-send your
own reply, and never deliver to the conversation you are replying in.

To put content on ANOTHER surface, call `builtin__outbound_deliver` (one call
per destination, ids from `builtin__outbound_delivery_targets_list`). It sends
from IronClaw's own identity and returns provider message references — that
result is your delivery evidence; report it honestly and never claim a
delivery the result does not show. If a requested destination is not listed,
IronClaw cannot deliver there: say so and offer the destinations that exist.

Routines: write delivery as an explicit prompt step naming the destination.
A fire that makes no delivery call delivers nothing externally — that is how
conditional routines work.

Integration messaging tools (e.g. `slack.send_message`) act AS THE USER to
reach other people and places. "Send it to me" is bot delivery via
`builtin__outbound_deliver` by default, not an act-as-user send.
```

New ScheduledTrigger origin line (verbatim): `Run origin: scheduled trigger fire. The final reply is recorded in this routine's own run thread; it is not delivered externally. Deliver externally only if the prompt instructs it, using builtin__outbound_deliver.` New notification line (all origins, one line): `Background-run notifications: {none set - web app only | {n} channel(s) configured}.`

- [ ] **Step 1:** Rewrite the pinned runtime-context tests to the new strings (failing), including deleting warning-line tests.
- [ ] **Step 2:** Implement; `cargo test -p ironclaw_turns` + comm_context integration green.
- [ ] **Step 3: Commit** `feat(turns): single delivery guidance block; heuristic prompt lines removed`

### Task 15: Bundled skill + Slack prompt doc

**Files:**
- Modify: `skills/routine-advisor/SKILL.md` (rewrite the delivery sections: prompt-authored delivery steps, notification channels, no `delivery_target_id`), `crates/ironclaw_first_party_extensions/assets/slack/prompts/slack/send_message.md` (keep act-as-user framing; replace the final paragraph with: `Never use this tool to deliver your own reply or routine results to the user — that is bot delivery via builtin__outbound_deliver. This tool acts as the user to reach other people and places.`), `crates/ironclaw_extension_host/src/bundled_skills.rs` (`embedded_skills_teach_reborn_trigger_tools_not_retired_v1_routines` — new expected substrings: `builtin__outbound_deliver`, "delivery as an explicit prompt step"; drop `delivery_target_id`, "delivered automatically")
- Test: `cargo test -p ironclaw_extension_host bundled_skills` + Slack extension crate tests

- [ ] **Step 1:** Update the bundled-skills test expectations first (failing) → rewrite SKILL.md → green. **Note in the PR: `skills/` is product surface — this changes shipped agent behavior (two-skill-systems rule).**
- [ ] **Step 2:** Slack prompt doc edit + any pinned assertions (`rg -n "send_message.md|arrive twice" crates/ tests/`).
- [ ] **Step 3: Commit** `feat(skills): routine-advisor + slack send_message teach explicit delivery`

### Task 16: Recorded model-behavior fixtures

**Files:**
- Modify/Add: `tests/reborn_qa_recorded_behavior.rs` + `tests/fixtures/llm_traces/` — record two tool-choice fixtures: interactive "send me a summary of X on slack" → model calls `builtin.outbound_deliver` (not `slack.send_message`); a scheduled-fire trace with a delivery step in the prompt → same. Re-validate the two existing `assert_tool_not_called(..., "builtin.outbound_delivery_targets_list")` fixtures still hold (read tasks must still not reach for delivery tools).
- Test: `scripts/ci/check-reborn-qa-fixtures.sh`

- [ ] **Step 1:** Read `tests/support/reborn_parity_qa/CLAUDE.md` and the recording procedure used by the pushy-today commit `c8f02c2ef` (`git show c8f02c2ef` — donor for the recording workflow only).
- [ ] **Step 2:** Record, pin assertions (`assert_tool_called` on the new capability + argument shape), run the fixture validator (no secrets/PII).
- [ ] **Step 3: Commit** `test(qa): record explicit-delivery tool choice`

### Task 17: Law + docs

**Files:**
- Modify: `docs/internal/reborn/extension-runtime/overview.md` (§5.2 note + §5.4 rewrite per spec §10: model-initiated delivery as policy-class intent through the one coordinator; sole-writer/attempt/crash language restated; boundary note: delivery tool = model delivering as the assistant, vendor send tools = model acting as the user, final replies = lane 1 and never ride either; "emitters never know what channel" scoped to host-emitted intents), `docs/internal/reborn/extension-runtime/checklist.md` (OUT items: add model-delivery evidence — provider refs in the tool result; no queued state in v1)
- Create: `.claude/rules/tools.md` + `.claude/rules/tool-evidence.md` — start from `git show pushy-today:.claude/rules/tools.md` / `tool-evidence.md`, fix the known stale bits (drop the "Everything Goes Through Tools" heading citation; verify every named path exists on main before committing)
- Modify: root `CLAUDE.md` (stale-doc rider from spec §10: `[channel.config]` → top-level `[admin_configuration]`, `ChannelAdapter` path, Slack tool count — cross-check each against live code first; also add the delivery-tool row where CLAUDE.md describes outbound), `.claude/skills/reborn-extension-surfaces/SKILL.md` (same drift, per the ironclaw-reborn-skill-maintainer skill's rules)
- Test: none mechanical (no docs-shape test greps overview.md — verified in recon); `rg` every path named in the edited docs to prove it exists

- [ ] **Step 1:** §5.4 + checklist rewrite; every claim cross-checked against the code as landed by Tasks 1–13.
- [ ] **Step 2:** Port + fix the two rule files; verify frontmatter paths.
- [ ] **Step 3:** CLAUDE.md + skill drift fixes (each fix verified against live code, not the salvage branch).
- [ ] **Step 4: Commit** `docs(reborn): §5.4 model-initiated delivery; port tool evidence rules`

### Task 18: Full gate + coverage manifests

**Files:**
- Modify: `tests/e2e/journey_cases.py` + `tests/e2e/journey_types.py` if the journey-coverage gate (`tests/e2e/test_journey_coverage.py`) demands evidence rows for changed surfaces; `tests/integration/coverage-floor.toml` recapture if the ratchet asks (follow that file's same-PR instructions)

- [ ] **Step 1:** `cargo fmt --check`
- [ ] **Step 2:** `cargo clippy --all --tests --examples -- -D warnings` AND `cargo clippy --all --tests --examples --all-features -- -D warnings` (both lanes — PR CI only runs one).
- [ ] **Step 3:** `cargo test` (workspace unit), `cargo test -p ironclaw_architecture`, `RUST_MIN_STACK=16777216 bash scripts/reborn-e2e-rust.sh`, Docker-backed legs with `DOCKER_HOST` set.
- [ ] **Step 4:** `pnpm lint && pnpm test` (frontend); Python e2e coverage gates (`pytest tests/e2e/test_journey_coverage.py tests/e2e/test_provider_capability_inventory.py`); fix manifests as their failure messages instruct.
- [ ] **Step 5:** `scripts/pre-commit-safety.sh` clean run.
- [ ] **Step 6: Commit** `test: coverage manifests for explicit delivery` — then assemble the three PRs per the Global Constraints train.

---

## Self-Review (done at write time)

- **Spec coverage:** §5 tool contract → Tasks 2–4; §6 architecture → Tasks 3–5; §7 notifications → Tasks 7–10; §8 automations → Tasks 9, 12; §9 guidance → Tasks 14–15; §10 law/docs → Task 17; §11 deletions → Tasks 11–13; §12 error handling → Tasks 3–4, 9; §13 testing → Tasks 5, 6, 8, 9, 12, 16, 18; §14 train → Global Constraints; §15 non-goals → nothing builds them.
- **Known judgment points for executors:** (a) exact `OutboundPolicyService` construction is mirrored from `observer.rs`, not invented; (b) `outbound_delivery_target_set` may not be in the frozen builtin list (synthetic lane) — Task 11 Step 2 says verify before editing; (c) struct-ratchet updates follow the failing test's own instructions.
- **Type consistency:** the port shape (Task 2; invocation identity rides `scope.invocation_id`, ruled 2026-07-27) is consumed verbatim in Tasks 3–5; Task 8's `RebornNotificationChannelsResponse` is consumed verbatim in Task 10.
