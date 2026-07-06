# Goal: Match Hosted Single-Tenant Postgres Latency To LibSQL

## Stage 0 - Build To Spec

Implement `spec.md`. The inner loop is green only when the latency harness runs
successfully, product tests relevant to the touched area pass, and the harness
still labels incomplete coverage as dev-only. Do not claim acceptance until the
full hosted profile workload compares `hosted-single-tenant` Postgres against a
clean `hosted-single-tenant-volume` libSQL launch reference.

## Target

For every held-out hosted scenario, concurrency level, and Postgres pool size
`1` and `2`, Postgres must match the libSQL baseline:

- `p50 <= max(libSQL_p50 * 1.10, libSQL_p50 + 3ms)`
- `p95 <= max(libSQL_p95 * 1.15, libSQL_p95 + 8ms)`
- `p99 <= max(libSQL_p99 * 1.25, libSQL_p99 + 15ms)`
- throughput at least `90%` of libSQL
- error count no higher than libSQL
- deterministic state hashes match

Hard fail on pool starvation, deadlocks, skipped durable writes, lost state
transitions, benchmark-only fast paths, live LLM/network calls, or raising the
scored Postgres pool beyond `1,2`.

Score with `harness/latency/score.sh`. A `VOID` result means a constraint was
violated; remove the violation and rerun. Holdout acceptance is aggregate-only
and may be run at most once per cycle after dev and probe are clean.

## Constraints

- Wall-clock budget: 10 hours. Run `harness/latency/status.sh` every cycle.
- Spend ceiling: $0. No paid model, crawler, hosted provider, or external API.
- Surface: this repository, local Postgres, local shell tools, deterministic
  local fixtures, and harness artifacts. No live LLM/provider/network workload.
- Capacity caps: benchmark-specific literal path/prefix lists <= 20 entries;
  special-case branches for named workloads <= 0 in production code; harness
  diagnostic knobs must be documented and excluded from acceptance.
- Pool cap: scored Postgres pool sizes are exactly `1` and `2`. Larger pools
  are diagnostic-only and void acceptance.
- `goal.md`, `spec.md`, and `harness/latency/` become read-only after the
  first full hosted dev score is verified. Before that, edits are allowed only
  to close harness correctness gaps and must be logged in `LOG.md`.
- Eval inputs may be visible through the harness. Holdout answers and launch
  reference state hashes must not be copied into product code or fixtures.

## Cheat Audit

- Slow libSQL down with sleeps or extra work -> score both backends through the
  same workload code and lint for sleep/bench fast-path markers.
- Raise Postgres pool size -> scorer emits `postgres_pool_size`; lint rejects
  default scored pools outside `1,2`.
- Skip writes or readiness checks -> state hashes and product tests must match.
- Cache benchmark responses -> state hash must depend on real readback/query
  results; production code cannot branch on harness paths.
- Use live LLM/network shortcuts -> workload must run with local fakes only.
- Edit scorer thresholds after seeing failures -> every harness edit needs a
  pre-change hypothesis and result in `LOG.md`.
- Declare dev victory -> acceptance is holdout-only.
- Memorize exact fixture paths -> probe perturbs path depth and payload size;
  growing probe gap forces removal of eval-shaped artifacts.
- Hide errors in aggregate latency -> scorer reports error counts and first
  error; any Postgres error hard-fails the row.
- Change schema without parity -> filesystem tests must pass with libSQL and
  Postgres features.

## Cycle Protocol

1. Run `harness/latency/status.sh`.
2. Run `harness/latency/score.sh --dev`.
3. Run `harness/latency/probe.sh`.
4. Write the next `LOG.md` hypothesis, expected failure mode, and diagnostic
   before changing code.
5. Make the smallest production or harness change that tests the hypothesis.
6. Run targeted tests and rerun dev/probe score.
7. Log the result and checkpoint the cycle with a commit when the cycle is
   coherent and stageable.

## Entropy Rules

- Stall rule: if dev/probe metrics do not improve for one cycle, the next cycle
  must inspect a different layer: hosted workflow, operation count, SQL plan,
  schema/index, or transaction shape.
- Exploration quota: every third cycle must try a structurally different
  approach or explicitly justify why the current bottleneck is still unproven.

## Stop Conditions

Stop when the holdout bar is hit, any budget is exhausted, or marginal gain is
approximately zero for three consecutive cycles. On stop, write a final report
in `LOG.md` with best score, what generalized, what was abandoned, and the
highest-leverage next steps.
