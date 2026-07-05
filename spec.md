# Hosted Single-Tenant Postgres Latency Spec

## Outcome

`hosted-single-tenant` must keep the hosted runtime/control-plane behavior of
the existing single-tenant surface while using PostgreSQL with latency close to
the `hosted-single-tenant-volume` libSQL baseline. The migration may change
PostgreSQL schema and indexes, but it must not change externally visible
runtime state, skip durable writes, or rely on larger-than-production pool
sizes.

## Baseline and Treatment

- Baseline: `hosted-single-tenant-volume` semantics over `LibSqlRootFilesystem`.
- Treatment: `hosted-single-tenant` semantics over `PostgresRootFilesystem`.
- The latency harness must run the same operation stream against both backends.
- Full acceptance must pin the libSQL baseline to a clean launch-reference
  worktree. Dev scoring may use the current checkout for initial calibration,
  but must label that result as dev-only.

## Workloads

The harness must grow toward these deterministic workloads:

- cold and warm `ironclaw-reborn serve` startup to `/api/health`
- WebUI health/session request paths
- local-runtime turn admission, queue, resume, and cancel paths
- filesystem `put`, `get`, `query`, `append_batch`, `tail`, and
  `reserve_sequence`
- trigger access seed/list paths
- approvals, secrets, and resource snapshot paths

No workload may call a live model, hosted provider, external network service, or
non-deterministic LLM/tool surface. Use local fixtures and fakes for everything
outside storage.

## Metrics

For each scenario and concurrency level `1`, `4`, and `16`, collect warmup
samples before measured samples. Full scoring must use at least 30 warmup and
300 measured samples per backend. Dev scoring may use smaller sample counts for
iteration speed.

For each scenario/backend/concurrency tuple, report:

- sample count
- error count
- throughput operations per second
- p50, p95, and p99 latency in milliseconds
- deterministic state hash

Acceptance is holdout-only:

- Postgres `p50 <= max(libSQL_p50 * 1.10, libSQL_p50 + 3ms)`
- Postgres `p95 <= max(libSQL_p95 * 1.15, libSQL_p95 + 8ms)`
- Postgres `p99 <= max(libSQL_p99 * 1.25, libSQL_p99 + 15ms)`
- Postgres throughput is at least 90% of libSQL throughput
- Postgres error count is not higher than libSQL
- produced state hashes match for equivalent workloads

Hard fail if any scenario has Postgres `p95 > 1.5x` libSQL, `p99 > 2x` libSQL,
pool starvation, deadlock, skipped durable write, or missing state transition.

## Constraints

- Do not slow the libSQL baseline to make Postgres look better.
- Do not increase hosted Postgres scoring pool size beyond `1` and `2`.
- Do not add benchmark-only fast paths, env flags, path-name special cases,
  response caches, in-memory replacements, fake readiness shortcuts, or skipped
  persistence.
- Harness output must distinguish dev score from holdout/acceptance score.
- `harness/latency/score.sh` is the scoring entry point.

