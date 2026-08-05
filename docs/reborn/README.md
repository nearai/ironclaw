# Reborn Harness Map

Reborn is IronClaw's host/runtime integration work. This page is the agent-facing map for Reborn harness, validation, and local evidence.

This page is intentionally short. Use it for progressive disclosure: start here, then follow the smallest relevant repo-local source instead of loading every Reborn file into context.

## Current Reborn sources in this branch

This repo exposes Reborn structure primarily through implementation crates, crate-local agent docs, tests, and CI guardrails.

| Need | Start with |
| --- | --- |
| Standalone Reborn binary | `crates/app/ironclaw_cli/` and `docs/reborn/onboarding.md` |
| Standalone Reborn onboarding | `docs/reborn/onboarding.md` |
| Production cutover readiness closeout | `docs/reborn/production-cutover-readiness-closeout.md` |
| Standalone Reborn Slack setup | `docs/reborn/setup-slack-for-reborn-binary.md` |
| Porting v1 channels to Reborn surfaces/ChannelAdapters | `docs/reborn/how-to-port-channel-to-reborn.md` |
| Proposed subagent spawn design | `docs/reborn/subagent-spawn/README.md` |
| Host API vocabulary | `crates/contracts/ironclaw_host_api/` |
| Host API local rules | `crates/contracts/ironclaw_host_api/CLAUDE.md` |
| Host/runtime composition and shared runtime HTTP egress | `crates/kernel/ironclaw_host_runtime/` |
| Architecture dependency guardrails | `crates/app/ironclaw_architecture_tests/` |
| Reborn dependency-boundary tests | `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs` |
| Events substrate | `crates/events/ironclaw_event_log/` |
| Event projection read models | `crates/events/ironclaw_event_projections/` |
| Standalone durable event/audit stores | `crates/events/ironclaw_event_store/` |
| Filesystem substrate | `crates/substrates/ironclaw_filesystem/` |
| Network policy and HTTP transport substrate | `crates/substrates/ironclaw_network/` |
| Secrets metadata and one-shot leases | `crates/substrates/ironclaw_secrets/` |
| Resource governor substrate | `crates/kernel/ironclaw_resources/` |
| Authorization substrate | `crates/kernel/ironclaw_authorization/` |
| Approval substrate | `crates/kernel/ironclaw_approvals/` |
| Process lifecycle state | `crates/kernel/ironclaw_processes/` |
| Approval and gate state | `crates/kernel/ironclaw_approvals/` |
| WASM runtime lane and WIT HTTP adapter | `crates/lanes/ironclaw_wasm/` |
| Script runtime lane and host HTTP adapter | `crates/lanes/ironclaw_sandbox/` (`src/script.rs`) |
| MCP runtime lane and host-mediated HTTP/fail-closed process policy | `crates/lanes/ironclaw_mcp/` |
| Replay / recorded-model fixtures | `tests/fixtures/llm_traces/README.md` |
| Recorded-fixture gate | `.github/workflows/reborn-tests.yml` (`Reborn QA recorded fixtures` job) + `scripts/ci/check-reborn-qa-fixtures.sh` |
| E2E test harness | `tests/e2e/README.md` |

## Reborn contract docs

Start with these common Reborn contract docs and prefer the full in-tree
`docs/reborn/contracts/` set over older design notes:

```text
docs/reborn/contracts/_contract-freeze-index.md
docs/reborn/contracts/host-api.md
docs/reborn/contracts/capability-access.md
docs/reborn/contracts/dispatcher.md
docs/reborn/contracts/events-projections.md
docs/reborn/contracts/triggers.md
docs/reborn/contracts/memory.md
docs/reborn/contracts/secrets.md
docs/reborn/contracts/network.md
docs/reborn/contracts/skills-extension.md
docs/reborn/contracts/migration-compatibility.md
```

If a topic is not covered there yet, use the crate-local `CLAUDE.md` files, public crate APIs, and architecture tests as the branch-local source of truth.

## Harness docs

| Harness area | Doc |
| --- | --- |
| Local per-worktree environment | `docs/reborn/harness/local-dev.md` |
| Replay and compatibility fixtures | `docs/reborn/harness/replay.md` |
| Logs, events, traces, debug bundles | `docs/reborn/harness/observability.md` |
| Change-category landing policy for review | `docs/reborn/harness/landing-policy.md` |

## Existing harness assets

Reborn should reuse the existing IronClaw harness where possible:

- `scripts/replay-snap.sh`
- `tests/fixtures/llm_traces/README.md`
- `.github/workflows/reborn-tests.yml` (Reborn crate/root/integration/QA gates)
- `.github/workflows/reborn-e2e.yml`
- `.github/workflows/live-canary.yml`
- `scripts/check_no_panics.py`

(The v1 `replay-gate.yml`, `e2e.yml`, `tests/support/LIVE_TESTING.md`, and
`scripts/check_gateway_boundaries.py` were removed under Tier B; Reborn
dependency/composition boundaries are enforced by
`cargo test -p ironclaw_architecture_tests`.)

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
2. `ironclaw_architecture_tests` stays test-only architecture enforcement.
3. Low-level substrate crates should not depend upward on product/runtime orchestration.
4. Product flows should not bypass authorization, approval, resource, network, secret, or event boundaries.
5. Secrets and credential material must not appear in user-facing errors, logs, events, snapshots, or debug bundles.
6. Persistence behavior that becomes production-facing must preserve PostgreSQL/libSQL parity unless explicitly scoped otherwise.
7. Caller-level tests are required when a helper gates a side effect.

## Related tracking issues

- Reborn substrate/cutover parent: #2987
- Reborn compatibility gate: #3020
- Reborn product-surface migration: #3031
- Reborn lifecycle behavior: `docs/reborn/contracts/extensions.md` and
  `docs/reborn/contracts/skills-extension.md`
