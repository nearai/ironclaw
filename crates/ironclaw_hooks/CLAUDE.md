# ironclaw_hooks — Reborn loop hook framework

This crate owns the contract for inline (before-behavior) and event-triggered (after-fact)
hooks across the Reborn loop. It does not own:

- The runner-facing `AgentLoopDriver` trait — that stays in `ironclaw_turns`.
- The concrete `LoopCapabilityPort` / `LoopPromptPort` / `LoopModelPort` impls —
  those stay in `ironclaw_loop_support` and `ironclaw_reborn`.
- The Reborn-side middleware composition that wraps host ports — that lives in
  `ironclaw_reborn::loop_driver_host` and consumes types from this crate.
- WASM hook execution. Programmatic hooks will run inside `wasmtime` via a sink
  exposed by the dispatcher; the actual wasm runtime integration is a follow-up.

## Dependency direction

```
ironclaw_turns       -> no dependency on ironclaw_hooks
ironclaw_hooks       -> depends on ironclaw_turns + ironclaw_host_api
ironclaw_reborn      -> depends on ironclaw_hooks for host composition (follow-up)
ironclaw_engine      -> no hook ownership; optional future driver consumer
```

Architecture test in `ironclaw_architecture::tests::reborn_dependency_boundaries`
proves the `ironclaw_turns -> ironclaw_hooks` edge stays absent.

## Trust model

Hooks have three trust classes and the framework enforces the differences
*at the type level*, not by convention:

- **Builtin** — compiled into IronClaw, identity = crate path + symbol. May
  produce any decision kind via `BuiltinHookSink`.
- **Trusted** — user-placed in `~/.ironclaw/hooks/` or workspace `hooks/`. Cannot
  register at `runtime`-class points (e.g., the inner side of capability
  attenuation). Uses `TrustedHookSink`.
- **Installed** — extension registry, eventually WASM-hosted. Restricted to
  `Observer` and `Effect` kinds by default; `Gate` and `Mutator` require an
  explicit per-extension grant. Uses `InstalledHookSink`, which exposes only
  monotonic-restriction constructors. An `Installed` hook cannot mint
  `Decision::Allow` — that variant is not reachable from the sink trait.

Trust class is *fixed by source*, never declarable. The extension manifest's
`[[hooks]]` section can describe the hook but cannot claim a trust class higher
than `Installed`. The registry installer is the only thing that decides
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
`ironclaw_reborn` (or any other internal crate) reads a hook from the
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
`dispatch.rs` is the regression guard that flags such changes.

## Non-negotiable invariants

- Hooks cannot grant authority.
- Hooks cannot bypass authorization, approvals, runtime policy, resource policy,
  secrets policy, filesystem policy, or network policy.
- Hooks cannot receive ambient secrets, filesystem handles, network clients,
  process handles, or raw runtime authority.
- Hook side effects must route through existing `HostRuntime` / capability
  dispatch paths.
- Inline hooks run before behavior and may block/change behavior.
- Event hooks run after durable facts and must not retroactively deny completed
  behavior.
- `Gate` / `Mutator` hooks fail closed.
- `Observer` / `Effect` hooks fail isolated with redacted audit.
- All model-visible hook output is bounded, typed, redacted/trust-labeled, and
  envelope-wrapped when untrusted (reuses the prompt envelope from
  `ironclaw_host_runtime::memory_context` once that helper is extracted).
- A hook that demonstrates protocol violation (timeout, panic, malformed
  decision) gets its slot poisoned for the rest of the current turn run.

## Module layout

- `identity` — `HookId`, `HookVersion`, content-addressed component identity
- `trust` — `HookTrustClass` enum + attenuation rules
- `error` — `HookError` thiserror
- `points/` — typed contexts the dispatcher hands hooks (`capability`,
  `prompt`, `observer`)
- `kinds/` — sealed decision types (`gate`, `mutator`, `observer`); only the
  dispatcher and matching hook sinks can mint them
- `sink` — `BuiltinHookSink` / `TrustedHookSink` / `InstalledHookSink`
- `ordering` — `HookPhase`, `HookPriority`, stable composition
- `failure_policy` — `FailureCategory` taxonomy and per-kind behavior
- `registry` — `HookRegistry`, `HookBinding`, run-profile-sourced resolution
- `dispatch` — `HookDispatcher` executor contract (will be wrapped by Reborn
  middleware in a follow-up)
- `manifest` — extension manifest `[[hooks]]` schema (serde types)
- `predicate` — declarative predicate language for `Installed` hooks (types
  only; evaluation lives in the dispatcher)

## Known deferred work

- **Dispatcher-per-build (tenant + run isolation).** Poison state and the
  registry today live inside the dispatcher and persist for the lifetime of
  the dispatcher instance. This is intentional in the current slice: a hook
  that demonstrates protocol violation stays disabled until the process
  restarts, which is the conservative default. The
  `PredicateEvaluator`'s sliding-window counter is keyed by
  `(hook_id, tenant_id, capability)` so rate-cap state is correctly
  partitioned across tenants. What remains is the broader pattern of
  building a fresh dispatcher per run (or per tenant) so that resume
  semantics, replay, and full cross-tenant isolation of mutable hook state
  are first-class. Tracked as a follow-up.
