# Turn-state row store vs direct in-memory authority — 2026-07-19 (#6263 Phase 1)

Measures whether `FilesystemTurnStateRowStore` (typed journal/delta rows +
process-local hot snapshot cache) meets the latency envelope of the direct
`InMemoryTurnStateStore` authority under the same-user contention scenario from
`results/2026-06-30-turn-state-inmemory/`. Two questions:

1. Does `filesystem-row` avoid the blob store's CAS collapse?
2. Is `row-memory` (the row store over an in-memory backend — the proposed
   replacement for the direct authority in the `inmemory-turn-state` profile)
   close enough to `memory` to justify the swap?

## What was added to the harness

- `--turn-state-backend row-memory` — the same `FilesystemTurnStateRowStore`
  the `filesystem-row` variant wires (shared per-tenant/user store cache, same
  `.with_limits(...)`), but scoped over one shared in-process
  `ironclaw_filesystem::InMemoryBackend` instead of the durable libSQL root.
  This models replacing the direct `InMemoryTurnStateStore` authority with the
  row-store mechanism while removing durable-backend cost.
- The per-scope row-store cache lookup was factored into one
  `cached_row_store` helper shared by `filesystem-row` and `row-memory`.

`filesystem-row` (row store over the durable libSQL scoped filesystem) already
existed in the harness; `memory` and `filesystem` are unchanged.

## Scenario

Identical command shape to the 2026-06-30 study: `chat-turn` (pure storage, no
model wait), 8 owner-users × 16 threads each, 20 operations/thread, libSQL
local file, concurrency swept 8→100, one run per backend, sequential:

```bash
ironclaw_stress --backend libsql --scenario chat-turn \
  --turn-state-backend <memory|row-memory|filesystem-row|filesystem> \
  --users 8 --active-thread-count 8 --threads-per-owner 16 --operations 20 \
  --sweep-concurrency 8,32,64,100 --progress-interval-seconds 0 \
  --output-jsonl chatturn-<variant>.jsonl
```

## Baseline shift vs the 2026-06-30 study — read this first

The June numbers (memory ≈130 ms p99, ~0% failures; blob filesystem livelocking
at 42–86 s with CAS-retries-exhausted) do **not** reproduce on this machine,
for either side. Today every backend — including `memory` — sheds 16–87% of
operations as `turn_thread_busy` at c32+. This is the coordinator's
single-active-turn-per-thread admission gate, not a store failure: with 8
owners × 16 threads and up to 100 workers, workers race onto the same thread,
and the faster the stack, the larger the fraction of each worker cycle the
thread's active lock is held, so the collision rate explodes. Two controls pin
this down (artifacts `chatturn-*-junetree.jsonl`, built from the June study's
commit `4f7832b94` and run on this box):

- June-tree binary, `memory`, c100: 10.2% busy-shed, p99 764 ms — already far
  from the June README's 0% / 128 ms, so the June numbers were
  environment-bound (slow 4-core container ⇒ low thread-lock occupancy ⇒ no
  collisions ⇒ deep same-user CAS pileup for the blob store).
- June-tree binary, blob `filesystem`, c100: p99 2.48 s, max 6.26 s, 47%
  busy-shed, zero CAS-retries-exhausted — the June livelock signature does not
  reproduce here either, because busy-shedding caps concurrent same-user
  writers before the CAS budget can exhaust.

Consequences for reading the tables below: (a) cross-run comparisons are valid
within this study but not against the June absolute numbers; (b) op-level
latency percentiles include busy-rejected ops (which fail fast), so the
store-isolated stage table is the clean per-store signal.

## Results — op level (p99 → max, fail%, succeeded ops/s)

Failures are overwhelmingly `turn_thread_busy` admission rejections (see
breakdown below); ops/s counts succeeded operations only.

| Concurrency | memory | row-memory | filesystem-row | filesystem (blob) |
| ---: | ---: | ---: | ---: | ---: |
| 8   | 58ms → 81ms (0%), 1088/s | 33ms → 42ms (0%), 632/s | 98ms → 123ms (0%), 371/s | 92ms → 100ms (0%), 404/s |
| 32  | 78ms → 135ms (16.1%), 875/s | 91ms → 158ms (20.0%), 736/s | 438ms → 658ms (18.0%), 321/s | 617ms → 978ms (26.1%), 168/s |
| 64  | 124ms → 163ms (34.1%), 646/s | 317ms → 420ms (62.4%), 264/s | 531ms → 650ms (84.8%), 64/s | 830ms → 1.27s (41.9%), 118/s |
| 100 | 175ms → 251ms (44.5%), 509/s | 510ms → 545ms (71.3%), 168/s | 873ms → 1.04s (87.2%), 51/s | 1.73s → 2.93s (55.7%), 81/s |

## Results — store-isolated (turn-store stage time per op: submit + claim + complete)

From `operation_attribution.turn_store` in each JSONL row; this excludes the
thread-service writes that dominate op time for the fast backends.

| Concurrency | memory p50 / p99 | row-memory p50 / p99 | filesystem-row p50 / p99 | filesystem p50 / p99 |
| ---: | ---: | ---: | ---: | ---: |
| 8   | 18µs / 68µs | 4.9ms / 6.2ms | 7.6ms / 39.7ms | 3.9ms / 34.7ms |
| 32  | 21µs / 121µs | 4.0ms / 13.6ms | 11.9ms / 410ms | 31.7ms / 305ms |
| 64  | 21µs / 42µs | 33.1ms / 308ms | 109ms / 504ms | 61.6ms / 522ms |
| 100 | 20µs / 37µs | 91.2ms / 501ms | 180ms / 838ms | 38.4ms / 1.13s |

## Failure-class breakdown at c100

| Backend | thread_busy | conflict | claim_miss | unavailable | store-level total |
| --- | ---: | ---: | ---: | ---: | ---: |
| memory | 889 | 0 | 0 | 0 | 0 (0%) |
| row-memory | 1365 | 54 | 6 | 1 | 61 (3.1%) |
| filesystem-row | 1638 | 102 | 2 | 2 | 106 (5.3%) |
| filesystem (blob) | 1114 | 0 | 0 | 0 | 0 (0%) |

`turn_thread_busy` is the admission gate and hits every backend; the
conflict/claim-miss/unavailable classes appear only on the row-store variants
(plus 8 `turn_scope_not_found` for filesystem-row at c64) — the row store
introduces store-level errors under concurrency that neither the direct
in-memory authority nor the blob store produce.

## Verdict

**filesystem-row vs the blob store:** no livelock, but not flat either. The
row store keeps c100 p99 sub-second (873 ms vs the blob's 1.73 s; worst case
1.04 s vs 2.93 s) and its store-isolated tail is better at every concurrency
above 8. However, the June livelock signature could not be re-triggered on this
machine even for the blob store (busy-shedding caps same-user writer
concurrency before the 32-retry CAS budget exhausts), so this run demonstrates
a ~2–3× tail improvement, not livelock avoidance per se. Two negatives:
filesystem-row's store-level error rate (~5% conflict/claim-miss/scope-not-found
at c64–c100, classes the blob never emitted) and its goodput collapse (51 ops/s
at c100 — the lowest of all four backends, driven by an 87% busy-shed rate
because its slower submit→complete window keeps threads locked longer).

**row-memory vs memory (the #6263 Phase 1 gate):** **not close — the gate
fails as measured.** The direct authority's turn-store cost is 20 µs p50 /
≤121 µs p99, flat across the sweep. The row store over the in-memory backend
costs 4–5 ms p50 at low concurrency (~250× the direct authority) and collapses
under contention: 91 ms p50 / 501 ms p99 at c100 — four orders of magnitude off
the direct authority's envelope, with 3.1% store-level errors and one third of
the goodput (168 vs 509 ops/s). At the op level the ratio is ~2.9× on p99
(510 ms vs 175 ms) only because libSQL thread-service writes dominate op time
for both. Caveats that bound, but do not reverse, the negative: the harness's
`InMemoryBackend` is the test backend (one global async mutex, linear scans),
and all 8 users share it, so part of the collapse is backend serialization
rather than the row-store mechanism itself; a single-user volume at c≤8 sees
~5 ms p50 per turn transition, which a chat turn could absorb. But replacing
the direct authority in the `inmemory-turn-state` profile with this composition
would today trade microseconds for milliseconds on every transition and
hundreds of milliseconds under concurrent same-user load. Phase 1 needs either
a sharded/finer-grained in-memory `RootFilesystem` backend or a row-store
commit path that batches its per-transition backend round-trips before this is
retested.

Artifacts: `chatturn-memory.jsonl`, `chatturn-row-memory.jsonl`,
`chatturn-filesystem-row.jsonl`, `chatturn-filesystem.jsonl` (today's tree);
`chatturn-memory-junetree.jsonl`, `chatturn-filesystem-junetree.jsonl`
(June-study commit `4f7832b94` rebuilt and run on this box, c8/c100 and c100
respectively).

> Environment: shared 32-core AMD Ryzen 9 9950X3D Linux box (not the 4-core
> container of the June study — a major driver of the baseline shift), libSQL
> local file, `rustc 1.96.0`, `cargo build -p ironclaw_stress --release`.
> Load average 8–22 during the runs (a concurrent agent was compiling), so
> absolute numbers carry noise; all comparisons are same-box, same-session.
