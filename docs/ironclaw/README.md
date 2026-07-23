# IronClaw Architecture and Harness Map

This page is the agent-facing map for the default IronClaw workspace, its
contracts, validation harness, and local evidence.

This page is intentionally short. Use it for progressive disclosure: start here, then follow the smallest relevant repo-local source instead of loading every IronClaw file into context.

## Current IronClaw sources in this branch

This repo exposes IronClaw structure primarily through implementation crates, crate-local agent docs, tests, and CI guardrails.

| Need | Start with |
| --- | --- |
| Standalone IronClaw binary | `docs/ironclaw-binary.md` |
| Standalone IronClaw onboarding | `docs/ironclaw/onboarding.md` |
| Retained compatibility identifiers | `docs/ironclaw/legacy-identifiers.md` |
| Production cutover readiness closeout | `docs/ironclaw/production-cutover-readiness-closeout.md` |
| Standalone IronClaw Slack setup | `docs/ironclaw/setup-slack-for-ironclaw-binary.md` |
| Porting v1 channels to IronClaw surfaces/ProductAdapters | `docs/ironclaw/how-to-port-channel-to-ironclaw.md` |
| Proposed subagent spawn design | `docs/ironclaw/subagent-spawn/README.md` |
| Host API vocabulary | `crates/ironclaw_host_api/` |
| Host API local rules | `crates/ironclaw_host_api/CLAUDE.md` |
| Host/runtime composition and shared runtime HTTP egress | `crates/ironclaw_host_runtime/` |
| Architecture dependency guardrails | `crates/ironclaw_architecture/` |
| IronClaw dependency-boundary tests | `crates/ironclaw_architecture/tests/ironclaw_dependency_boundaries.rs` |
| Events substrate | `crates/ironclaw_events/` |
| Event projection read models | `crates/ironclaw_event_projections/` |
| Standalone durable event/audit stores | `crates/ironclaw_event_store/` |
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
| Replay / recorded-model fixtures | `tests/fixtures/llm_traces/README.md` |
| Recorded-fixture gate | `.github/workflows/ironclaw-tests.yml` (`IronClaw QA recorded fixtures` job) + `scripts/ci/check-ironclaw-qa-fixtures.sh` |
| E2E test harness | `tests/e2e/README.md` |

## IronClaw contract docs

Start with these common IronClaw contract docs and prefer the full in-tree
`docs/ironclaw/contracts/` set over older design notes:

```text
docs/ironclaw/contracts/_contract-freeze-index.md
docs/ironclaw/contracts/host-api.md
docs/ironclaw/contracts/capability-access.md
docs/ironclaw/contracts/dispatcher.md
docs/ironclaw/contracts/events-projections.md
docs/ironclaw/contracts/triggers.md
docs/ironclaw/contracts/memory.md
docs/ironclaw/contracts/secrets.md
docs/ironclaw/contracts/network.md
docs/ironclaw/contracts/skills-extension.md
docs/ironclaw/contracts/migration-compatibility.md
```

If a topic is not covered there yet, use the crate-local `CLAUDE.md` files, public crate APIs, and architecture tests as the branch-local source of truth.

## Harness docs

| Harness area | Doc |
| --- | --- |
| Local per-worktree environment | `docs/ironclaw/harness/local-dev.md` |
| Replay and compatibility fixtures | `docs/ironclaw/harness/replay.md` |
| Logs, events, traces, debug bundles | `docs/ironclaw/harness/observability.md` |
| Change-category landing policy for review | `docs/ironclaw/harness/landing-policy.md` |

## Existing harness assets

IronClaw should reuse the existing IronClaw harness where possible:

- `scripts/replay-snap.sh`
- `tests/fixtures/llm_traces/README.md`
- `.github/workflows/ironclaw-tests.yml` (IronClaw crate/root/integration/QA gates)
- `.github/workflows/ironclaw-e2e.yml`
- `.github/workflows/live-canary.yml`
- `scripts/check_no_panics.py`

(The v1 `replay-gate.yml`, `e2e.yml`, `tests/support/LIVE_TESTING.md`, and
`scripts/check_gateway_boundaries.py` were removed under Tier B; IronClaw
dependency/composition boundaries are enforced by
`cargo test -p ironclaw_architecture`.)

## Harness principles

1. Humans steer with issues, docs, plans, compatibility manifests, and acceptance criteria.
2. Agents execute with isolated worktrees, deterministic fixtures, replay traces, E2E artifacts, and mechanical guardrails.
3. `AGENTS.md` remains a quick-start map, not the full architecture spec.
4. IronClaw details should live in repo-local docs, crate-local `CLAUDE.md` files, tests, and scripts.
5. Architecture boundaries should be mechanically enforced where possible.
6. Product-surface compatibility should be proven through replay, E2E, and compatibility evidence before cutover.

## Golden boundaries

Preserve these IronClaw boundaries unless the relevant contract or architecture test is deliberately changed:

1. `ironclaw_host_api` stays vocabulary/contract-only.
2. `ironclaw_architecture` stays test-only architecture enforcement.
3. Low-level substrate crates should not depend upward on product/runtime orchestration.
4. Product flows should not bypass authorization, approval, resource, network, secret, or event boundaries.
5. Secrets and credential material must not appear in user-facing errors, logs, events, snapshots, or debug bundles.
6. Persistence behavior that becomes production-facing must preserve PostgreSQL/libSQL parity unless explicitly scoped otherwise.
7. Caller-level tests are required when a helper gates a side effect.

## Related tracking issues

- IronClaw substrate/cutover parent: #2987
- IronClaw compatibility gate: #3020
- IronClaw product-surface migration: #3031
- IronClaw lifecycle UX realignment: `docs/ironclaw/2026-05-24-3288-lifecycle-ux-realignment.md`
