# Process journal contention rerun - 2026-07-26

This reruns the former turn-state contention matrix against the process journal
that replaced the turn stores. It answers two questions:

1. Does `ProcessJournalStore` preserve process transitions under same-owner,
   cross-thread contention?
2. How much of the user-turn latency comes from the journal rather than thread
   and message persistence?

## Environment

- Commit: `fcbdfc7bb1811ab9e7ea19eb41cc2f3c1c2ced3b`
- Branch: `process-journal-kernel-transition`
- Rust: `rustc 1.96.0`
- CPU: AMD Ryzen 9 9950X3D, 16 cores / 32 threads
- Storage: local LibSQL on `/data`
- Load average after the runs: 2.13, 5.44, 4.02
- Build: `cargo run -p ironclaw_stress --release -- --help`

Each matrix uses 8 owners, 16 threads per owner, 20 operations per worker, and
concurrency 8, 32, 64, and 100. Runs were sequential.

`filesystem-journal` is `ProcessJournalStore` over LibSQL.
`memory-journal` is the same store over `InMemoryBackend`.

At the measured commit the stress-only mount mapped `/processes` onto a target
directory named `turns`, so failure samples contain `/turns/journal/state.json`.
The active harness now uses a `processes` target. This naming correction does
not change the backend or journal implementation measured here.

## Commands

The realistic user-turn matrix:

```bash
target/release/ironclaw_stress \
  --backend libsql \
  --scenario chat-turn \
  --process-journal-backend <filesystem-journal|memory-journal> \
  --users 8 \
  --active-thread-count 8 \
  --threads-per-owner 16 \
  --operations 20 \
  --sweep-concurrency 8,32,64,100 \
  --progress-interval-seconds 0 \
  --human-read \
  --bottleneck-report \
  --output-jsonl <artifact>
```

The journal-isolated matrix uses the same command with:

```text
--scenario turn-lifecycle-churn
```

That scenario performs only submit, claim, and complete. It does not write
threads, messages, assistant output, or context.

## Chat-turn results

The failure columns distinguish expected exclusive-thread admission rejection
from process-journal failure. `unavailable` is CAS retry exhaustion in every
observed case.

| Backend | c | succeeded / attempted | busy | unavailable | op p99 | journal p50 / p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| memory | 8 | 160 / 160 | 0 | 0 | 98 ms | 25 ms / 41 ms |
| memory | 32 | 249 / 640 | 229 | 162 | 413 ms | 112 ms / 410 ms |
| memory | 64 | 268 / 1280 | 835 | 177 | 1.13 s | 61 ms / 1.12 s |
| memory | 100 | 266 / 2000 | 1594 | 140 | 2.77 s | 122 ms / 2.75 s |
| LibSQL | 8 | 160 / 160 | 0 | 0 | 78 ms | 4.8 ms / 57 ms |
| LibSQL | 32 | 470 / 640 | 166 | 4 | 289 ms | 26 ms / 146 ms |
| LibSQL | 64 | 559 / 1280 | 667 | 54 | 591 ms | 19 ms / 298 ms |
| LibSQL | 100 | 540 / 2000 | 1364 | 96 | 758 ms | 18 ms / 345 ms |

The harness throughput metric counts attempted operations, including fast
rejections. It is not successful-operation goodput when failures are present.

## Journal-isolated results

| Backend | c | succeeded / attempted | busy | unavailable | journal p50 / p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| memory | 8 | 160 / 160 | 0 | 0 | 8.8 ms / 20 ms |
| memory | 32 | 244 / 640 | 202 | 194 | 79 ms / 382 ms |
| memory | 64 | 216 / 1280 | 886 | 178 | 38 ms / 771 ms |
| memory | 100 | 164 / 2000 | 1677 | 159 | 49 ms / 922 ms |
| LibSQL | 8 | 160 / 160 | 0 | 0 | 5.4 ms / 83 ms |
| LibSQL | 32 | 252 / 640 | 285 | 103 | 37 ms / 220 ms |
| LibSQL | 64 | 154 / 1280 | 987 | 139 | 13 ms / 283 ms |
| LibSQL | 100 | 121 / 2000 | 1744 | 135 | 17 ms / 334 ms |

## Findings

1. **The journal fails its contention correctness gate.** Both backends exhaust
   the five-attempt CAS budget at concurrency 32 and above. Failures occur in
   submit, claim, and complete, so this is not only admission behavior.

2. **The failure is in the process-journal storage shape.** Every mutation
   loads, mutates, serializes, and CAS-writes one growing materialized state
   document. Independent operations on different processes conflict because
   they share that document.

3. **`InMemoryBackend` is not a fast-authority baseline.** Its global backend
   synchronization and the journal's full-state serialization make the memory
   arm slower than LibSQL under this workload. It only removes durable I/O; it
   does not remove journal contention.

4. **High-concurrency throughput is misleading.** The apparent throughput rise
   in lifecycle churn comes from busy and unavailable operations failing
   quickly. Successful counts fall from 160 at c8 to 164 (memory) and 121
   (LibSQL) at c100 despite 12.5 times more attempts.

5. **The journal allocates heavily as terminal history grows.** End RSS reached
   420 MiB for memory lifecycle churn and 317 MiB for LibSQL lifecycle churn at
   c100. Allocator retention means this is not by itself proof of a live-memory
   leak, but it is consistent with repeatedly cloning and serializing growing
   materialized state and needs a bounded-history soak test after the storage
   fix.

## Verdict

The process/journal abstraction remains useful, but the current single-document
implementation is not a production-quality authority under same-owner
concurrency. Do not move more durable state into this document before replacing
the storage layout with independently CAS-able process records plus an
append-only journal/index mechanism.

The follow-up consolidation slices are defined in
`docs/internal/reborn/2026-07-26-process-kernel-next-collapses.md`.

Artifacts:

- `chatturn-memory-journal.jsonl`
- `chatturn-filesystem-journal.jsonl`
- `lifecycle-memory-journal.jsonl`
- `lifecycle-filesystem-journal.jsonl`
