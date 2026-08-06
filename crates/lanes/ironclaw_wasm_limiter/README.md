# ironclaw_wasm_limiter

The `wasmtime::ResourceLimiter` implementation shared by every WASM host in
the workspace, so the tool lane (`ironclaw_wasm`) and the hook engine
(`ironclaw_hooks`) cannot silently diverge on resource limits. It was extracted
from `ironclaw_wasm`'s private `limiter.rs` so the hook crate could depend on
it through Cargo rather than a cross-crate `#[path]` file import — the crate's
entire reason to exist is holding zero internal dependencies while sitting
between two consumers that must not depend on each other.

- **Family / layer:** `lanes` / `runtimes` · **Package:**
  `ironclaw_wasm_limiter` · **Manifest:**
  `crates/lanes/ironclaw_wasm_limiter/Cargo.toml`
- **Use this when:** any wasmtime host in the workspace needs resource
  ceilings — install `WasmResourceLimiter` on the store; do not write a second
  limiter.
- **Don't use this when:** you need store setup, component loading, or
  bindings → those are host-specific and stay with each consumer
  (`ironclaw_wasm`, `ironclaw_hooks`); you need *time* or *fuel* metering →
  fuel/epoch ceilings are configured on the engine/store by the host, not
  here.

## Public surface

One type, `WasmResourceLimiter`:

- `new(memory_limit: u64)` — ceilings fixed at construction: aggregate linear
  memory against `memory_limit`, max 10 tables, 10 instances, 10 memories
  (component-model internals legitimately create several), table growth capped
  at 10,000 entries.
- `memory_used()` / `memory_limit()` — usage accessors.
- `impl wasmtime::ResourceLimiter` — tracks **aggregate** growth across all
  memories (so a multi-memory component cannot multiply its budget), and rolls
  back staged accounting in `memory_grow_failed` when the OS-level grow fails
  after approval, so a retry up to the full ceiling still succeeds.

## Depends on / consumed by

- **Depends on:** nothing in the workspace. External: `wasmtime`, `tracing`
  (denials log a `warn`, allowed grows a `trace`).
- **Consumed by (measured 2026-08-05):** exactly `ironclaw_wasm` (runtimes —
  the family's single same-layer edge) and `ironclaw_hooks` (loops → runtimes,
  legal downward).

## Invariants

- **Zero workspace dependencies**, asserted by
  `wasm_sandbox_core_module_stays_domain_free_v1_parity_kernel` in
  `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs` —
  the crate's only own gate, and it checks *outbound* edges. No `BoundaryRule`
  names `ironclaw_wasm_limiter` as its `crate_name`; inbound consumption is
  governed by the layer ladder plus the same-layer edge inventory
  (`reborn_same_layer_edge_inventory.rs` pins `wasm → wasm_limiter` as the
  only runtimes→runtimes edge).
- **One limiter for all hosts.** The consumers enforce identical limits by
  construction; centralizing the impl is what makes the shared edge visible to
  `cargo check` and the architecture tests (henrypark133 must-fix #1 on
  PR #3634).

## Tests

```bash
cargo test -p ironclaw_wasm_limiter   # 3 unit tests: ceilings, aggregate growth, grow-failure rollback
```

## See also

Family boundary: [`crates/lanes/AGENTS.md`](../AGENTS.md) · design record:
`docs/reborn/target-architecture/families/lanes.md` (§ `ironclaw_wasm_limiter`)
and PROPOSAL §6.6.2 · the consuming hosts: `crates/lanes/ironclaw_wasm`,
`crates/loop/ironclaw_hooks`.
