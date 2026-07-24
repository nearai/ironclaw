# PR #6159 Architecture Simplification Design

**Date:** 2026-07-17

**Status:** Implemented and verified

**Baseline:** `nearai/ironclaw#6159` at `0575d3815d03fce0d43e6247f0bb3956af9e9ada`

**Governing direction:** `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`

## Goal

Refactor the Telegram host work in PR #6159 so it preserves the shipped behavior while
conforming materially better to the repository's architecture direction: composition is
assembly-only, traits represent genuine runtime variation or dependency inversion, domain
state uses the existing filesystem seam instead of local test-store families, and types
represent distinct states rather than mirrored transport wrappers.

This is a structure-preserving refactor. The Telegram setup, webhook, pairing, identity,
delivery, security, and rollback contracts in `docs/reborn/contracts/telegram-v2.md` remain
unchanged.

## Scope boundary

This implementation fixes the architecture debt introduced or materially expanded by PR
#6159. It is not the repository-wide kernel rewrite proposed by the simplification note,
and it does not implement the full Unified Extension Runtime migration.

In scope:

1. Move generic channel-delivery behavior out of `ironclaw_reborn_composition`.
2. Make the Telegram host core concrete and remove same-crate test-seam traits.
3. Remove Telegram mirror and speculative DTOs.
4. Replace the Telegram-specific lifecycle slot with a provider-neutral account-setup
   registry.
5. Move Telegram-specific delivery-hook and revision-runtime behavior out of composition.
6. Split oversized Telegram modules by domain responsibility.
7. Add red-first architecture ratchets that prevent each removed pattern from returning.

Explicit non-goals:

- Do not rebase or resolve PR #6159's conflicts with current `main`; that is a separate
  integration step after this architecture branch is complete.
- Do not change Telegram HTTP routes, request/response JSON, manifest fields, secret
  handles, persisted record shapes or paths, identity keys, webhook behavior, pairing
  semantics, delivery-status mapping, or feature flags.
- Do not migrate Slack, WebUI, OpenAI compatibility, or product auth wholesale to the
  Unified Extension Runtime.
- Do not implement the simplification note's capability-kernel DTO collapse, runtime-lane
  enum, or repository-wide `LocalDev*` and `InMemory*Store` deletion.
- Do not add a second Telegram extension, a Telegram tool surface, or ambient network,
  filesystem, or secret authority.

## Design principles

### Ownership follows behavior

`ironclaw_reborn_composition` may construct services, supply production dependencies, and
register route fragments or hooks. It may not own delivery algorithms, Telegram revision
caches, Telegram-specific runtime decorators, or lifecycle policy keyed to the literal
`telegram` product.

### A trait must earn its seam

A retained trait must have at least two production implementations, be an intentional
decorator/strategy seam, or invert a dependency that cannot point downward. A trait whose
only alternate implementation is a same-crate test fake is replaced by a concrete type
tested through a genuine lower-level seam.

### One type per meaningful state

Wire input, persisted secret update, persisted record, redacted public status, verified
ingress, and authorized delivery are genuinely distinct states. A one-field wrapper or an
identical status mirror is not.

### In-memory is a backend, not a domain implementation

Telegram persistence tests use the production `FilesystemTelegramHostState` over
`InMemoryBackend`. Failure tests inject failure at `RootFilesystem`, secret-store, or
mediated HTTP boundaries instead of maintaining parallel `InMemory*Store` domain models.

## Target architecture

### 1. New `ironclaw_channel_delivery` owner

Create `crates/ironclaw_channel_delivery` as the product-neutral owner of the generic
delivery engine currently implemented in
`ironclaw_reborn_composition/src/outbound/channel_delivery.rs`.

The crate owns focused modules for:

- `observer`: live inbound-ack observation and final-reply delivery;
- `triggered`: triggered-run target resolution and delivery;
- `actionable`: approval/auth notification projection and cancellation behavior;
- `routing`: reply-target validation, route persistence, and target authority;
- `hooks`: post-submit delivery hook, keyed composite registry, and no-op behavior;
- `services`: delivery dependencies and settings.

It consumes existing contract-owner ports such as `ChannelDeliveryProtocol`,
`OutboundDeliveryTargetProvider`, `ProductAdapter`, `ProtocolHttpEgress`,
`OutboundDeliverySink`, `SessionThreadService`, `TurnCoordinator`, and outbound stores. It
does not define channel-specific protocol logic and does not depend on
`ironclaw_reborn_composition`.

The auth-prompt projection and blocked-flow cancellation inputs currently declared under
composition's product-auth API move to the lowest reusable owner that can express them
without an upward dependency. Existing production product-auth services continue to
implement those dependency-inversion ports. This is a justified dynamic seam because the
implementation must remain above the delivery crate and because a missing product-auth
service is a supported production configuration. No composition-owned request wrapper may
remain solely to cross this boundary.

`ironclaw_channel_host` remains the smaller owner of vendor-neutral channel-host contracts
and host-ingress helpers. Its guidance is updated to point delivery behavior at the new
crate. Delivery behavior is not folded into `ironclaw_channel_host`, avoiding a broad
god-crate and preserving the distinct host-contract versus delivery-engine responsibilities.

Composition's outbound modules retain only factories that assemble
`FinalReplyDeliveryServices`, construct observers/drivers, and install them into runtime
slots.

### 2. Concrete Telegram host core

`FilesystemTelegramHostState` becomes the single concrete owner of Telegram setup,
pairing, user binding, and DM-target persistence. It stores an
`Arc<ScopedFilesystem<dyn RootFilesystem>>` or equivalent erased filesystem backend so
production service types do not need a second layer of Telegram store traits or generic
parameters.

The following same-crate, single-production-implementation traits are deleted:

- `TelegramInstallationSetupStore`
- `TelegramPairingStore`
- `TelegramUserBindingStore`
- `TelegramDmTargetStore`
- `TelegramEgressCredentialProvider`
- `TelegramInstallationResolver`

Their methods become inherent methods on the concrete Telegram service/state that owns
the operation. Tests instantiate real state over `InMemoryBackend`. Store outage and
compare-and-swap failure tests use a failure-injecting `RootFilesystem` test support
wrapper, so they continue to prove fail-closed behavior without a parallel domain store.

`TelegramBotApi` is also removed if the existing mediated host HTTP seam can express every
current success, rejection, malformed-envelope, and compensation test without weakening
the credential boundary. The concrete Bot API client remains Telegram-owned; tests fake
the mediated HTTP transport, never Telegram domain behavior. If a concrete migration
would require exposing token bytes or bypassing `ironclaw_network`, implementation stops
at the existing mediated HTTP boundary and records the retained trait as a security
dependency-inversion exception in the checklist. The default and acceptance target is
deletion.

Retained dynamic seams are limited to genuine cross-owner or multi-implementation ports:

- `ChannelDeliveryProtocol`
- `RebornUserIdentityLookup`
- `OutboundDeliveryTargetProvider`
- `TelegramRevisionWorkflowBuilder`
- `TelegramChannelSetupActivation`
- `TelegramUpdatesWebhookDispatcher` and its dispatcher decorator chain
- product-auth challenge/cancellation ports required by the generic delivery owner

### 3. DTO and wrapper deletion

Delete `TelegramPairingStatusResponse`; route handlers serialize the existing redacted
`TelegramPairingStatus` projection directly. This removes an identical manual copy while
preserving the public JSON shape.

Delete `ResolvedTelegramIngress`; the resolver and webhook route consume
`ResolvedTelegramInstallation` directly. The implementation has exactly one resolved
ingress shape, so the wrapper encodes no state transition.

Audit all new Telegram public structs, enums, request wrappers, accessors, and builders
introduced by the PR. Delete or narrow any item that has no downstream production
consumer, represents a future-only option, or only republishes another owned type. Keep
the distinct shapes that enforce real boundaries:

- untrusted setup input to secret-bearing update;
- full persisted setup record to redacted public status;
- raw webhook request to verified installation context;
- live setup revision to cached workflow/runtime state.

Public visibility is narrowed to `pub(crate)` where cross-crate consumers do not exist.

### 4. Provider-neutral account-setup registry

Replace `telegram_paired_source: ChannelPairedStatusSlot` and
`with_telegram_pairing_requirement` in generic extension lifecycle with a registry keyed
by `ExtensionId`.

The registry entry supplies:

- a provider-neutral account-connection status source for the authenticated user;
- the `RuntimeCredentialAuthRequirement` to add when disconnected;
- a stable provider id and requester extension identity;
- an error classification that distinguishes an unregistered host from a transient
  status-read outage.

The lifecycle algorithm is generic:

1. Load and authorize the installation exactly as today.
2. Derive manifest-declared credential requirements.
3. Look up an optional account-setup registration by extension id.
4. If registered, query the authenticated user's connection status.
5. Add the registration's requirement when disconnected.
6. Fail closed when the package requires registered account setup but the host has not
   supplied the registration; map status-read failures to the existing transient error.

Telegram constructs and registers its pairing requirement from its host module. Generic
lifecycle source contains no Telegram identifiers, feature-specific fields, or imports.
The existing pairing challenge shape and provider id remain unchanged. Pairing
dispatches the extension-scoped `LifecycleActivation` continuation, and the
resumed run recomputes readiness.

### 5. Telegram behavior leaves composition

Move these Telegram-specific behaviors from
`ironclaw_reborn_composition/src/telegram/telegram_host_beta.rs` into
`ironclaw_telegram_extension`:

- dynamic setup-revision workflow construction and caching;
- Telegram's triggered-delivery hook adapter;
- Telegram-specific no-op/fallback implementations;
- Telegram dispatcher decoration and revision-host behavior;
- construction of Telegram outbound targets and delivery protocol from Telegram-owned
  dependencies.

Composition keeps a thin `build_telegram_host_runtime_mounts`-style factory that:

1. gathers already-composed neutral dependencies;
2. builds a Telegram host input value;
3. calls the Telegram-owned builder;
4. mounts returned public/protected route fragments;
5. registers returned facades, account-setup registration, outbound target provider, and
   post-submit delivery hook.

The Telegram crate does not own `RebornRuntime`, listener binding, or global mount
assembly. The builder returns facade-shaped parts rather than raw substrate handles.

### 6. Focused Telegram modules

Split oversized Telegram source files by responsibility while preserving public module
paths through deliberate re-exports only where a downstream consumer exists.

Target layout:

```text
src/
  setup/
    mod.rs
    service.rs
    status.rs
    compensation.rs
  pairing/
    mod.rs
    code.rs
    service.rs
    status.rs
  ingress/
    mod.rs
    resolver.rs
    route.rs
    dispatch.rs
  delivery/
    mod.rs
    protocol.rs
    targets.rs
    triggered.rs
  host/
    mod.rs
    builder.rs
    revision.rs
  state/
    mod.rs
    records.rs
    setup.rs
    pairing.rs
    bindings.rs
    dm_targets.rs
```

`channel_routes.rs` remains an HTTP adapter and delegates to setup/pairing
services; handler tests remain caller-level. Files should remain below 1,000 lines unless
the architecture test documents an existing exception. No production-only compatibility
shim remains after all consumers migrate.

### 7. Red-first architecture ratchets

Before each structural move, add or extend architecture tests so the intended target is
red against the PR baseline for the expected reason. Flip each test green only by making
the production change.

Required ratchets:

1. Composition may not own `outbound/channel_delivery.rs`; generic delivery behavior is
   owned by `ironclaw_channel_delivery`.
2. Generic extension lifecycle source may not contain the Telegram product name or a
   Telegram-only status slot.
3. The deleted Telegram trait names may not reappear in production source.
4. Telegram tests may not define `InMemory*Store` domain implementations; they use
   `FilesystemTelegramHostState<InMemoryBackend>` or its erased equivalent.
5. `TelegramPairingStatusResponse` and `ResolvedTelegramIngress` may not reappear.
6. Telegram production files touched by this refactor must remain below the agreed
   1,000-line budget.
7. `ironclaw_channel_delivery` receives an explicit dependency-boundary rule in
   `ironclaw_architecture`; it cannot depend on composition, CLI, WebUI, or concrete
   channel crates.
8. Composition's Telegram module has a narrow source budget and may contain assembly
   symbols only; Telegram behavior identifiers are pinned to the Telegram crate.

Ratchets compare exact paths or symbol sets rather than aggregate counts, so deleting one
violation cannot mask adding another.

## Data and control flow after the refactor

### Inbound Telegram message

```text
composition mounts returned route fragment
  -> Telegram ingress route verifies the current setup revision
  -> Telegram-owned resolver builds or reuses revision workflow
  -> pairing-aware Telegram dispatcher classifies the verified DM
  -> NativeProductAdapterRunner submits through ProductWorkflow
  -> generic delivery observer in ironclaw_channel_delivery watches the run
  -> TelegramDeliveryProtocol renders and sends through mediated egress
```

### Extension activation requiring pairing

```text
generic lifecycle loads installation
  -> account-setup registry lookup by ExtensionId
  -> Telegram registration queries concrete pairing service
  -> disconnected: generic RuntimeCredentialAuthRequirement is returned
  -> blocked run renders Pairing challenge
  -> Telegram pairing consume dispatches LifecycleActivation continuation
  -> resumed activation recomputes status and succeeds
```

### Triggered delivery

```text
trigger poller invokes product-neutral PostSubmitDeliveryHook
  -> keyed hook registry fans out
  -> Telegram-owned hook selects Telegram target provider
  -> generic TriggeredRunDeliveryDriver observes and delivers
  -> outbound state records the honest terminal delivery status
```

## Compatibility and security invariants

The refactor must preserve all of the following:

- Telegram webhook secret verification occurs before admission and remains constant-time.
- Body limits, manifest-projected route policy, per-installation rate limiting, immediate
  acknowledgement, DM-only admission, and bounded concurrency do not change.
- Secret bytes stay inside secret/host-egress mediation and never enter adapter-visible
  state, URLs in composition, logs, or public DTOs.
- Setup compensation and rollback order remains unchanged, including same-bot rotation,
  bot swap, activation failure, record-write failure, and delete-webhook behavior.
- Pairing remains OS-CSPRNG, short-lived, single-use, user-scoped, CAS-safe, and resistant
  to cross-user rebinding.
- Identity binding epoch checks remain live on every message and unpair remains immediate.
- Delivery status remains honest for partial chunks, provider rejection, unauthorized,
  and retryable failures; actionable blocked-run prompts remain deliverable.
- Generic delivery continues to support both Slack and Telegram through
  `ChannelDeliveryProtocol` without channel-name branching.
- No persisted schema, filesystem path, public route, manifest, JSON field, feature flag,
  or environment variable changes.

## Test strategy

### Red/green unit and caller tests

- Add architecture ratchets first and observe each fail for the intended baseline
  violation.
- Migrate Telegram setup/pairing/state tests to the concrete filesystem-backed state.
- Test filesystem failures through a failure-injecting `RootFilesystem` wrapper.
- Test Bot API success/failure through the mediated HTTP seam.
- Keep HTTP handler tests for authorization, redaction, unknown-field rejection, and
  activation rollback.
- Keep webhook route tests for auth, rate limits, malformed input, timeout, dynamic
  reconfiguration, and paired forwarding.
- Keep delivery tests at the real adapter/protocol caller, asserting outbound HTTP request
  and recorded delivery state rather than mock call counts.

### Regression matrix

Run at minimum:

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

Clippy and final safety verification:

```bash
cargo clippy -p ironclaw_channel_delivery --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_channel_host --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_telegram_extension --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
scripts/pre-commit-safety.sh
```

The final report distinguishes checks run successfully from checks unavailable because of
external services. No test may silently skip a missing dependency.

## Material acceptance checklist

This checklist is the durable mapping from the approved design to implementation. The
implementation plan expands each row into test-first steps and updates its checkbox only
after the named evidence is green.

- [x] **A1 — Delivery ownership:** generic live/triggered/actionable delivery behavior is
  owned by `ironclaw_channel_delivery`; its 87 unit tests, public API test, targeted Clippy,
  and Reborn dependency/composition boundary suites pass, and the old composition path is gone.
- [x] **A2 — Concrete Telegram state:** setup, pairing, binding, and DM-target services use
  one filesystem-backed state; the four parallel state-store traits and all Telegram
  `InMemory*Store` fakes are gone. Eight state tests, the 100-test Telegram suite, targeted
  Clippy, the real-state architecture ratchet, and the Telegram composition feature check pass.
- [x] **A3 — Concrete Bot API:** the same-crate Bot API, egress-credential, and installation
  resolver traits are gone. Setup, dispatch, pairing, and egress tests use the concrete client
  over host-mediated HTTP; the 101-test Telegram suite, targeted Clippy, exact deleted-symbol
  scan, and Telegram composition feature check pass.
- [x] **A4 — DTO cleanup:** `TelegramPairingStatusResponse`,
  `ResolvedTelegramIngress`, dead wrappers, and unused public accessors are gone while
  exact connected/pending route JSON tests remain compatible; verified by the 104-test
  Telegram suite and deleted-symbol ratchet in commit `f1668d763`.
- [x] **A5 — Generic lifecycle:** activation uses an `ExtensionId`-keyed account-setup
  registry; generic lifecycle code contains no Telegram policy, and Telegram owns its descriptor,
  pairing requirement, connection projection, activation copy, and connection-status source.
- [x] **A6 — Telegram-owned runtime behavior:** revision caching, dispatcher decoration,
  triggered hook construction, outbound target construction, and Telegram fallbacks live
  in `ironclaw_telegram_extension`; 108 owner tests, 11 Telegram-filtered composition tests,
  targeted Clippy, and the assembly-only ratchet pass in commit `e1f660030`.
- [x] **A7 — Focused files:** setup, pairing, ingress, delivery, state, and host are split by
  responsibility; the physical-file line-budget ratchet passes in commit `e1f660030`.
- [x] **A8 — Ratchets:** all eight structural ratchets are committed and all nine tests in
  `telegram_extension_gates` pass (`219a03f86`, completed by `e1f660030`).
- [x] **A9 — Contract preservation:** 108 Telegram owner tests, 60 adapter tests, 10
  default-off tests, all 1,528 full-feature composition tests, and exact JSON/manifest
  projection tests pass across setup, ingress, pairing, identity, and delivery.
- [x] **A10 — Cross-channel preservation:** 87 shared delivery tests plus its public-API
  contract and the full Slack/Telegram-enabled composition suite pass. Generic dispatch uses
  `ChannelDeliveryProtocol`; compatibility-locked legacy idempotency-key text is not branching.
- [x] **A11 — Quality gates:** all four targeted Clippy commands and workspace-wide
  all-target/all-feature Clippy pass with zero Rust warnings; 51 architecture tests, all 50
  Reborn E2E binaries, production source scans, and baseline-scoped pre-commit safety pass.
- [x] **A12 — Scope audit:** the 102-file owner-move/module-split diff contains no rebase,
  migration, manifest asset, route/wire, secret-handle, persistence-root, frontend, or full URT
  change; contracts and crate guidance name the new owners.

## Rollback strategy

Each workstream lands as a reviewable commit with its tests green. Because wire and
persistence formats do not change, rollback is commit-level: revert the latest structural
slice and restore its previous factory imports. The new crate is not used until the move
commit compiles all consumers, the lifecycle registry preserves the prior error mapping,
and concrete-state migrations retain existing record serializers. There is no data
migration to reverse.
