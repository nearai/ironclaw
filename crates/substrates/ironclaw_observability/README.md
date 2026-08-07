# ironclaw_observability

Zero-cost-when-off latency-trace macros over the `ironclaw_latency` tracing
target, shared by every crate that wants to time an operation without adopting
a tracing dependency of its own. ~90 lines, seven consumers, exactly **one**
dependency — and that count is the crate's enforcement mechanism: a change
that needs a second dependency is a change that belongs somewhere else.

- **Family / layer:** `substrates` / `substrates` · **Package:**
  `ironclaw_observability` · **Manifest:**
  `crates/substrates/ironclaw_observability/Cargo.toml`
- **Use this when:** you want `live_latency_trace!`-style timing that costs
  nothing while the `ironclaw_latency` target is disabled.
- **Don't use this when:** you're producing a *value* a trace merely records
  (byte counts, sizes) → that measurement belongs to whoever produces the
  thing being measured (PROPOSAL §12.12 D-K); you need sinks, exporters, or
  state → no such thing lives here.

## Public surface

`live_latency_trace!`, `live_latency_trace_ok!`, `live_latency_trace_error!`,
plus `elapsed_ms`, `live_latency_enabled`, `live_latency_started_at`, and the
`pub use tracing` facade (a deliberate macro-hygiene tradeoff). No traits.

## Depends on / consumed by

- **Depends on:** nothing in the workspace; external: `tracing` only.
- **Consumed by (measured 2026-08-05):** 7 — `ironclaw_filesystem`,
  `ironclaw_host_runtime`, `ironclaw_loop_host`, `ironclaw_turn_runner`,
  `ironclaw_turns`, `ironclaw_composition`, `ironclaw_extension_support`.

## Invariants

- **One dependency, and it stays that way** — the manifest is the charter made
  mechanical; the story of the `serde_json` eviction and the condition under
  which it would be revisited is in [`AGENTS.md`](./AGENTS.md).
- **Zero-cost-when-off covers the trace, not the fields** — guard expensive
  field computation with `live_latency_enabled()` first (see `AGENTS.md` for
  the caller-side shape).

## Tests

```bash
cargo test -p ironclaw_observability   # 2 tests: elapsed_ms clamps; disabled without a subscriber
```

## See also

Working rules and the full D-K rationale: [`AGENTS.md`](./AGENTS.md)
(canonical). Family boundary: [`crates/substrates/AGENTS.md`](../AGENTS.md).
Design record: PROPOSAL §6.2.5.
