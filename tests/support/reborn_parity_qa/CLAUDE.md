# Reborn Parity/QA Test Support

Support tree for the **parity and QA suites** (`tests/reborn_*_parity.rs`,
`tests/reborn_qa_*.rs`, `tests/reborn_*_e2e.rs`). These suites are current and
maintained, but they are **not coverage-bearing** — the coverage program runs
exclusively over `tests/integration/` (see `tests/integration/CLAUDE.md`).

## Tier

`RebornBinaryE2EHarness` (in `binary_e2e.rs`) swaps the whole
`HostManagedModelGateway` with `RebornTraceReplayModelGateway`
(`model_replay.rs`) at the *gateway* seam, skipping `ironclaw_llm`. The
integration tier in `tests/integration/` mocks one layer lower (the vendor-SDK
seam) so the real decorator chain runs; prefer that tier for new
coverage-bearing scenarios.

## Files

- `binary_e2e.rs` — `RebornBinaryE2EHarness` + `SubmittedTurn` +
  `RebornHarnessSharedStorage` + `assert_milestone_order` /
  `trace_tool_call_response`. Drives the product caller path (inbound bytes →
  ProductAdapter → workflow → coordinator → scheduler → loop) with trace-replay
  model + recording capability port.
- `model_replay.rs` — `RebornTraceReplayModelGateway` and trace-replay step
  types.
- `qa_trace.rs` — recorded-behavior QA trace tooling (sole consumer
  `reborn_qa_recorded_behavior`).
- `qa_scenarios.rs` — QA scenario coverage ledger (sole consumer
  `reborn_qa_smoke_scenarios_e2e`).
- `delivery.rs` — `RecordingOutboundDeliverySink` (channel-delivery QA +
  outbound reply-target parity).
- `network.rs` — `RecordingNetworkHttpTransport` (doc-grounding / web-fetch QA).

## Module paths

Each consuming bin mounts BOTH trees:

```rust
#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;
```

Import the binary-E2E family from `parity_qa_support::…`; the shared adopted
core (`config`, `test_adapter`, `session_thread`, `harness` doubles,
`filesystem`, `product_surface`, …) stays under `reborn_support::…`.

## Direction invariant (CI-enforced)

`parity_qa_support` imports FROM `tests/integration/support/` — never the
reverse. Nothing under `tests/integration/` may reference
`reborn_parity_qa` or `parity_qa_support`.
