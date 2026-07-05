# Hosted Single-Tenant Latency Harness

This harness compares libSQL and PostgreSQL latency through the real
`ironclaw_filesystem::RootFilesystem` implementations.

PostgreSQL pool size is part of the score. By default each scorer invocation
runs Postgres at pool sizes `1` and `2` and compares both result sets to the
same libSQL baseline sample. Do not raise the pool size to pass this goal.

Current scope is storage hot paths:

- `put_get`
- `query_exact`
- `append_tail`
- `reserve_sequence`
- `hosted_substrate_build`

`hosted_substrate_build` uses the exported Reborn production substrate builders
with deterministic fake process and wake ports. It exercises hosted
filesystem-backed secrets, resources, approvals, run-state, triggers, event
store setup, and production wiring validation without live providers.

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
