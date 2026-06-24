# Fusion Design: Slack as a Generic Host-Ingress/Egress Integration (Reborn)

**Status:** Proposed (design-council fusion — Opus 4.8 + GPT-5.5 xhigh; final signoff: both ACCEPT_WITH_NONBLOCKING_NOTES, no unresolved blockers; notes folded in)
**Date:** 2026-06-17

## 1. Problem statement

Slack is wired as a **special composition path** in the Reborn host. `ironclaw-reborn serve` directly builds Slack-specific route mounts, HMAC verification, workflow dispatch, outbound reply handling, and drain lifecycle. The intended invariant is:

> Everything that crosses the Reborn kernel boundary goes through **host-owned ingress and egress**. Extensions/product-adapters own *protocol semantics only*; the host owns ingress, egress, secrets, policy, lifecycle, and kernel access.

The fix must make Slack a **declared integration** whose only kernel-boundary crossing is host ingress/egress — and the end state must be **stricter** than today, not "an extension with raw host power."

## 2. Goals / Non-goals

**Goals:** generic host-ingress contract; host builds the Axum router (adapter provides only a handler); host-owned ingress secret resolution mirroring egress; generic egress *selection*; generalize to future webhook integrations; full behavior parity with a reversible migration.

**Non-goals:** rewriting Slack protocol parse/render; changing kernel/ProductWorkflow internals; a third-party dynamic-loading plugin runtime (first-party bundled manifests only); multi-tenant OAuth distribution beyond what exists.

## 3. Constraints & verified ground truth

These already exist and are **reused, not reinvented** (verified by reading the code):

- `IngressRouteDescriptor` / `IngressPolicy` — `crates/ironclaw_host_api/src/ingress.rs` (route_id, method, route_pattern, policy: listener_class/auth schemes incl. `WebhookSignature`/body/rate/cors/audit/`effect_path`). `IngressPolicy::new` validation enforces the `PublicWebhook ⇔ WebhookSignature` invariant.
- `PublicRouteMount { router, descriptors, drain }` + `trait PublicRouteDrain` — `crates/ironclaw_reborn_composition/src/webui_serve.rs:266,274`. The `router: Router` field is the **leak**.
- Slack already builds a valid descriptor (`slack_events_policy()`, `slack_serve.rs:225-258`) — declaration exists; what's bespoke is *who builds the Router and verifies HMAC*.
- Egress is **already host-mediated**: `SlackProtocolHttpEgress: ProtocolHttpEgress` → `HostRuntimeHttpEgressPort::execute()` with `NetworkPolicy`, `TrustClass::System`, opaque `EgressCredentialHandle` (`slack_egress.rs`). Adapter never calls Slack API directly.
- `SecretStore::lease_once(scope, handle)` + `consume(scope, lease_id) -> SecretMaterial` exist (`crates/ironclaw_secrets/src/lib.rs:985,1013`; re-wrapped in `host_runtime/src/obligations.rs:618-632`).
- `installation.credential_bindings()` exists and is already iterated (`crates/ironclaw_product_adapter_registry/src/lib.rs:407`).
- `HostApiContractRegistry` + `ProductAdapterHostApiContract::new()` registration pattern (`host_runtime/src/extension_contracts.rs:18-26`) — a new `HostIngressHostApiContract` slots in idiomatically.
- `NativeProductAdapterRunner` already owns a bounded admission `Semaphore` (max 64 in-flight), workflow timeout, observer path, and `drain_immediate_ack_tasks()` over an internal `JoinSet` (`runner.rs:243,271`; `runner_immediate_ack.rs:88-159`).
- The Slack HMAC format is **hardcoded** in `auth_verifier.rs:185,197` (`v0:{ts}:{body}`, `v0={hex}`, `DEFAULT_HMAC_MAX_AGE_SECS`) — only the one Slack scheme exists.

## 4. Final design

### 4.1 Crate ownership (resolves "keep host_api neutral")
- **`ironclaw_host_api`** — *declaration vocabulary only*. No `HeaderMap`/`Bytes`/handler traits/crypto.
- **`ironclaw_host_ingress_registry`** (new, mirrors `ironclaw_product_adapter_registry`) — manifest → typed declaration projection; `HostIngressHostApiContract`.
- **`ironclaw_reborn_composition`** — host mounting loop, executable handler trait + request/response value types, the credential resolver, and the Slack bridge handler. Reuses `PublicRouteMount`/`PublicRouteDrain`.
- **`ironclaw_slack_v2_adapter`** — pure protocol parse/render (unchanged).

### 4.2 Declaration types — `ironclaw_host_api/src/ingress.rs` (additive)
```rust
pub struct HostIngressRouteDeclaration {
    pub route_id: IngressRouteId,
    pub method: NetworkMethod,
    pub route_pattern: IngressRoutePattern,   // identity; validated to parse
    pub policy_profile: IngressPolicyProfile,  // NAMES a code-built policy; NOT a hand-authored IngressPolicy
    pub target: HostIngressTarget,
    pub auth: Vec<IngressAuthBinding>,         // >=1; multiple handles allowed (rotation/multi-install)
    pub ack: IngressAckMode,                   // AwaitHandler | Immediate
    pub drain: IngressDrainMode,               // None | DrainBeforeRuntimeShutdown
}
pub enum HostIngressTarget { ProductAdapterInbound { capability_id: CapabilityId, product_adapter_section: String }, HostCapability { capability_id: CapabilityId } }
pub struct IngressAuthBinding { pub scheme: IngressAuthSchemeName, pub credential_handles: Vec<IngressCredentialHandle> }
// IngressAuthSchemeName names a CODE-built verifier (e.g. "slack_v0_hmac"); crypto internals stay in auth_verifier.rs.
```
**Validation:** `Immediate ⇒ DrainBeforeRuntimeShutdown`; `ProductAdapterInbound ⇒ effect_path == ProductWorkflow`; profile's declared schemes ⊇ `auth[].scheme`; `credential_handle` is opaque, never a `SecretString`. **No `IngressPolicy` or HMAC constants in TOML.**

### 4.3 Manifest + registry projection
Slack manifest (`assets/slack/manifest.toml`) grows a `host_ingress.events` section carrying **identity + profile name + scheme name + credential handle(s) + ack/drain + target** only. `ironclaw_host_ingress_registry` projects it into a typed `HostIngressRouteDeclaration` and resolves `policy_profile` → a Rust-constructed `IngressPolicy` via the **existing** validated constructor (reuse `slack_events_policy()`). A test asserts the projected policy `==` the code constant, eliminating drift. `HostIngressHostApiContract` is registered in `host_runtime/extension_contracts.rs`. `list_enabled_host_ingress_entries(store)` enumerates enabled installations' ingress.

### 4.4 Handler trait + host mounting loop — `ironclaw_reborn_composition/src/host_ingress.rs` (new)
```rust
#[async_trait] pub trait HostIngressCapabilityHandler: Send + Sync {
    // Bounded, side-effect-free parse of UNTRUSTED envelope to narrow candidate installations (cap 8).
    async fn auth_candidates(&self, req: &UnverifiedHostIngressRequest<'_>) -> Result<Vec<HostIngressAuthCandidate>, HostIngressError>;
    // Called only AFTER host verifies exactly one candidate and mints installation-scoped evidence.
    async fn handle_verified(&self, req: VerifiedHostIngressRequest) -> Result<HostIngressImmediateResponse, HostIngressError>;
    fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output=()> + Send + 'a>> { Box::pin(async {}) }
}
pub fn public_ingress_route_mount(regs: Vec<HostIngressRegistration>, creds: Arc<dyn HostIngressCredentialResolver>) -> Result<PublicRouteMount, HostIngressError>;
```
`public_ingress_route_mount` is the **only** code that builds Axum routes. Per `(method, route_pattern)` group (fail closed on duplicate `route_id` / method-path collision / unsupported scope): one generated host handler that **(1)** buffers body once under `BodyLimitPolicy`; **(2)** applies coarse `RateLimitPolicy` (Global, pre-auth); **(3)** calls `auth_candidates` (bounded, post-body-limit); **(4)** for each candidate (hard cap 8 — the cap bounds resolver fan-out **even on cache miss**, so a forged envelope claiming many installs cannot amplify secret-store load) resolves its signing secret(s) via the credential resolver and verifies HMAC using the code-built verifier; **(5)** requires **exactly one** match (reject 0 or >1 — fail closed); **(6)** mints installation-scoped `ProtocolAuthEvidence`; **(7)** calls `handle_verified`; **(8)** translates `HostIngressImmediateResponse` to an Axum `Response`. URL-verification challenge is echoed only after exactly-one-install match. The adapter never receives `Router`, `SecretString`, raw `SecretHandle`, listener lifecycle, or `RebornRuntime`.

### 4.5 Immediate-ACK + drain (reuse runner; host orders only)
`HostIngressImmediateResponse` represents "accepted & scheduled" — **not** an opaque `Future` handed to the host. `SlackEventsIngressHandler::handle_verified` delegates to `NativeProductAdapterRunner::process_verified_webhook_immediate_ack(...)`, reusing its bounded semaphore, timeout, **observer error path** (post-ACK workflow failures are observable — not hidden), and `drain_immediate_ack_tasks()`. The handler's `drain()` delegates to the runner; `public_ingress_route_mount` aggregates handler drains into a host-owned `PublicRouteDrain`. The drain ordering is owned by `PublicRouteDrains::drain` in `webui_serve.rs` (constructed ~:619, consumed ~:823 in the serve flow) — drained before `runtime.shutdown()`, unchanged by this design.

### 4.6 Ingress secret resolution — `ironclaw_reborn_composition/src/host_ingress_credentials.rs`
```rust
#[async_trait] pub trait HostIngressCredentialResolver: Send + Sync {
    async fn resolve_ingress_secret(&self, installation: &ExtensionInstallation, handle: &IngressCredentialHandle, scope: &ResourceScope) -> Result<SecretMaterial, HostIngressError>;
}
// impl ExtensionInstallationIngressCredentialResolver { secret_store: Arc<dyn SecretStore> }
//   find binding by handle on installation.credential_bindings() -> lease_once(scope, binding.secret_handle()) -> consume(...)
```
Per-request resolution supports rotation (active + previous secret bindings) and per-installation distinct secrets on one path. The env→`SecretString` read in `serve_slack.rs` is replaced by an installation credential binding (bridged from `[slack]` env during migration). Host MAY cache the resolved material with a short TTL keyed by `(handle, scope)` to bound secret-store load on the public path (12k/min ceiling).

### 4.7 Egress (separate, later migration — reuse, don't fork)
Reuse existing `DeclaredEgressTarget` / `EgressCredentialHandle` / `EgressPolicy`. Extend the product-adapter credential declaration: `ProductAdapterCredentialDecl { handle, target: RuntimeCredentialTarget, required }` so the bot-token injection target is declared in manifest. Introduce `HostMediatedProtocolHttpEgress` selected by installation/capability; retire `SlackProtocolHttpEgress` / `StaticSlackEgressCredentialProvider`. **Do not** introduce a parallel `DeclaredHttpEgress`. Sequence this *after* ingress parity lands.

## 5. Key decisions & alternatives rejected
- **Wrap `IngressRouteDescriptor`, never extend/replace** — preserves wire contract + validation; policy stays the security source of truth. *Rejected:* adding `ingress_secret`/`handler` fields in place (breaks wire contract; entangles policy with resolution).
- **Manifest declares identity+profile; registry projects to code-built `IngressPolicy`** — first-party bundled manifest is in scope (non-goal forbids only *third-party* dynamic loading), and matches the handover principle "declared through extension state," while a projection-equality test eliminates drift. *Rejected:* hand-authoring full `IngressPolicy`/HMAC constants in TOML (silent crypto drift — verifier ignores TOML and uses hardcoded `v0=`); pure code-wired declaration (loses the declarative end state — kept only as transient shim).
- **Per-request candidate secret resolution** — only correct option for multi-install + rotation; mount-time single verifier goes stale and can't disambiguate installs. *Rejected:* mount-time resolution.
- **Runner owns background work; host orders drain** — reuses tested bounded backpressure + observable failures. *Rejected:* host owns an opaque `Future<Output=()>` (bypasses semaphore/timeout/observer, hides post-ACK errors).
- **Keep executable handler/request types out of `host_api`** — host_api is neutral declaration vocabulary. *Rejected:* `HeaderMap`/handler traits in host_api.

## 6. Test & validation plan
Drive `public_ingress_route_mount` end-to-end via `tower::ServiceExt::oneshot` (mirrors current `post_to_mount`, `slack_serve.rs:689`) — test through the host mount/serve composition, not helpers.

| Brief test | Drives |
|---|---|
| URL verification | signed `url_verification` → exactly-one-install → `Respond{challenge}`, 200 |
| Forged-HMAC rejection (**cutover gate**) | bad signature → 401, handler/dispatcher call-count 0 |
| Multi-install exactly-one-match (**cutover gate**) | two installs on one path → correct install selected; 0-match → 401; >1-match → fail closed |
| Rotation | active+previous secret both verify; rotated-away secret rejected |
| DM dispatch | signed DM `event_callback` → workflow accepted after drain |
| Channel-mention dispatch | `app_mention` → channel subject route |
| Bot/subtype ignore | bot/`message_changed` → no user turn |
| Personal binding / pairing-code | WebUI pairing redeem + Slack DM challenge via egress |
| Shared-channel route mapping | configure route → post channel event → asserted subject user |
| Approval prompt delivery / reply resume | approval gate → egress send + delivered route; signed reply → gate resumes |
| Auth prompt delivery / completion outside Slack | auth challenge egress; OAuth/WebUI callback → Slack delivered route still resolves |
| Delivered-route resolution | reply target resolves from `DeliveredGateRouteStore` |
| Final reply delivery | completion → egress with capped body |
| Idempotency / duplicate event | same `event_id` twice → one workflow side effect |
| Graceful shutdown drain (**cutover gate**) | slow immediate-ACK task → `PublicRouteDrain::drain` completes it before runtime shutdown |
| Duplicate route_id / method-path collision | composition fails closed at mount |
| Projection equality | manifest-projected `IngressPolicy` == `slack_events_policy()` constant |

## 7. Migration plan (each step shippable & reversible)
**Gate:** `[slack].host_ingress_mode = "legacy" | "generic_shadow" | "generic"` (default `legacy`). `generic_shadow` builds + validates declarations/projection **but mounts the legacy route** — it proves *projection/construction*, NOT dispatch parity. Dispatch parity is proven only under `generic`. Keep `slack` Cargo feature, `slack-v2-host-beta` alias, `[slack].enabled=true` throughout.

1. **host_api** declaration types + validation tests (dormant).
2. **`ironclaw_host_ingress_registry`** + register `ironclaw.host_ingress/v1` in host_runtime; add Slack manifest `host_ingress.events`; projection-equality test. Not mounted.
3. **composition** host loop + handler trait + request/response types + generated handler; fake-integration tests incl. duplicate/collision fail-closed.
4. **composition** `ExtensionInstallationIngressCredentialResolver` (lease/consume) + per-request candidate resolution; bridge `[slack]` env secret → installation credential binding.
5. **composition** `SlackEventsIngressHandler` (`auth_candidates` + `handle_verified`) delegating to `NativeProductAdapterRunner`; reuse runner drain. **Confirm the code-built HMAC verifier (`auth_verifier.rs`, currently in `ironclaw_wasm_product_adapters`) is reachable from composition without a layering cycle** — if it isn't, move it to a neutral host-runtime module *in this step* rather than deferring to open question 3.
6. Add tri-state gate; wire `generic`/`generic_shadow` in serve.rs.
7. Run full parity suite under `generic`; flip default to `generic`. Keep `legacy` one release.
8. Delete `build_slack_host_beta_mounts()` route construction, raw `signing_secret`/`bot_token` fields on `SlackHostBetaConfig`, Slack-named `WebuiServeConfig` builder methods. Keep features/aliases.
9. **Separate migration:** egress generalization (`HostMediatedProtocolHttpEgress` + manifest credential target); retire `SlackProtocolHttpEgress`.

## 8. Risks & mitigations
- **Pre-auth DoS via candidate parse** → bounded, side-effect-free, post-body-limit, cap 8.
- **HMAC drift** → verifier code-built + named scheme; projection-equality test; move (not rewrite) `HmacWebhookAuth` logic.
- **Multi-install ambiguity** → fail closed on 0 or >1 match (preserve current ambiguous-install behavior).
- **Secret-store load on public path** → sanctioned lease/consume; runner caps concurrency at 64; candidate cap (8) bounds per-request resolver fan-out even on cache miss; optional short-TTL host-side cache **must** be bounded with explicit TTL, scope-aware keys, and no logging/debug exposure of secret material.
- **Cutover behavioral parity** → cutover tests pin legacy-compatible HTTP status *and* body for 0-match, >1-match, bad-HMAC, and URL-verification, not just status codes.
- **Drain regression** → reuse runner `drain_immediate_ack_tasks()`; host orders via existing `serve.rs:528` path; drain test is a cutover gate.
- **Two routing paths during migration** → `public_ingress_route_mount` *produces* a `PublicRouteMount`, so downstream middleware is identical; only construction differs, transiently.
- **Egress blast radius** → reuse existing egress declarations; sequence as a separate migration, not coupled to ingress cutover.
- **Shadow false confidence** → documented: shadow proves projection only, not dispatch.

## 9. Agreement ledger
- Wrap (not extend/replace) `IngressRouteDescriptor` — **both ACCEPT**.
- Adapter provides handler; host builds Router — **both ACCEPT** (core inversion).
- Manifest identity+profile → code-built `IngressPolicy` via registry; no crypto/policy in TOML — **both ACCEPT** (Opus's drift correction + GPT's projection requirement merged).
- Per-request candidate secret resolution via `SecretStore` lease/consume — **GPT proposed, Opus conceded**.
- Runner owns bounded in-flight + drain; host orders only — **GPT proposed, Opus conceded**.
- Executable handler/request types stay out of `host_api` — **GPT raised, Opus accepts**.
- Reuse existing egress declarations; separate egress migration — **both ACCEPT** (GPT "don't fork egress" + Opus "decouple egress").
- `auth_candidates` bounded/post-body-limit/cap-8/fail-closed — **both ACCEPT**.
- Tri-state migration gate; shadow proves projection not dispatch — **GPT proposed, Opus accepts**.

## 10. Unresolved blockers
None.

## Open questions (non-blocking)
1. Should `ExtensionInstallation` grow typed non-secret installation settings, or keep Slack metadata in a Slack-owned host-state store initially? (Lean: keep Slack host-state for v1.)
2. Migrate Slack pairing / channel-admin WebUI routes (session-authed `ProtectedRouteMount`) into generic ingress now, or only the events webhook first? (Lean: events webhook first; pairing/admin need no secret-handle change.)
3. Move the HMAC verifier out of `ironclaw_wasm_product_adapters` into a neutral host-runtime module? (Lean: yes, when the second integration needs it.)
