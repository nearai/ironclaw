# `ironclaw_observability` — latency-trace macros, and nothing else

Target-architecture entry: PROPOSAL §6.2.5, `families/substrates.md`.

Zero-cost-when-off latency-trace macros over the `ironclaw_latency` tracing
target. 90 lines, seven consumers, **one dependency**.

## The charter, stated as a test you can apply

> Everything here is either a macro or a helper the macros need.

Public surface: `live_latency_trace!`, `live_latency_trace_ok!`,
`live_latency_trace_error!`, plus `elapsed_ms`, `live_latency_enabled`,
`live_latency_started_at`, and the `pub use tracing` facade (a deliberate
macro-hygiene tradeoff, so a consumer can use the macros without adding a
`tracing` import of its own).

**Never contains:** state, policy, sinks — and, the one that is easy to get
wrong, *a function that merely produces a value a trace happens to record*.
That measurement belongs to whoever produces the thing being measured.

## Why the second dependency is the tripwire

The crate had `serde_json` for exactly one function, `json_value_bytes`, which
counted the serialized size of a JSON value. It read like an observability
helper and was not one: of its five call sites in
`ironclaw_extension_support`, three fed `ResourceUsage::set_output_bytes` —
**resource accounting**, not a trace field.

Sharing it also bought no invariant. `output_bytes` is measured three
different ways in production today — that counter, `output.stdout.len()` in
`ironclaw_scripts`, and `Value::to_string().len()` in `ironclaw_loop_host` —
because each producer measures what *it* produced. So the function moved to
its two consumers (WS6, PROPOSAL §12.12 D-K) and `serde_json` left with it.

**If a change here needs a second dependency, that is the signal the thing
being added is not this crate's job.** The alternatives considered and
rejected are recorded in §12.12 D-K: `ironclaw_common` (the crate the
restructure is actively narrowing) and `ironclaw_host_api` (the contracts
leaf already criticised for carrying behavior).

The ruling is not unconditional, and the condition is written down so it can
be checked rather than re-argued: it holds at **two** copies. If a third
consumer needs that byte counter, the duplication argument flips and D-K
should be revisited — do not simply add a third copy, and do not resolve it
by moving the function back here.

## Consumers

`ironclaw_filesystem`, `ironclaw_host_runtime`, `ironclaw_loop_host`,
`ironclaw_turn_runner`, `ironclaw_turns`, `ironclaw_composition`,
`ironclaw_extension_support`. Every one of them gets whatever this crate
depends on, which is the whole reason the dependency list is the enforcement
mechanism and this file is only the explanation.

## Zero-cost-when-off, and where it is the caller's job

`live_latency_started_at()` returns `None` when the target is disabled, and
every macro is a no-op on `None`. That covers the *trace*, not the *fields*:
a caller that computes an expensive field before checking is paying for it
with tracing off. Guard the computation, not just the emission — see
`ironclaw_host_runtime::latency::RuntimeLatencyFields::from_json_input` for
the shape (`live_latency_enabled()` first, then the measurement).

## Tests

`cargo test -p ironclaw_observability`. Two, both about the properties above:
`elapsed_ms` clamps rather than wraps (a wrapped duration reads as a *fast*
operation), and the enabled-check is false with no subscriber installed.
