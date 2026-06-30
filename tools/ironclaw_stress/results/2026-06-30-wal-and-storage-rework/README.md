# libSQL write-concurrency: WAL + storage-rework results — 2026-06-30

Measures the two-step throughput work on the libSQL `RootFilesystem` write path:

1. **WAL + PRAGMA tuning** (PR #5451, merged) — `journal_mode=WAL`,
   `synchronous=NORMAL`, plus cache/mmap/temp_store tuning.
2. **Row-native sequence primitive + thread/turn append paths** (PR #5455) —
   `reserve_sequence` + finalized assistant-append, collapsing per-turn
   full-document rewrites into appends.

## Environment

- Kernel: `Linux 6.18.5 x86_64`
- CPU: 4 cores (cloud container)
- Backend: `libsql` (local file)
- Build: `rustc 1.96.0`, `cargo build -p ironclaw_stress --release`
- Date: 2026-06-30

> NOTE: This is a 4-core Linux container, **not** the Apple M4 used by
> `2026-06-30-usable-boundary/`. Absolute numbers are not comparable across
> the two machines; use the before/after deltas *within this directory*, which
> were all collected on the same box.

## Scenario

Pure-storage `chat-turn` (no synthetic model wait), so latency reflects
storage only:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql --scenario chat-turn \
  --sweep-concurrency <points> --operations <n> --users <u> \
  --progress-interval-seconds 0 --output-jsonl <file>
```

## Step 1 — WAL + PRAGMA tuning (#5451)

`--operations 30 --users 200`. Aggregate p95 and throughput, before vs after.

| Concurrency | p95 (DELETE journal) | p95 (WAL) | throughput (DELETE → WAL) | failures |
| ---: | ---: | ---: | ---: | ---: |
| 1  | 168.9ms | 94.3ms  | 12.9 → 11.2 ops/s | 0 / 0 |
| 4  | 1091.8ms | 217.8ms | 12.8 → 27.7 ops/s | 0 / 0 |
| 8  | 1285.5ms | 176.5ms | 9.4 → 33.6 ops/s | 0 / 0 |
| 16 | 1139.5ms | 199.6ms | 6.9 → 27.7 ops/s | 0 / 0 |
| 32 | 1311.7ms | 276.4ms | 5.7 → 19.5 ops/s | **2 → 0** |

Without WAL, p95 jumps to ~1.1–1.3s the moment concurrency exceeds 1 and
throughput *declines* as concurrency rises (writers serialize destructively on
the whole-file lock, with lock-timeout failures at c32). With WAL, p95 holds at
175–280ms and throughput rises to a ~33 ops/s plateau.

Artifacts: `wal-off-chatturn.jsonl`, `wal-on-chatturn.jsonl`.

## Step 2 — storage rework on top of WAL (#5455)

`--operations 30 --users 200`. WAL-only vs WAL + append-native storage.

| Concurrency | `thread_store_writes` p95 | aggregate p95 | throughput |
| ---: | ---: | ---: | ---: |
| 8  | 126.2ms → 106.4ms | 176.5ms → 200.8ms | 33.6 → 36.2 ops/s |
| 32 | **186.0ms → 97.0ms** | **276.4ms → 183.6ms** | **19.5 → 34.2 ops/s** |

WAL-only throughput collapses past c8 (19.5 ops/s at c32); with the
append-native paths it holds flat from c8 to c32.

Artifacts: `wal-plus-rework-chatturn.jsonl` (compare against
`wal-on-chatturn.jsonl`).

## Headline — 100 concurrent writes (WAL + storage rework)

`--operations 20 --users 500`, sweeping to c100.

| Concurrency | throughput | p50 | p95 | p99 | failures |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 8   | 31.9 ops/s | 103.3ms | 234.1ms | 432.9ms | 0 |
| 32  | 38.5 ops/s | 93.6ms  | 174.4ms | 258.6ms | 0 |
| 64  | 30.5 ops/s | 124.2ms | 191.9ms | 227.0ms | 0 |
| **100** | 21.1 ops/s | 183.1ms | **256.8ms** | **294.3ms** | **0 / 2000** |

**100 concurrent writes complete with zero failures and p95/p99 well inside
the usability SLO** (`p95 ≤ 2s`, `p99 ≤ 5s`). Throughput peaks at c32 and eases
to 21 ops/s at c100 — no collapse. The remaining bottleneck is split evenly
between `thread_store_writes` (~100–136ms) and `turn_store` (~91–143ms) — the
still-monolithic snapshot read-modify-writes; `context_reads` is negligible
(~12–22ms).

Artifact: `storage-rework-chatturn-to-c100.jsonl`.

## Interpretation

- The journey: pre-WAL throughput *fell* with concurrency (failures at c32);
  +WAL held ~33 ops/s but tapered past c8; +storage-rework holds with no
  failures and bounded p95 to c100.
- `chat-turn` has no model latency. Real turns are model-bound, so storage
  sustaining 100 concurrent at sub-300ms p95 means storage is no longer the
  ceiling for realistic workloads.
- Next structural lever for the *throughput* ceiling (not concurrency/latency,
  which are met): per-scope sharding of the turn/thread snapshots so
  non-overlapping work stops contending on one document.
