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
- `turn_lifecycle`
- `webui_session`
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
single-blob contention path. Production hosted Postgres composition also uses
the row-backed resource governor.

`turn_lifecycle` exercises the durable turn-state path through
`ScopedFilesystem`. libSQL and Postgres both use the filesystem row store so
the comparison measures backend behavior instead of the old full-snapshot blob
CAS path.

`webui_session` builds the real
`build_reborn_runtime -> RebornRuntime::product_surface -> webui_v2_app` stack once per
backend, then measures authenticated `GET /api/webchat/v2/session` requests
through the composed Axum router. It uses deterministic multi-user bearer
tokens so the workload measures normal session-bootstrap latency instead of
the per-caller read-rate limiter after the first 120 requests.

It is a dev scorer, not the full acceptance gate yet. The spec requires future
cycles to add launch-reference baseline scoring and request-level
trigger/approval/secret/resource flows.

## Run

```bash
export IRONCLAW_REBORN_POSTGRES_URL=postgres://postgres:postgres@localhost:5432/ironclaw_latency
harness/latency/score.sh --dev
```

Override the scored pool list only for diagnostics:

```bash
LATENCY_POSTGRES_POOL_SIZES=1,2 harness/latency/score.sh --dev
```

Run a single backend only for diagnostics, never acceptance. `score.sh` voids
filtered-backend runs unless they are explicitly marked diagnostic:

```bash
LATENCY_BACKENDS=postgres LATENCY_ALLOW_DIAGNOSTIC_BACKENDS=1 harness/latency/score.sh --dev
```

Use `harness/latency/probe.sh` for a perturbed workload mix.
