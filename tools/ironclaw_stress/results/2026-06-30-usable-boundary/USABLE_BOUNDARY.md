# IronClaw Stress Usable Boundary - 2026-06-30

## Environment

- Machine: `Darwin Firats-Mac-mini.local 25.1.0 arm64`
- CPU: `Apple M4`
- Backend: `libsql`
- Stress tool: `cargo run -p ironclaw_stress --release`
- Date: 2026-06-30

## Usability Definition

For interactive user-turn workloads, this run treats the system as:

- **Usable**: `failure_rate <= 1%`, aggregate `p95 <= 2s`, aggregate `p99 <= 5s`.
- **Degraded**: aggregate SLO still passes, but interval traces show p95 spikes above
  `2s`, throughput collapse, or large RSS drift.
- **Unusable**: `failure_rate > 1%`, aggregate `p95 > 2s`, aggregate `p99 > 5s`, or
  sustained interval p95 above `2s`.

The headline boundary below uses aggregate p95/p99 because that is what the
current CLI thresholds enforce. The trace notes call out stricter interval-level
breaches.

## Commands Run

### Mixed User Concurrency Sweep

This sweep was interrupted after concurrency 8 because the concurrency 12 point
was taking several minutes. The completed points are still useful as the
low-concurrency baseline.

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario mixed-user-session \
  --sweep-concurrency 1,2,4,8,12,16,24 \
  --operations 50 \
  --users 1000 \
  --progress-interval-seconds 0 \
  --human-read \
  --bottleneck-report \
  --output-jsonl tools/ironclaw_stress/results/2026-06-30-usable-boundary/mixed-user-concurrency-sweep-completed.jsonl
```

### Targeted Boundary Points

```bash
for c in 10 12 16; do
  cargo run -p ironclaw_stress --release -- \
    --backend libsql \
    --scenario mixed-user-session \
    --concurrency "$c" \
    --operations 10 \
    --users 1000 \
    --progress-interval-seconds 0 \
    --human-read \
    --bottleneck-report
done
```

### Sustained Mixed User Runs

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario mixed-user-session \
  --concurrency 8 \
  --operations 20 \
  --duration-seconds 60 \
  --users 1000 \
  --progress-interval-seconds 10 \
  --trace-jsonl tools/ironclaw_stress/results/2026-06-30-usable-boundary/mixed-user-c8-60s.trace.jsonl \
  --human-read \
  --bottleneck-report
```

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario mixed-user-session \
  --concurrency 12 \
  --operations 20 \
  --duration-seconds 120 \
  --users 1000 \
  --progress-interval-seconds 10 \
  --trace-jsonl tools/ironclaw_stress/results/2026-06-30-usable-boundary/mixed-user-c12-120s.trace.jsonl \
  --human-read \
  --bottleneck-report
```

### Resource Governor Control

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario reserve-reconcile \
  --concurrency 12 \
  --operations 20 \
  --duration-seconds 60 \
  --users 1000 \
  --progress-interval-seconds 10 \
  --trace-jsonl tools/ironclaw_stress/results/2026-06-30-usable-boundary/reserve-reconcile-c12-60s.trace.jsonl \
  --human-read \
  --bottleneck-report
```

## Results

### Completed Concurrency Sweep Points

| Scenario | Concurrency | Attempted | Failed | Throughput ops/s | p95 | p99 | Max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `mixed-user-session` | 1 | 50 | 0 | 78.32 | 14.8ms | 15.2ms | 15.2ms |
| `mixed-user-session` | 2 | 100 | 0 | 51.03 | 94.7ms | 127.7ms | 127.7ms |
| `mixed-user-session` | 4 | 200 | 0 | 27.85 | 318.4ms | 497.1ms | 512.2ms |
| `mixed-user-session` | 8 | 400 | 0 | 13.60 | 1.16s | 1.94s | 2.64s |

### Targeted Short Boundary Points

| Scenario | Concurrency | Attempted | Failed | Throughput ops/s | p95 | p99 | Top Operation Group |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `mixed-user-session` | 10 | 100 | 0 | 28.93 | 815.7ms | 1.31s | `thread_store_writes` p95 514.4ms |
| `mixed-user-session` | 12 | 120 | 1 | 28.62 | 1.05s | 2.11s | `thread_store_writes` p95 694.4ms |
| `mixed-user-session` | 16 | 160 | 0 | 28.54 | 916.5ms | 1.59s | `resource_governor` p95 557.0ms |

These short points show the system can briefly handle concurrency above 8.
They are not sufficient to establish sustained usability because the longer
trace runs show drift.

### Sustained Runs

| Scenario | Concurrency | Duration | Attempted | Failed | Throughput ops/s | p50 | p95 | p99 | Peak RSS | DB Growth |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `mixed-user-session` | 8 | 60s | 958 | 0 | 15.86 | 481ms | 953ms | 1.54s | 315MB | +27MB |
| `mixed-user-session` | 12 | 120s | 1528 | 0 | 12.66 | 887ms | 1.84s | 2.42s | 524MB | +39MB |
| `reserve-reconcile` | 12 | 60s | 1860 | 0 | 30.72 | 454ms | 637ms | 666ms | 376MB | +13MB |

## Boundary

### Aggregate SLO Boundary

Using aggregate p95/p99 and failure-rate SLOs, the tested boundary is:

- **Usable through concurrency 12** for `mixed-user-session`.
- Concurrency 12 has aggregate `p95=1.84s`, `p99=2.42s`, and `0` failures over
  120 seconds.
- We did not prove concurrency 16 sustained usability; only a short 160-op run
  completed under the SLO.

Recommended current aggregate limit:

```text
mixed-user-session usable boundary: concurrency 12 on this Apple M4/libsql setup
```

### Stricter Interval Boundary

If the definition requires no interval-level p95 spikes above 2s, the boundary
is lower:

- Concurrency 8 had aggregate `p95=953ms`, but trace interval p95 reached
  `2.42s`.
- Concurrency 12 had aggregate `p95=1.84s`, but trace interval p95 reached
  `3.20s` and repeatedly exceeded 2s near the end of the run.

Recommended stricter limit:

```text
strict no-spike boundary: below concurrency 8, likely concurrency 4
```

This stricter limit should be confirmed with a 60-120s concurrency 4 trace run
before turning it into a hard CI threshold.

## Drift Evidence

### Mixed User Session, Concurrency 8

Early intervals:

| Elapsed | Recent ops/s | Interval p95 | RSS |
| ---: | ---: | ---: | ---: |
| 1s | 24.9 | 453ms | 18MB |
| 2s | 34.9 | 479ms | 21MB |
| 3s | 30.8 | 444ms | 22MB |
| 4s | 38.9 | 295ms | 25MB |

Late intervals:

| Elapsed | Recent ops/s | Interval p95 | RSS |
| ---: | ---: | ---: | ---: |
| 51s | 15.9 | 712ms | 233MB |
| 56s | 12.0 | 687ms | 235MB |
| 59s | 15.0 | 614ms | 236MB |
| 60s | 15.4 | 741ms | 152MB |

The aggregate result is usable, but throughput drops from a high near 39 ops/s
to about 12-16 ops/s while RSS grows above 300MB peak.

### Mixed User Session, Concurrency 12

Early intervals:

| Elapsed | Recent ops/s | Interval p95 | RSS |
| ---: | ---: | ---: | ---: |
| 1s | 15.8 | 754ms | 18MB |
| 2s | 26.9 | 935ms | 20MB |
| 3s | 30.8 | 1.06s | 22MB |
| 6s | 30.9 | 593ms | 27MB |

Late intervals:

| Elapsed | Recent ops/s | Interval p95 | RSS |
| ---: | ---: | ---: | ---: |
| 114s | 7.0 | 1.87s | 516MB |
| 115s | 3.0 | 2.40s | 519MB |
| 116s | 4.0 | 2.90s | 520MB |
| 117s | 7.9 | 3.01s | 521MB |
| 118s | 9.9 | 2.95s | 522MB |
| 119s | 5.0 | 1.28s | 523MB |
| 120s | 8.0 | 1.98s | 473MB |

This is the strongest drift signal. The final aggregate p95 is still below 2s,
but late-run intervals are often above 2s while RSS approaches 500MB.

### Resource Governor Control, Concurrency 12

Early intervals:

| Elapsed | Recent ops/s | Interval p95 | RSS |
| ---: | ---: | ---: | ---: |
| 1s | 210.8 | 96.8ms | 25MB |
| 2s | 98.0 | 140ms | 28MB |
| 3s | 71.9 | 172ms | 29MB |
| 4s | 62.4 | 200ms | 96MB |

Late intervals:

| Elapsed | Recent ops/s | Interval p95 | RSS |
| ---: | ---: | ---: | ---: |
| 52s | 15.0 | 619ms | 144MB |
| 56s | 23.8 | 637ms | 145MB |
| 58s | 23.0 | 650ms | 145MB |
| 60s | 14.0 | 687ms | 145MB |

The governor path also drifts, but remains far below the mixed-user latency
ceiling. It is a contributor, not the only cause.

## Bottleneck Analysis

### Aggregate Bottlenecks

For sustained `mixed-user-session` at concurrency 12:

- `resource_governor`: p95 `1.27s`, p99 `2.06s`
- `thread_store_writes`: p95 `402ms`, p99 `953ms`
- `turn_store`: p95 `198ms`, p99 `422ms`
- `context_reads`: p95 `13.9ms`

This means the sustained-load bottleneck is no longer just transcript writes.
As the run continues, resource governor operations dominate aggregate p95.

For sustained `mixed-user-session` at concurrency 8:

- `resource_governor`: p95 `522ms`
- `thread_store_writes`: p95 `422ms`
- `turn_store`: p95 `203ms`
- `context_reads`: p95 `13.4ms`

At concurrency 8, both governor and thread writes matter, with governor slightly
higher.

For `reserve-reconcile` at concurrency 12:

- p95 `637ms`, p99 `666ms`
- throughput `30.72 ops/s`
- CPU pressure: `55.49s CPU / 60.55s wall`
- DB growth: `+13MB`

This isolates the resource governor as a real load-sensitive component, but the
full user-turn path is still slower because it combines governor, thread-store,
and turn-store writes.

### Likely Causes of Baseline Drift

1. **Resource governor write/CAS pressure grows under sustained concurrency.**
   The c12 mixed run reports `resource_governor p95=1.27s`. The governor-only
   control also shows p95 rising from sub-200ms early intervals to roughly
   600-690ms late intervals.

2. **Thread-store writes remain expensive and amplify the full user-turn path.**
   Even when governor dominates at c12, `thread_store_writes p99=953ms`, and
   the c8 run has thread writes close to governor cost.

3. **RSS growth correlates with throughput drop and interval latency spikes.**
   Mixed c12 grows from about 11MB start RSS to 524MB peak RSS. Late intervals
   with RSS around 500MB have p95 in the 2.4-3.0s range and lower recent ops/s.

4. **DB growth is non-trivial but not alone sufficient to explain the drift.**
   Mixed c12 writes +39MB for 1528 operations. Governor control writes +13MB
   for 1860 operations and still drifts, so write volume and update contention
   both matter.

## Recommendations

### Operational Baseline

Use this as the current local libsql CI guard:

```text
concurrency=8, mixed-user-session, duration=60s
failure_rate <= 1%
aggregate p95 <= 2s
aggregate p99 <= 5s
max RSS <= 768MB
```

Use concurrency 12 as a stress boundary probe, not a hard green baseline yet:

```text
concurrency=12, mixed-user-session, duration=120s
expected: aggregate p95 near 2s, interval p95 may breach
```

### Engineering Follow-Ups

1. Investigate resource governor persistence under concurrency:
   - reserve/reconcile write count per user turn
   - whether unchanged resource state is rewritten
   - CAS/update retry behavior
   - indexes and transaction shape for governor state

2. Investigate thread-store write amplification:
   - number of thread writes per turn
   - assistant append/finalize transaction split
   - inbound message write path
   - default compact message size versus observed DB growth

3. Investigate memory/RSS growth:
   - run a longer soak with trace and RSS artifacts
   - compare `memory-churn` control against mixed-user-session
   - inspect retained buffers around context, transcript serialization, and
     libsql connection/cache behavior

4. Add a stricter CI mode after confirming c4/c6/c8 interval behavior:
   - fail if any trace interval p95 exceeds 2s for N consecutive intervals
   - fail if RSS keeps increasing after warmup

## Raw Artifacts

Raw run artifacts live in this directory:

- `mixed-user-concurrency-sweep-completed.jsonl`
- `mixed-user-c10-target.summary.json`
- `mixed-user-c12-target.summary.json`
- `mixed-user-c16-target.summary.json`
- `mixed-user-c8-60s.summary.json`
- `mixed-user-c8-60s.trace.jsonl`
- `mixed-user-c12-120s.summary.json`
- `mixed-user-c12-120s.trace.jsonl`
- `reserve-reconcile-c12-60s.summary.json`
- `reserve-reconcile-c12-60s.trace.jsonl`
