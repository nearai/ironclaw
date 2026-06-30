# Turn-state store: in-memory authority vs per-user filesystem CAS — 2026-06-30

Reproduces and measures the turn-state contention behind the "runtime wedge"
(`turn state filesystem CAS retries exhausted`): all of a user's turns — a
foreground turn plus routine/heartbeat turns on different threads — write the
single per-user `/turns/state.json` via CAS read-modify-write, and under
concurrent same-user writers the 32-retry budget livelocks.

## What was added to the harness

- `--turn-state-backend {filesystem,memory}` — selects the turn-state store.
  `filesystem` is the current production path (`FilesystemTurnStateStore`,
  per-user `state.json` CAS). `memory` is one shared in-process
  `InMemoryTurnStateStore` authority (coordination in memory, no per-step CAS),
  shared across all workers to model the single-process runtime.
- `--threads-per-owner N` — spreads one owner-user's concurrent load across N
  distinct threads that all share that owner's single `state.json`. Without
  this the harness accidentally shards (thread ↔ owner 1:1), so the filesystem
  CAS never contends cross-thread and the bug doesn't reproduce.

## Scenario

`chat-turn` (pure storage, no model wait), 8 owner-users × 16 threads each,
20 operations/thread, libSQL local file, concurrency swept 8→100:

```bash
ironclaw_stress --backend libsql --scenario chat-turn \
  --turn-state-backend <filesystem|memory> \
  --users 8 --active-thread-count 8 --threads-per-owner 16 --operations 20 \
  --sweep-concurrency 8,32,64,100 --progress-interval-seconds 0
```

## Results

| Concurrency | filesystem p99 → max (fail%) | memory p99 → max (fail%) | ops/s (fs → mem) |
| ---: | ---: | ---: | ---: |
| 8   | 466ms → 498ms (0%)    | 146ms → 155ms (0%)   | 31.3 → 69.6 |
| 32  | **2.09s** → 18.85s (1.56%) | 139ms → 304ms (0%)   | 25.3 → 65.0 |
| 64  | **13.07s** → 48.80s (1.80%) | 133ms → 226ms (0.23%) | 21.0 → 63.4 |
| 100 | **42.19s** → 85.87s (5.35%) | 128ms → 169ms (0%)   | 18.7 → 53.2 |

The filesystem per-user CAS **livelocks** as concurrency rises: p99 explodes to
42s, max to 86s, throughput *declines* (31→19 ops/s), and 5.35% of operations
fail with CAS-retries-exhausted at c100. The single shared in-memory authority
holds p99 flat at ~130ms (a ~320× tail improvement at c100), ~0 failures, and
throughput that scales (~3× higher). This validates moving turn-state
coordination to one in-process authority for the single-process hosted runtime.

Artifacts: `chatturn-filesystem.jsonl`, `chatturn-memory.jsonl`.

> Environment: 4-core Linux container, libSQL local file, `rustc 1.96.0`,
> `cargo build -p ironclaw_stress --release`.
