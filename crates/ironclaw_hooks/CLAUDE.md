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
