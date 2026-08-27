# Agent Map — ironclaw_hooks

Working rules for the Reborn loop hook framework. Orientation lives in
`README.md`; family rules in `crates/loop/AGENTS.md`.

This crate owns the contract for inline (before-behavior) and event-triggered
(after-fact) hooks across the Reborn loop, and the `HookedLoop*Port`
middleware decorators (`src/middleware/`) that wrap the composed host
outermost. It does not own:

- The runner-facing `AgentLoopDriver` trait — that stays in
  `ironclaw_loop_contracts` (`src/driver.rs`).
- The concrete *base* (non-decorating) `Loop*Port` implementations — those
  stay in `ironclaw_loop_host`. This crate only ever wraps another
  implementation.
- The middleware *installation* into a claimed run's host — that lives in
  `ironclaw_turn_runner`'s loop-host composition, which consumes types from
  this crate.
- Extension bundle loading and installation. Installed-tier WASM hooks execute
  here once their module bytes are resolved, but the extension installer
  remains the authority for sourcing those bytes.

## Dependency direction

Measured normal workspace deps (5): `ironclaw_event_log`, `ironclaw_host_api`,
`ironclaw_loop_contracts`, `ironclaw_prompt_envelope`,
`ironclaw_wasm_limiter` — plus direct `libsql`/`tokio-postgres` drivers for
the predicate-state backends (ADR 0004, the documented persistence exception).

```
ironclaw_turns        -> no dependency on ironclaw_hooks (and none back: this
                         crate does not name the kernel at all)
ironclaw_hooks        -> loop_contracts + host_api + event_log +
                         prompt_envelope + wasm_limiter
ironclaw_turn_runner  -> depends on ironclaw_hooks (installs the middleware
                         into each claimed run's host)
ironclaw_composition  -> depends on ironclaw_hooks (loads/wires)
```

The `ironclaw_hooks` `BoundaryRule` in
`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`
forbids runtime adapters and dispatcher concretions (that is what keeps the
framework from acquiring authority it should not have), and the kernel's own
rules keep the `ironclaw_turns -> ironclaw_hooks` edge absent.

## Trust model

Hooks have **four** trust classes; the framework enforces the differences
*at the type level*, not by convention. The first three are loadable from
an external source; the fourth is run-scoped only.

- **Builtin** — compiled into IronClaw, identity = crate path + symbol. May
  produce any decision kind: installs through the *privileged* hook traits
  (`install_builtin_*` takes `Privileged*Hook`), whose sinks
  (`PrivilegedGateSink` / `PrivilegedMutatorSink`) expose `allow`.
- **Trusted** — user-placed in `~/.ironclaw/hooks/` or workspace `hooks/`.
  Cannot register at `runtime`-class points (e.g., the inner side of
  capability attenuation). Also installs through the *privileged* traits —
  this tier's restriction is on registration points, not on the sink.
- **Installed** — extension registry, eventually WASM-hosted. Restricted to
  `Observer` and `Effect` kinds by default; `Gate` and `Mutator` require an
  explicit per-extension grant. Installs through the *restricted* traits
  (`install_installed_*` takes `Restricted*Hook`), whose sinks
  (`RestrictedGateSink` / `RestrictedMutatorSink`) omit `allow` entirely. An
  `Installed` hook cannot mint `Decision::Allow` — that method is not on the
  sink trait.
- **SelfAuthored** — the agent authors a hook for the current run via
  `SelfAuthoredEvaluator` (typically after user ratification). The sink
  (`SelfAuthoredHookSink`) is monotonic-restriction only: no `Allow`, no
  `Effect`. **Run-scoped only**: the dispatcher discards self-authored
  hooks at run end; durable persistence requires the channel-to-user path
  tracked at #3567. This tier exists in the trust enum + threat model but
  has no manifest representation and is not loadable from an external
  source.

Trust class is *fixed by source*, never declarable. The extension manifest's
`[[hooks]]` section can describe the hook but cannot claim a trust class
higher than `Installed`. The registry installer is the only thing that decides
classification, and it does so based on where the hook came from.

## Loader responsibility

The tier-specific installers on `HookDispatcher`
(`install_builtin_*` / `install_trusted_*` / `install_installed_*`) are the
*only* public path through which a hook implementation enters the dispatcher.
The `BeforeCapabilityHookImpl::{Privileged, Restricted}` variants are sealed
`pub(crate)`, so no external caller can mint a wrong-tier impl: it is a
type-level fact that an `Installed`-tier installer cannot accept a
`PrivilegedBeforeCapabilityHook`.

What the type system **does not** enforce is *origin*. If loader code inside
`ironclaw_turn_runner` (or any other internal crate) reads a hook from the
extension registry and accidentally routes it through
`install_builtin_before_capability`, the trust-class ↔ impl-tier pairing at
the registry-binding boundary breaks — the dispatcher will happily install
a registry-sourced hook as a Builtin. The tier-specific installers prevent
*minting* a wrong-tier impl, but they cannot enforce that the loader picked
the right installer for the hook's actual source.

That responsibility lives with the **loader** — the code that constructs the
dispatcher and calls `install_*`. The contract is:

- A loader **must** match the installer to the hook's *source*, not just to
  its declared capability.
- A loader **must not** select an installer based on manifest claims; the
  trust class is fixed by where the hook came from (built-in code path /
  user filesystem / extension registry).
- Registry-loaded extension hooks **should** be type-tagged at the loader
  level — e.g., a `LoadedHook::Installed(Box<dyn RestrictedBeforeCapabilityHook>)`
  enum produced by the registry loader — so that a loader can never call
  `install_builtin_*` with installed-sourced code. The compiler then enforces
  the origin → installer mapping at the loader's own seams.

If the dispatcher's install API changes in the future (new installer, renamed
method, additional trust tier), the loader contract must be re-evaluated:
the `tier_specific_installers_are_documented_as_loader_contract` test in
`src/dispatch/mod.rs` is the regression guard that flags such changes.

## Non-negotiable invariants

- Hooks cannot grant authority.
- Hooks cannot bypass authorization, approvals, runtime policy, resource
  policy, secrets policy, filesystem policy, or network policy.
- Hooks cannot receive ambient secrets, filesystem handles, network clients,
  process handles, or raw runtime authority.
- Hook side effects must route through existing mediated paths: `HostRuntime`
  / capability dispatch, or — for `Lifecycle` hooks only (amended 2026-08-20,
  #7770 phase 1) — prepared-context turn submission
  (`accept_prepared_context` → `TurnCoordinator::submit_turn`), which is
  admission-checked at accept and capability-authorized at execution. A
  lifecycle hook starting an unbound run is mediated work, not ambient
  authority; every capability that run invokes still crosses the normal
  authorization stages.
- Inline hooks run before behavior and may block/change behavior.
- Event hooks run after durable facts and must not retroactively deny
  completed behavior.
- `Gate` / `Mutator` hooks fail closed.
- `Observer` / `Effect` hooks fail isolated with redacted audit.
- All model-visible hook output is bounded, typed, redacted/trust-labeled, and
  envelope-wrapped when untrusted (reuses the extracted
  `ironclaw_prompt_envelope` crate, which this crate already depends on).
- A hook that demonstrates protocol violation (timeout, panic, malformed
  decision) gets its slot poisoned for the rest of the current turn run.
  Poison is therefore RUN-scoped, and every consumer must hold a dispatcher
  *factory* rather than a dispatcher: the `after_turn` seam mints one per
  terminal run (`RebornTurnRunExecutor::with_after_turn_hook_dispatcher_factory`),
  exactly as the loop middleware mints one per host build. A process-lifetime
  dispatcher would silently turn one bad run into a hook that stays dead until
  the process restarts.
- **The tier-specific installer *names* are load-bearing. Do not collapse the
  trust-class x hook-point installers into one generic,
  trust-class-parameterized installer.**
  `composition_crate_installs_installed_tier_only_through_registrar`
  (`crates/app/ironclaw_architecture_tests/tests/reborn_composition_boundaries.rs`)
  is a *source-text* scan, not a type check: it asserts that
  `ironclaw_composition` contains none of `install_installed_*`,
  `install_observer(`, `insert_binding(`, or `HookTrustClass::Installed`.
  Rename or delete one of those names and the assertion passes **vacuously** —
  the registrar-only invariant stops being enforced with every test still
  green. The test's own doc comment names the generic installer as the
  evasion it exists to forbid. The `arch-exempt: too_many_args, needs
  HookInstallContext aggregation, plan #4088` waivers in `dispatch/mod.rs`
  apply **only** to the two already-generic installers (`install_observer`,
  `install_event_triggered`), whose names the scan still matches after
  aggregation — they are not license to flatten the tier-specific ones.
  Re-verify: `cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries composition_crate_installs_installed_tier_only_through_registrar`

## Module layout

- `identity` — `HookId`, `HookVersion`, content-addressed component identity
- `trust` — `HookTrustClass` enum + attenuation rules
- `error` — `HookError` thiserror
- `points/` — typed contexts the dispatcher hands hooks (`capability`,
  `prompt`, `observer`)
- `kinds/` — sealed decision types (`gate`, `mutator`, `observer`); only the
  dispatcher and matching hook sinks can mint them
- `sink` — the trust-tiered sink/hook traits (`PrivilegedGateSink` /
  `RestrictedGateSink` / `PrivilegedMutatorSink` / `RestrictedMutatorSink` /
  `ObserverSink`, and the matching `*BeforeCapabilityHook` /
  `*BeforePromptHook` / `ObserverHook` / `EventTriggeredHook`)
- `ordering` — `HookPhase`, `HookPriority`, stable composition
- `failure_policy` — `FailureCategory` taxonomy and per-kind behavior
- `registry` / `registrar` — `HookRegistry`, `HookBinding`,
  run-profile-sourced resolution, `HookRegistrar`
- `dispatch` — `HookDispatcher` executor contract
- `middleware` — the `HookedLoop*Port` decorators over the loop ports
- `manifest` — extension manifest `[[hooks]]` schema (serde types)
- `predicate` / `predicate_hash` / `evaluator` — declarative predicate
  language for `Installed` hooks and its evaluator
- `predicate_state` + `libsql_backend` / `postgres_backend` — the predicate
  store and its durable backends (complete but unwired; ADR 0004)
- `installed_hook` / `self_authored` / `telemetry` / `wasm` — installed-tier
  execution, the self-authored evaluator, milestone telemetry, and the
  sandboxed WASM engine

## Dispatcher-per-build (per-run isolation)

The `HookDispatcher` owns mutable state — most importantly the registry's
slot-poisoning bits — that must not survive across host builds. The Reborn
factory (`RebornLoopDriverHostFactory::with_hook_dispatcher_factory`) takes a
`Fn + Send + Sync + 'static` closure returning `Arc<HookDispatcher>`; it is
invoked exactly once per `build_text_only_host*` call, so the dispatcher —
and its poison state — is scoped to one run. Any state the closure captures
(template registry, milestone-sink template, feature flags) is fine to share
across calls; the dispatcher instance itself is not. See the doc comment on
`RebornLoopDriverHostFactory::with_hook_dispatcher_factory`
(`crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs`) for the current
signature and usage notes.

The legacy `with_hook_dispatcher(Arc<HookDispatcher>)` adapter is an explicit
opt-in that preserves the old shared-state semantic for backward compat: it
wraps the supplied `Arc` in a closure that returns clones of the same `Arc`,
so a hook poisoned in run N stays poisoned for run N+1. New call sites should
use `with_hook_dispatcher_factory` for real per-run isolation.

Cross-run isolation is pinned by
**`poisoned_hook_slot_does_not_leak_into_the_next_run`** in
`tests/integration/hooks.rs` ([#6945](https://github.com/nearai/ironclaw/issues/6945),
[ADR 0004](../../../docs/internal/adr/0004-hooks-keeps-its-predicate-state-backends.md)).
Poisoning *within* one dispatcher is separately pinned by
`dispatch/mod.rs::poisoned_during_dispatch_skips_subsequent_invocations`.
Predicate-counter state (keyed `(hook_id, tenant_id, capability)`) is
deliberately **not** asserted isolated across runs — it is tenant-scoped and
shared by design; a test asserting isolation there would pin a rate-cap
bypass.

## Validation

- Fast local check: `cargo test -p ironclaw_hooks`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`
