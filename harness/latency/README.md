# Hosted Single-Tenant Latency Harness

This harness compares libSQL and PostgreSQL latency through real hosted
persistence paths. It includes root filesystem hot paths plus selected
production-shaped control-plane stores.

PostgreSQL pool size is part of the score. By default each scorer invocation
runs Postgres at pool sizes `1` and `2` and compares both result sets to the
same libSQL baseline sample. Do not raise the pool size to pass this goal.

Current dev scope:

- `put_get`
- `query_exact`
- `append_tail`
- `reserve_sequence`
- `trigger_seed_list`
- `control_plane_snapshot`
- `hosted_substrate_build`

`hosted_substrate_build` uses the exported Reborn production substrate builders
with deterministic fake process and wake ports. It exercises hosted
filesystem-backed secrets, resources, approvals, run-state, triggers, event
store setup, and production wiring validation without live providers.

`control_plane_snapshot` performs timed approval-request, secret
metadata/lease/consume, and resource-governor reserve/reconcile operations.
It is currently diagnostic: Postgres uses row-backed secret and resource
stores in this harness, while libSQL stays on the production filesystem-backed
stores. The workload validates that the control-plane row stores remove the
single-blob contention path before those stores are wired into production
composition.

It is a dev scorer, not the full acceptance gate yet. The spec requires future
cycles to add launch-reference baseline scoring, hosted profile startup,
WebUI/session, turn admission/resume/cancel, and request-level
triggers/approvals/secrets/resources.

## Run

```bash
export IRONCLAW_REBORN_POSTGRES_URL=postgres://postgres:postgres@localhost:5432/ironclaw_latency
harness/latency/score.sh --dev
```

Override the scored pool list only for diagnostics:

```bash
LATENCY_POSTGRES_POOL_SIZES=1,2 harness/latency/score.sh --dev
```

Use `harness/latency/probe.sh` for a perturbed workload mix.
