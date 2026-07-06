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

## Persist-on-block under a *blocking* workload (correctness, not throughput)

The sweep above is the hot path — it never blocks, so the sink stays idle. To
actually exercise persist-on-block under load, `--gate-blocked-every N` routes
every Nth measured operation through a real gate block + resume (alternating
approval/auth by blocked-hit count), then re-claims and completes the resumed
run — driving a durable snapshot write on each blocked-set change under
concurrency. Command shape:

```bash
ironclaw_stress --backend libsql --scenario chat-turn \
  --turn-state-backend memory-persist-on-block --gate-blocked-every 4 \
  --users 64 --active-thread-count 64 --threads-per-owner 1 --operations 20 \
  --sweep-concurrency 8,32,64
```

What this run establishes:

- **0% failures at every block rate (4 and 20).** The block → resume → re-claim →
  complete cycle works under concurrency for both approval and auth gates; gate
  turns are persisted and re-claimed cleanly, with no CAS livelock (contrast the
  filesystem backend, which livelocks on the *non*-blocking hot path itself). The
  `memory` (sink-off) arm at the same block rate stays flat at ~130–190 ms p99 /
  ~45–60 ops/s, confirming the block/resume/reclaim machinery itself is cheap.

**Why this harness does *not* give a production throughput number for the sink.**
The harness deliberately shares **one** `InMemoryTurnStateStore` across every
synthetic user (that shared authority is the whole point of the CAS-livelock
repro above). So its persist-on-block snapshot grows to *all* accumulated
synthetic runs — hundreds to thousands — and every block-set change serializes
and writes that entire blob. A production hosted-single-tenant-volume runtime has
a **separate, small** store per tenant/user volume and blocks only on rare human
gates, so its snapshot is tiny and the write is cheap. The sink-on arm's
throughput under this shared-store harness is therefore a pessimistic artifact of
the harness model (a giant snapshot), **not** a production estimate, and is not
reported here to avoid being misread as one.

Correctness of the durable path — block → persist → rehydrate for both approval
and auth gates, plus terminal convergence (a resumed-then-completed run rehydrates
as `Completed`, not a live `Queued`) — is pinned by the `ironclaw_turns` unit
test `blocked_run_persists_to_sink_and_rehydrates_across_restart`. Concurrent
block-set changes are coalesced and stale snapshots are dropped (monotonic
sequence + stale-skip in `persist_blocked_state`) so an older snapshot can never
blind-overwrite a newer durable one. If a future *production* workload ever parks
a large fraction of a single volume's turns on gates at once, the
snapshot-per-change write is the knob to revisit (an append-only block delta would
remove the full-snapshot cost).

Artifacts (raw 0%-failure runs, config recorded in each row):
`chatturn-blocked-memory.jsonl`,
`chatturn-blocked-memory-persist-on-block.jsonl` (25% block rate);
`chatturn-blocked5pct-memory.jsonl`,
`chatturn-blocked5pct-memory-persist-on-block.jsonl` (5% block rate).
