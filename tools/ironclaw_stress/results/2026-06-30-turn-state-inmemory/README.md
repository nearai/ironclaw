# Turn-state store: in-memory authority vs per-user filesystem CAS — 2026-06-30

Reproduces and measures the turn-state contention behind the "runtime wedge"
(`turn state filesystem CAS retries exhausted`): all of a user's turns — a
foreground turn plus routine/heartbeat turns on different threads — write the
single per-user `/turns/state.json` via CAS read-modify-write, and under
concurrent same-user writers the 32-retry budget livelocks.

## What was added to the harness

- `--turn-state-backend {filesystem,memory,memory-persist-on-block}` — selects
  the turn-state store. `filesystem` is the current production path
  (`FilesystemTurnStateStore`, per-user `state.json` CAS). `memory` is one shared
  in-process `InMemoryTurnStateStore` authority (coordination in memory, no
  per-step CAS), shared across all workers to model the single-process runtime.
  `memory-persist-on-block` is that same shared authority with the durable
  `FilesystemTurnStateBlockPersistence` sink attached — the shipped
  hosted-single-tenant-volume config — used to confirm persist-on-block adds no
  hot-path cost.
- `--threads-per-owner N` — spreads one owner-user's concurrent load across N
  distinct threads that all share that owner's single `state.json`. Without
  this the harness accidentally shards (thread ↔ owner 1:1), so the filesystem
  CAS never contends cross-thread and the bug doesn't reproduce.
- `--gate-blocked-every N` — every Nth measured user-turn operation blocks its
  run on a gate (alternating approval/auth), resumes it, then re-claims and
  completes. 0 (default) = the pure claim/complete hot path. Combined with
  `memory-persist-on-block` this drives the durable persist-on-block writes under
  concurrency (see the blocking-workload section below).

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

## Heavier-scale re-run (16 owners × 32 threads) — validates the shipped feature path

Same harness `memory` backend is the exact `InMemoryTurnStateStore` the product
now wires via the `inmemory-turn-state` feature, so this stresses the shipped
path. `--users 16 --active-thread-count 16 --threads-per-owner 32 --operations 20`,
concurrency 16→128:

| Concurrency | filesystem p99 → max (fail%) | memory p99 → max (fail%) | ops/s (fs → mem) |
| ---: | ---: | ---: | ---: |
| 16  | 281ms → 320ms (0%)      | 137ms → 172ms (0%)  | 28.2 → 52.1 |
| 64  | 376ms → **53.97s** (0.16%) | 143ms → 225ms (0%)  | 20.3 → 47.6 |
| 128 | 519ms → **147.64s** (0.78%) | 176ms → 194ms (0%)  | 14.4 → 32.9 |

At c128 the filesystem CAS worst-case is **147 seconds** (livelock) vs the
in-memory authority's **194ms** — ~760× better worst-case, 0% failures, ~2.3×
throughput that keeps scaling instead of collapsing.

Artifacts: `chatturn-filesystem-16x32.jsonl`, `chatturn-memory-16x32.jsonl`.

## Persist-on-block durability — no hot-path regression

The in-memory authority is volatile across a process restart, which is fine for
in-flight compute (short-lived, re-triggerable) but not for a turn parked on a
human gate (approval/auth). Persist-on-block closes that gap: the store snapshots
to the durable filesystem **only when the gate-blocked set changes** (a run
blocks on a gate, or such a run resumes/terminates), and rehydrates from the last
such snapshot on startup. Normal chat traffic (claim → complete) never blocks on
a gate, so the sink never fires on the hot path; the only added cost is a probe
that is short-circuited away entirely when no sink is attached.

`--turn-state-backend memory-persist-on-block` attaches the same
`FilesystemTurnStateBlockPersistence` sink the hosted-single-tenant-volume
runtime wires, so this measures the shipped durable config against plain
`memory`. Same 8×16 sweep as above:

| Concurrency | memory p99 → max (fail%) | memory-persist-on-block p99 → max (fail%) | ops/s (mem → mem+block) |
| ---: | ---: | ---: | ---: |
| 8   | 157ms → 173ms (0%)    | 156ms → 162ms (0%)    | 52.6 → 51.9 |
| 32  | 143ms → 1.21s (0%)    | 127ms → 174ms (0.31%) | 56.3 → 62.6 |
| 64  | 151ms → 171ms (0.62%) | 139ms → 190ms (0%)    | 45.4 → 49.5 |
| 100 | 176ms → 222ms (0%)    | 162ms → 191ms (0.90%) | 32.2 → 38.4 |

The two backends track each other within run-to-run noise — persist-on-block adds
no measurable hot-path cost while making gate-blocked turns durable across a
restart.

Artifacts: `chatturn-memory-persist-on-block-baseline.jsonl` (plain `memory`),
`chatturn-memory-persist-on-block.jsonl`.

## Persist-on-block under a *blocking* workload

The sweep above is the hot path — it never blocks, so the sink stays idle. To
actually exercise persist-on-block under load, `--gate-blocked-every N` routes
every Nth measured operation through a real gate block + resume (alternating
approval/auth), then re-claims and completes the resumed run. This drives the
durable snapshot write on each blocked-set change under concurrency. Correctness
of a single block → persist → rehydrate cycle (both approval and auth gates) is
pinned by the `ironclaw_turns` unit test
`blocked_run_persists_to_sink_and_rehydrates_across_restart`; this sweep measures
the *cost* the durable writes add when many turns block concurrently.

`--users 64 --active-thread-count 64 --threads-per-owner 1` (one thread per
worker, so a blocked run's same-thread lock doesn't manufacture `ThreadBusy`
noise), concurrency 8→64, `memory` (sink off) vs `memory-persist-on-block` (sink
on) at two block rates:

**25% of turns block (`--gate-blocked-every 4`) — a deliberately pathological rate:**

| Concurrency | memory p99 → max (fail%) | persist-on-block p99 → max (fail%) | ops/s (off → on) |
| ---: | ---: | ---: | ---: |
| 8  | 130ms → 143ms (0%) | 155ms → 158ms (0%) | 65.1 → 53.9 |
| 32 | 131ms → 163ms (0%) | 150ms → 203ms (0%) | 59.1 → 51.4 |
| 64 | 133ms → 171ms (0%) | 215ms → 255ms (0%) | 48.8 → 40.8 |

**5% of turns block (`--gate-blocked-every 20`) — closer to a realistic gate rate:**

| Concurrency | memory p99 → max (fail%) | persist-on-block p99 → max (fail%) | ops/s (off → on) |
| ---: | ---: | ---: | ---: |
| 8  | 177ms → 187ms (0%) | 106ms → 114ms (0%) | 56.4 → 64.1 |
| 32 | 139ms → 195ms (0%) | 131ms → 198ms (0%) | 58.0 → 56.8 |
| 64 | 128ms → 181ms (0%) | 178ms → 259ms (0%) | 49.4 → 46.2 |

Reading:

- **0% failures at every block rate.** The block → resume → re-claim → complete
  cycle works under concurrency; gate-blocked turns are persisted and re-claimed
  cleanly, with no CAS livelock (contrast the filesystem backend, which livelocks
  on the *non*-blocking hot path itself).
- **Cost scales with block rate, not the hot path.** Each blocked turn triggers
  two durable snapshot writes (one on block, one on resume) to the single
  `/turns/state.json`; under a pathological 25% block rate that serialized write
  costs ~15–20% throughput and a higher tail at c64. At a realistic ~5% rate the
  two backends are within run-to-run noise apart from a modest tail bump at high
  concurrency.
- **Net:** persist-on-block is free on the hot path and cheap at realistic gate
  rates. If a future workload ever parks a large fraction of turns on gates
  concurrently, the snapshot-per-change write is the knob to revisit (an
  append-only block delta would remove the full-snapshot cost).

Artifacts: `chatturn-blocked-memory.jsonl`,
`chatturn-blocked-memory-persist-on-block.jsonl` (25%);
`chatturn-blocked5pct-memory.jsonl`,
`chatturn-blocked5pct-memory-persist-on-block.jsonl` (5%).
