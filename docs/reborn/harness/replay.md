# Reborn Replay Harness

Reborn should reuse IronClaw's existing replay harness and extend it for Reborn compatibility evidence.

This document describes the target shape. It does not mean every Reborn-specific fixture exists today.

## What already exists

Current replay assets include:

- `tests/fixtures/llm_traces/README.md`;
- `scripts/replay-snap.sh`;
- `scripts/trace-coverage.sh`;
- `.github/workflows/replay-gate.yml`;
- `tests/snapshots/`;
- `tests/support/LIVE_TESTING.md`.

The existing replay model has two layers:

| Layer | Purpose |
| --- | --- |
| JSON fixture | Scripted LLM/provider/tool behavior |
| Snapshot output | Observed agent/runtime behavior |

## Goals

- Make compatibility behavior reviewable as snapshots.
- Let agents validate Reborn changes without live LLM/provider calls.
- Capture #3020 compatibility-gate evidence.
- Capture #3031 product-surface migration evidence.
- Detect drift in tool ordering, approval flow, events, redaction, and process state.

## Target Reborn fixture families

The compatibility gate should eventually include replay coverage for:

- normal chat turn;
- tool call chain;
- approval required -> approve -> resume;
- approval denied;
- auth blocked;
- MCP auth flow;
- WASM capability call;
- script capability call;
- memory read/write/search;
- secret lease usage;
- network policy denial;
- background process spawn/status/result;
- SSE replay cursor;
- WebSocket reconnect;
- routine/job trigger;
- extension install/activate/remove;
- v1 product-surface compatibility cases from #3031;
- #3020 blocking compatibility cases.

## Snapshot review rule

A snapshot diff should answer:

- Which product surface changed?
- Which contract or crate-local boundary allows or forbids the change?
- Is this expected migration drift or a regression?
- Does the change affect #3020 compatibility?
- Does the change affect #3031 product-surface parity?
- Are sensitive fields redacted?

## Determinism requirements

Replay fixtures should avoid:

- wall-clock time;
- random IDs unless injected;
- live HTTP;
- live OAuth;
- live LLM calls;
- environment-dependent filesystem listings;
- unseeded memory/search state.

Replay fixtures should prefer:

- fixed IDs;
- fake providers;
- recorded HTTP exchanges;
- deterministic tool outputs;
- explicit memory setup;
- local JSONL event/audit sinks;
- redacted snapshots.

## Event coverage

The existing `scripts/trace-coverage.sh` reports event coverage for current snapshots.

A future Reborn-specific coverage script can build on the same pattern once Reborn durable event types and snapshots are stable.
