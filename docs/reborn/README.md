# Reborn Harness Map

Reborn is IronClaw's host/runtime integration work. This page is the agent-facing map for Reborn harness, validation, and local evidence.

This page is intentionally short. Use it for progressive disclosure: start here, then follow the smallest relevant repo-local source instead of loading every Reborn file into context.

## Current Reborn sources in this branch

The `reborn-integration` branch currently exposes Reborn structure primarily through implementation crates, crate-local agent docs, tests, and CI guardrails.

| Need | Start with |
| --- | --- |
| Host API vocabulary | `crates/ironclaw_host_api/` |
| Host API local rules | `crates/ironclaw_host_api/CLAUDE.md` |
| Host/runtime composition and shared runtime HTTP egress | `crates/ironclaw_host_runtime/` |
| Architecture dependency guardrails | `crates/ironclaw_architecture/` |
| Reborn dependency-boundary tests | `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` |
| Events substrate | `crates/ironclaw_events/` |
| Filesystem substrate | `crates/ironclaw_filesystem/` |
| Network policy and HTTP transport substrate | `crates/ironclaw_network/` |
| Secrets metadata and one-shot leases | `crates/ironclaw_secrets/` |
| Resource governor substrate | `crates/ironclaw_resources/` |
| Authorization substrate | `crates/ironclaw_authorization/` |
| Approval substrate | `crates/ironclaw_approvals/` |
| Run-state substrate | `crates/ironclaw_run_state/` |
| WASM runtime lane and WIT HTTP adapter | `crates/ironclaw_wasm/` |
| Script runtime lane and host HTTP adapter | `crates/ironclaw_scripts/` |
| MCP runtime lane and host-mediated HTTP/fail-closed process policy | `crates/ironclaw_mcp/` |
| Replay fixtures | `tests/fixtures/llm_traces/README.md` |
| Replay workflow | `.github/workflows/replay-gate.yml` |
| E2E test harness | `tests/e2e/README.md` |
| Live/replay testing guide | `tests/support/LIVE_TESTING.md` |

## Future Reborn contract docs

When the Reborn contract-doc packet is present in this branch, agents should prefer these docs as the source of truth:

```text
docs/reborn/contracts/_contract-freeze-index.md
docs/reborn/contracts/host-api.md
docs/reborn/contracts/capability-access.md
docs/reborn/contracts/dispatcher.md
docs/reborn/contracts/events-projections.md
docs/reborn/contracts/memory.md
docs/reborn/contracts/secrets.md
docs/reborn/contracts/network.md
docs/reborn/contracts/migration-compatibility.md
```

Until then, use the crate-local `CLAUDE.md` files, public crate APIs, and architecture tests as the branch-local source of truth.

## Harness docs

| Harness area | Doc |
| --- | --- |
| Local per-worktree environment | `docs/reborn/harness/local-dev.md` |
| Replay and compatibility fixtures | `docs/reborn/harness/replay.md` |
| Logs, events, traces, debug bundles | `docs/reborn/harness/observability.md` |

## Existing harness assets

Reborn should reuse the existing IronClaw harness where possible:

- `scripts/replay-snap.sh`
- `scripts/trace-coverage.sh`
- `tests/fixtures/llm_traces/README.md`
- `tests/support/LIVE_TESTING.md`
- `.github/workflows/replay-gate.yml`
- `.github/workflows/e2e.yml`
- `.github/workflows/live-canary.yml`
- `scripts/check-boundaries.sh`
- `scripts/check_gateway_boundaries.py`
- `scripts/check_no_panics.py`

## Harness principles

1. Humans steer with issues, docs, plans, compatibility manifests, and acceptance criteria.
2. Agents execute with isolated worktrees, deterministic fixtures, replay traces, E2E artifacts, and mechanical guardrails.
3. `AGENTS.md` remains a quick-start map, not the full architecture spec.
4. Reborn details should live in repo-local docs, crate-local `CLAUDE.md` files, tests, and scripts.
5. Architecture boundaries should be mechanically enforced where possible.
6. Product-surface compatibility should be proven through replay, E2E, and compatibility evidence before cutover.

## Golden boundaries

Preserve these Reborn boundaries unless the relevant contract or architecture test is deliberately changed:

1. `ironclaw_host_api` stays vocabulary/contract-only.
2. `ironclaw_architecture` stays test-only architecture enforcement.
3. Low-level substrate crates should not depend upward on product/runtime orchestration.
4. Product flows should not bypass authorization, approval, resource, network, secret, or event boundaries.
5. Secrets and credential material must not appear in user-facing errors, logs, events, snapshots, or debug bundles.
6. Persistence behavior that becomes production-facing must preserve PostgreSQL/libSQL parity unless explicitly scoped otherwise.
7. Caller-level tests are required when a helper gates a side effect.

## Related tracking issues

- Reborn substrate/cutover parent: #2987
- Reborn compatibility gate: #3020
- Reborn product-surface migration: #3031
