# ironclaw_stress

`ironclaw_stress` is a developer tool for finding IronClaw infrastructure
bottlenecks. It runs synthetic workloads through the same storage, thread,
turn, resource-governor, and process-pressure paths used by the runtime, then
prints JSON plus optional human-readable bottleneck reports.

The tool is intentionally diagnostic rather than a correctness test. Use it to
answer questions like:

- How far can local `libsql` go before p95 latency or throughput degrades?
- Is the current limit storage writes, context reads, the turn store, resource
  governor writes, synthetic model/tool latency, CPU, or memory?
- Does a hot thread serialize work as expected?
- Does context growth or tool output size cause write/read amplification?
- Does a long run show RSS growth, throughput collapse, or tail-latency drift?

## Quick Start

Run the broad libsql bottleneck scan:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --suite bottleneck-finder \
  --human-read \
  --bottleneck-report
```

Run a small smoke test:

```bash
cargo run -p ironclaw_stress -- \
  --backend libsql \
  --scenario chat-turn \
  --concurrency 1 \
  --operations 5 \
  --users 5 \
  --human-read \
  --bottleneck-report
```

Run against a stable local database file:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --libsql-path /tmp/ironclaw-stress.db \
  --suite bottleneck-finder \
  --human-read \
  --bottleneck-report
```

## Output Streams

The main JSON summary is written to stdout. Progress, human tables, and
bottleneck reports are written to stderr.

Useful flags:

- `--human-read`: append readable tables to stderr.
- `--bottleneck-report`: append heuristic bottleneck findings to stderr.
- `--trace-jsonl PATH`: write interval samples for long runs and throughput
  collapse analysis.
- `--output-jsonl PATH`: write one JSON object per sweep or suite case.
- `--compare-json PATH`: compare the current run or suite against a prior JSON
  or JSONL output file.

Example with files:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --suite bottleneck-finder \
  --trace-jsonl /tmp/ironclaw-trace.jsonl \
  --output-jsonl /tmp/ironclaw-suite.jsonl \
  --human-read \
  --bottleneck-report \
  > /tmp/ironclaw-summary.json \
  2> /tmp/ironclaw-report.txt
```

## What It Measures

Every run reports:

- attempted, succeeded, and failed operations
- throughput in operations/sec
- operation latency: min, p50, p95, p99, max
- process metrics: CPU time, peak RSS, thread count, open file descriptors
- DB probe metrics:
  - libsql database, WAL, and SHM file growth
  - Postgres database size and connection state when using Postgres
- error buckets and failure causes

User-turn workloads also report stage latency and operation attribution.

Stage latency identifies the exact measured step, such as:

- `ensure_thread`
- `accept_inbound`
- `submit_turn`
- `claim_run`
- `append_assistant`
- `load_context`
- `resource_reserve`
- `model_wait`
- `tool_wait`
- `append_tool_result`

Operation attribution groups those stages into broader bottleneck classes:

- `thread_store_writes`: thread service writes such as inbound messages,
  assistant output, tool results, previews, and draft updates.
- `context_reads`: context-window loads.
- `turn_store`: turn submission, claim, and completion transitions.
- `resource_governor`: reserve, reconcile, and release operations.
- `model_tool_wait`: model and tool waits. By default these are synthetic
  sleeps; with `--model-latency-source provider`, `model_wait` is a real LLM
  provider request. If this dominates, storage is probably not the current p95
  ceiling.

## Scenarios

Use `--scenario` for a single workload.

| Scenario | Purpose |
| --- | --- |
| `reserve-release` | Resource governor reserve/release pressure. |
| `reserve-reconcile` | Resource governor reserve/reconcile/release pressure. |
| `chat-turn` | One realistic user turn with thread writes, turn state, assistant write, and context load. |
| `mixed-user-session` | Realistic user turn with configurable synthetic or provider-backed model latency. |
| `context-growth` | Sequentially grows history, then loads context to expose context read amplification. |
| `tool-session` | Realistic turn with synthetic tool calls, tool previews, tool results, and optional tool wait/failure paths. |
| `cpu-burn` | Process-local CPU pressure control. |
| `memory-churn` | Process-local allocation/RSS pressure control. |

## Presets

Use `--preset` for a named single workload. Explicit CLI flags override preset
defaults.

| Preset | What it targets |
| --- | --- |
| `chat-baseline` | Baseline user-turn storage latency and throughput. |
| `hot-thread` | Same-thread serialization and busy-thread rejection behavior. |
| `large-context` | Context read amplification with prefilled history. |
| `tool-heavy` | Tool transcript writes and larger tool output payloads. |
| `model-tail` | Tail-spike synthetic model latency. |
| `resource-contention` | Resource governor write contention. |
| `cpu-burn` | CPU ceiling. |
| `memory-churn` | Allocation and RSS pressure. |
| `soak-user-session` | Long-run mixed user session for memory growth, throughput decay, and tail-latency drift. |

Examples:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset large-context \
  --human-read \
  --bottleneck-report
```

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset hot-thread \
  --concurrency 16 \
  --users 100 \
  --active-thread-count 1 \
  --span-log-failures \
  --human-read \
  --bottleneck-report
```

## Suites

Use `--suite` for a curated multi-case run. Suite mode runs several cases and
prints one JSON object containing all case summaries. It also adds per-case
fields:

- `top_failure_bucket`
- `top_operation_group`
- `postgres_pool_size`

### bottleneck-finder

The broad scan for libsql and general local bottleneck discovery:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --suite bottleneck-finder \
  --human-read \
  --bottleneck-report
```

Cases include:

- `resource-contention`
- `chat-baseline`
- `hot-thread`
- `large-context`
- `tool-heavy`
- `tool-wait`
- `tool-failure`
- `model-tail`
- `cpu-burn`
- `memory-churn`

### postgres-pool-pressure

This suite is available for Postgres pool and remote database pressure work.
It requires `--backend postgres`.

```bash
export IRONCLAW_FILESYSTEM_POSTGRES_URL='postgresql://USER:PASSWORD@HOST:PORT/DB'

cargo run -p ironclaw_stress --release -- \
  --backend postgres \
  --suite postgres-pool-pressure \
  --postgres-pool-size 4 \
  --human-read \
  --bottleneck-report
```

Current libsql-focused work does not require this suite. Keep it for future
remote Postgres validation.

## Common Runs

### Broad libsql scan

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --suite bottleneck-finder \
  --human-read \
  --bottleneck-report
```

### Find the single-process concurrency ceiling

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset chat-baseline \
  --ramp-concurrency 64 \
  --max-p95-ms 500 \
  --max-failure-rate 0.01 \
  --human-read \
  --bottleneck-report
```

### Test hot-thread behavior

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset hot-thread \
  --concurrency 16 \
  --users 100 \
  --active-thread-count 1 \
  --human-read \
  --bottleneck-report \
  --span-log-failures
```

Expected interpretation: `turn_thread_busy` failures mean the same thread is
being intentionally serialized. If the goal is raw storage throughput, increase
`--active-thread-count` or use one thread per user.

### Test context read amplification

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset large-context \
  --prefill-threads 100 \
  --prefill-turns-per-thread 100 \
  --context-max-messages 200 \
  --human-read \
  --bottleneck-report
```

Look for `context_reads` and `load_context`.

### Test payload write amplification

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario chat-turn \
  --concurrency 4 \
  --operations 50 \
  --users 100 \
  --user-message-bytes 4096 \
  --assistant-message-bytes 8192 \
  --human-read \
  --bottleneck-report
```

Look for `thread_store_writes`, `accept_inbound`, `append_assistant`, and DB
file/WAL growth.

### Test tool output pressure

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario tool-session \
  --concurrency 4 \
  --operations 50 \
  --users 100 \
  --tool-calls-per-turn 8 \
  --tool-output-bytes 8192 \
  --human-read \
  --bottleneck-report
```

Look for `append_tool_result`, `append_tool_preview`, and
`thread_store_writes`.

### Test tool wait versus storage

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario tool-session \
  --concurrency 4 \
  --operations 50 \
  --users 100 \
  --tool-calls-per-turn 4 \
  --tool-latency-ms 250 \
  --human-read \
  --bottleneck-report
```

If `model_tool_wait` dominates, tool latency is the current p95 ceiling.

### Test real LLM provider latency

Provider latency mode sends a small completion request through
`ironclaw_llm`'s provider chain during the `model_wait` stage of
`mixed-user-session`. This measures provider latency plus the runtime/storage
work that happens around it.

Configure the provider the same way the runtime does. Examples:

```bash
export LLM_BACKEND=openai
export OPENAI_API_KEY=...
```

```bash
export LLM_BACKEND=nearai
export NEARAI_API_KEY=...
```

Then run a bounded probe:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario mixed-user-session \
  --model-latency-source provider \
  --provider-max-tokens 8 \
  --concurrency 4 \
  --operations 20 \
  --users 100 \
  --human-read \
  --bottleneck-report
```

Look at:

- `model_latency_source`: confirms the run used `provider`.
- `provider_model`: optional per-request model override; omit it to use the
  configured provider default.
- `model_wait`: real provider request latency.
- `thread_store_writes`, `turn_store`, `resource_governor`, and
  `context_reads`: runtime/storage latency while provider calls are in flight.
- error buckets such as `model_provider_rate_limited`, `model_provider_auth`,
  `model_provider_model_unavailable`, and `model_provider_error`.

Keep early runs small. Provider mode can spend real tokens, hit rate limits,
and exercise retries/circuit breakers.

### Test failed tool-result paths

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario tool-session \
  --concurrency 4 \
  --operations 50 \
  --users 100 \
  --tool-calls-per-turn 4 \
  --tool-failure-every 3 \
  --span-log-failures \
  --human-read \
  --bottleneck-report
```

This records failed synthetic tool results. It does not necessarily fail the
whole operation; use it to measure the failed-tool transcript path.

### Long-run soak

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset soak-user-session \
  --trace-jsonl /tmp/ironclaw-soak.jsonl \
  --human-read \
  --bottleneck-report
```

The soak preset defaults to:

- `mixed-user-session`
- 15 measured minutes
- 60 second warmup
- interval traces every 30 seconds
- prefilled history
- moderate message payloads

Watch for RSS growth, throughput drops, increasing p95/p99, and DB file/WAL
growth.

## Sweeps

Sweeps run multiple points and can write JSONL for later comparison.

Example: sweep concurrency and users:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario chat-turn \
  --sweep-concurrency 1,2,4,8,16 \
  --sweep-users 10,50,100 \
  --operations 50 \
  --output-jsonl /tmp/ironclaw-sweep.jsonl \
  --human-read \
  --bottleneck-report
```

Example: sweep payload sizes:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario chat-turn \
  --concurrency 4 \
  --users 100 \
  --operations 50 \
  --sweep-user-message-bytes 0,1024,4096 \
  --sweep-assistant-message-bytes 0,2048,8192 \
  --output-jsonl /tmp/ironclaw-payload-sweep.jsonl
```

Example: sweep context size:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset large-context \
  --sweep-context-max-messages 20,50,100,200 \
  --output-jsonl /tmp/ironclaw-context-sweep.jsonl
```

## Ramps and Thresholds

Ramps increase one axis until a threshold fails. This is useful for finding the
first point where the system exceeds an SLO.

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset chat-baseline \
  --ramp-concurrency 64 \
  --ramp-factor 2 \
  --max-p95-ms 500 \
  --max-failure-rate 0.01 \
  --min-throughput 20 \
  --max-rss-mb 1024 \
  --human-read \
  --bottleneck-report
```

Threshold flags:

- `--max-failure-rate`
- `--max-p95-ms`
- `--min-throughput`
- `--max-rss-mb`
- `--max-cpu-ms`

## Trace JSONL

`--trace-jsonl` writes interval-level samples. Use it for long runs where the
final summary hides when degradation started.

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --preset soak-user-session \
  --trace-jsonl /tmp/ironclaw-soak.jsonl
```

Multi-process runs write child-specific trace files derived from the requested
path.

Useful quick inspection:

```bash
jq '{phase, sequence, recent_ops_sec, interval_latency}' /tmp/ironclaw-soak.jsonl
```

## Reading Bottleneck Signals

Start with the suite summary:

- High `fail%`: inspect `top_failure`.
- Low `ops/sec` with high p95: inspect `top_group`.
- High CPU with low storage attribution: the process is CPU-bound.
- High RSS or rising RSS in trace output: investigate memory growth.

Then map `top_group` to the next probe:

| Top group | Likely meaning | Next probe |
| --- | --- | --- |
| `thread_store_writes` | Message/tool transcript writes dominate. | Sweep message/tool payload sizes and active thread count. |
| `context_reads` | Context window loading dominates. | Increase/decrease prefill and `--context-max-messages`. |
| `turn_store` | Turn submission/claim/complete state dominates. | Compare `chat-turn` to `reserve-reconcile`. |
| `resource_governor` | Reservation/reconcile/release writes dominate. | Run `resource-contention` and compare concurrency. |
| `model_tool_wait` | Model/tool wait dominates. | Lower synthetic model/tool latency, or inspect provider latency when using provider mode, to reveal storage overhead. |

Failure buckets:

- `turn_thread_busy`: expected under hot-thread pressure. Increase
  `--active-thread-count` to test storage throughput instead.
- `storage_cross_process_cas_contention`: expected in multi-process resource
  contention. Compare with `--processes 1`.
- Postgres auth/connect errors: fix credentials or pooler configuration before
  interpreting storage latency.

## Multi-Process Runs

Multi-process mode is for low-level resource scenarios, not user-turn scenarios.
User-turn scenarios currently require `--processes 1`.

Example:

```bash
cargo run -p ironclaw_stress --release -- \
  --backend libsql \
  --scenario reserve-reconcile \
  --processes 4 \
  --concurrency 4 \
  --operations 100 \
  --human-read \
  --bottleneck-report
```

Use this to expose cross-process CAS/update contention.

## Backend Notes

### libsql

If `--libsql-path` is omitted, the tool creates a temporary database path and
prints a redacted target. Use a stable path when comparing DB growth across
runs.

```bash
--libsql-path /tmp/ironclaw-stress.db
```

libsql DB probe fields include:

- `libsql_file`
- `libsql_wal`
- `libsql_shm`

### Postgres

Postgres is supported by the tool, but current local bottleneck discovery can be
done with libsql. To use Postgres, pass `--postgres-url` or set:

```bash
export IRONCLAW_FILESYSTEM_POSTGRES_URL='postgresql://USER:PASSWORD@HOST:PORT/DB'
```

The URL is redacted in output. Postgres DB probe fields include database size
and active/idle/waiting connection counts.

## Practical Workflow

1. Run `--suite bottleneck-finder` on libsql.
2. Identify the worst case by failure rate, p95, throughput, CPU, RSS, and
   `top_group`.
3. Rerun that case as a single `--preset` or `--scenario` with
   `--human-read --bottleneck-report --trace-jsonl`.
4. Sweep one axis at a time: concurrency, users, active thread count, payload
   size, context size, tool calls, or model/tool latency.
5. Use thresholds to turn the discovered limit into a reproducible guard.
6. Use `--compare-json` against a prior run after changes.

## CI and Review Use

For quick local confidence:

```bash
cargo fmt -p ironclaw_stress -- --check
cargo test -p ironclaw_stress
cargo clippy -p ironclaw_stress --all-targets --all-features -- -D warnings
```

For a small runtime smoke:

```bash
cargo run -p ironclaw_stress -- \
  --backend libsql \
  --suite bottleneck-finder \
  --operations 1 \
  --concurrency 1 \
  --users 2 \
  --progress-interval-seconds 0 \
  --human-read \
  --bottleneck-report
```

## Caveats

- This tool uses synthetic workloads. It is designed to isolate infrastructure
  bottlenecks, not to perfectly replay production traffic.
- Synthetic model and tool waits are controlled sleeps. They help separate
  external latency from storage/runtime overhead.
- Provider model latency is opt-in with `--model-latency-source provider` and
  requires live provider credentials. It is intentionally blocked from suite,
  ramp, sweep, and repeated-run modes to avoid accidental token spend.
- Failed synthetic tool results do not necessarily fail the whole operation.
  They exercise the failed-tool transcript path.
- `turn_thread_busy` is often expected in hot-thread tests because the same
  thread is intentionally serialized.
- Long-run soak results are more meaningful in release builds.
