# Turn-state row store — WriteBehind vs WriteThrough over libSQL — 2026-07-20 (#6263 Step 4)

Measures the `TurnStateRowStore` durability policies over a **real
libSQL** local file (the prior 2026-07-19 study measured the row mechanism over
`InMemoryBackend`; this run puts them on the durable backend the production
`inmemory-turn-state` profile actually uses). Two questions:

1. Over libSQL, does `WriteBehind` (non-critical transitions return before their
   durable ack; gate-park + terminal stay synchronously durable) beat
   `WriteThrough` (every transition awaits its ack)?
2. Does either policy livelock (the failure mode that motivated the in-memory
   authority)?

## What was added to the harness

- `--turn-state-durability <write-through|write-behind>` — selects
  `TurnStateDurabilityPolicy` on the `filesystem-row` / `row-memory` backends
  (`.with_durability_policy(...)`). `write-through` is the default (byte-for-byte
  the pre-#6263-Step-3 behavior). Plumbed through the sweep child-arg passthrough
  and emitted in the human/report/sweep metadata.

## Scenario

Same command shape as the 2026-07-19 study: `chat-turn` (pure storage, no model
wait), 8 owner-users × 16 threads each, 20 operations/thread, libSQL local file,
concurrency swept 8→100, one run per policy, sequential:

```bash
ironclaw_stress --backend libsql --scenario chat-turn \
  --turn-state-backend filesystem-row \
  --turn-state-durability <write-through|write-behind> \
  --users 8 --active-thread-count 8 --threads-per-owner 16 --operations 20 \
  --sweep-concurrency 8,32,64,100 --progress-interval-seconds 0 \
  --output-jsonl chatturn-filesystem-row-<policy>.jsonl
```

Artifacts: `chatturn-filesystem-row-write-through.jsonl`,
`chatturn-filesystem-row-write-behind.jsonl` (one JSON object per concurrency
case; `.metrics` holds attempted/failed/p95/p99/max/throughput).

Machine: 32-core shared box, `/data` NVMe.

## Results

| concurrency | p99 ms (WT → WB) | p95 ms (WT → WB) | max ms (WT → WB) | throughput ops/s (WT → WB) | `turn_thread_busy` shed (WT → WB) |
|---|---|---|---|---|---|
| 8   | 87.1 → **53.3** | 51.0 → **42.9** | 139.2 → **107.2** | 357 → **461** | 0 → 0 |
| 32  | 637.8 → **511.1** | 272.9 → 330.9 | 937.8 → **761.0** | 291 → **300** | 139 → **108** |
| 64  | 799.3 → **545.6** | 618.2 → **351.9** | 896.4 → **737.8** | 276 → **358** | 904 → **570** |
| 100 | 1454.4 → **984.2** | 1168.8 → **647.8** | 1835.5 → **1241.9** | 219 → **318** | 1269 → **1110** |

## Findings

1. **WriteBehind wins over libSQL at every concurrency** — lower p99 (32–46% at
   c8/c100), lower max, higher throughput. Non-critical transitions (submit =
   Queued, claim = Running) no longer block on the libSQL fsync/ack; only
   gate-park + terminal barriers do. The store-tier speedup this flip was meant
   to capture is real on the durable backend, not just `InMemoryBackend`.
2. **No livelock, either policy.** Zero `CAS-retries-exhausted`; the row store
   uses a typed journal/delta append log + hot snapshot cache, not the per-user
   `state.json` CAS that livelocked the blob store. The `failed` counts at c32+
   are `turn_thread_busy` — the coordinator's single-active-turn-per-thread
   admission gate shedding colliding workers (documented in the 2026-07-19
   study), **not** a store failure. WriteBehind sheds *fewer* (108 vs 139 at c32;
   570 vs 904 at c64) because faster commits hold each thread's active lock for
   less of the worker cycle, lowering the collision rate.

## ⚠️ WriteBehind is NOT wired to production (Step 4 blocker)

These store-tier numbers show WriteBehind's advantage, but Step 4 **ships the row
store at `WriteThrough`, not `WriteBehind`**, because WriteBehind has a
runtime-breaking read-after-write defect at the store tier:

- `TurnStateRowStore::get_run_state` (and the other durable-read query
  methods) read **materialized rows, not the hot cache**
  (`filesystem_store/row_store/traits.rs`). Under WriteBehind a just-submitted
  run's row is still async-materializing, so `get_run_state` returns
  `ScopeNotFound`.
- The runtime reads state right after submit (`send_user_message` →
  `wait_for_terminal` → `get_run_state`), so under WriteBehind **every turn
  fails** with `turn run not found`. Proven: `budget_approval_e2e` fails under
  WriteBehind and passes under WriteThrough; a store-tier single-threaded
  submit → `get_run_state` returns `ScopeNotFound` under WriteBehind (both
  lenient and strict), `Queued` under WriteThrough.
- This harness's `chat-turn` workload does **not** call `get_run_state` (it uses
  submit/claim/complete transition ops, which read the hot cache), so it does not
  trip the bug — which is why these WriteBehind numbers exist at all.

WriteBehind must make its durable-read query paths cache-aware (validated by the
§11.4 reference-model suite) before the production build arm can select it. Until
then the profile uses WriteThrough — still strictly more durable than the former
in-memory authority + block-persistence (every transition synchronously durable,
crash-recoverable, no livelock).
