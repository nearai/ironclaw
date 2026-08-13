# ironclaw_hooks

The trust-tiered hook framework: four trust classes fixed by a hook's *source*
(never declarable by the hook), sealed decision sinks, ordering and
failure-policy rules, a declarative predicate language, a sandboxed WASM hook
engine, and decorator implementations of the full `ironclaw_loop_contracts`
port set — the outermost layer of the family's declared `Loop*Port` chain, so
every port call is policy-checked and logged before it reaches the runner's
composition and `ironclaw_loop_host`'s kernel-facing base.

- **Family / layer:** `loop` / `loops` · **Package:** `ironclaw_hooks` · **Manifest:** `crates/loop/ironclaw_hooks/Cargo.toml`
- **Use this when:** changing hook trust tiers, decision kinds, dispatch
  ordering, failure policy, the predicate language, or the hook middleware.
- **Don't use this when:** implementing a *base* (non-decorating) port → use
  `ironclaw_loop_host`; sourcing or installing hook code → the extension
  family owns loading; changing the driver contract → it lives in
  `ironclaw_loop_contracts`, not here.

## Public surface

- `HookDispatcher` (`dispatch/`) with tier-specific installers
  (`install_builtin_*` / `install_trusted_*` / `install_installed_*`) — the
  only public path a hook implementation enters by; wrong-tier impls are
  unmintable at the type level.
- `HookRegistry`, `HookBinding`, `HookRegistrar`; `HookTrustClass` (builtin /
  trusted / installed / self-authored); the sealed sink traits
  (`Privileged*`/`Restricted*` gate & mutator sinks, `ObserverSink`).
- `middleware/` — the `HookedLoop*Port` decorators over every loop port.
- `predicate/` + `predicate_state/` with `libsql_backend` / `postgres_backend`
  — the predicate store and its durable backends (complete but **unwired**:
  composition currently hard-codes the in-memory backend; ADR 0004).
- `wasm/` — the sandboxed engine for portable hook code (via
  `ironclaw_wasm_limiter`).

## Depends on / consumed by

- **Normal workspace deps (5):** `ironclaw_event_log`, `ironclaw_host_api`,
  `ironclaw_loop_contracts`, `ironclaw_prompt_envelope`,
  `ironclaw_wasm_limiter`. Plus direct `libsql` / `tokio-postgres` drivers for
  the predicate backends — the documented second exception to the filesystem
  persistence idiom (`docs/internal/adr/0004-hooks-keeps-its-predicate-state-backends.md`).
- **Consumed by (2):** `ironclaw_turn_runner` (installs the middleware into
  each claimed run's host) and `ironclaw_composition` (loads/wires).
- **Never depends on:** `ironclaw_turns` — the dependency direction is the
  point: nothing in the turn-admission kernel depends on hooks, and hooks no
  longer names the kernel at all.

## Invariants

- A hook cannot grant authority, cannot bypass authorization / approvals /
  runtime policy / resource / secrets / filesystem / network policy, and never
  receives an ambient secret, filesystem handle, network client, or process
  handle.
- Gate and mutator decisions **fail closed**; observer and effect decisions
  **fail isolated** with redacted audit; a protocol-violating hook is barred
  (slot-poisoned) for the rest of the run — and the dispatcher is minted fresh
  per host build, so poison does not leak across runs
  (`tests/integration/hooks.rs::poisoned_hook_slot_does_not_leak_into_the_next_run`).
- Trust class is fixed by source; a manifest `[[hooks]]` section can describe
  a hook but cannot claim a class above `Installed`.
- The `BoundaryRule` forbids runtime adapters and dispatcher concretions
  (`--test reborn_dependency_boundaries reborn_crate_dependency_boundaries_hold`);
  the persistence-idiom rule tracks the driver exception
  (`--test reborn_persistence_driver_boundary`).

## Tests

```bash
cargo test -p ironclaw_hooks
cargo test -p ironclaw_architecture_tests    # after dependency/API changes
```

## See also

Family rules: `crates/loop/AGENTS.md` · working rules: `AGENTS.md` beside this
file · design record: `docs/internal/reborn/target-architecture/families/loop.md`
(§6.7.4) + ADR 0004.
