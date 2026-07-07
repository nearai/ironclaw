# Goal: prove Reborn is the production path for NEAR Foundation use

Source page: https://app.notion.com/p/36e29a6526bf8042b267c52a1dab02cd

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` as an umbrella acceptance spec for the Reborn stack. The spec must name the production entrypoint, the surfaced product capabilities, the v1 behavior that is explicitly out of scope, and the evidence required before a lane can be called production-ready.

Minimum Stage 0 checks:

- Build and run the Reborn serve graph from `crates/ironclaw_reborn_cli`.
- Read repo `AGENTS.md`, `CLAUDE.md`, relevant `crates/*/AGENTS.md`, and the local docs for Reborn composition, product workflow, WebUI v2, runtime, and frontend.
- Create hermetic fake-provider scenarios for WebUI turns, routines, connector/channel routing, secret mediation, and operator diagnostics.
- Keep existing Reborn unit, crate, and integration tests green before any outer-loop scoring.

## Target (outer loop)

Optimize a weighted rollout-readiness score:

- 30% WebUI authenticated turn submission, event streaming, and recovery behavior.
- 20% routines and long-running run-profile behavior.
- 20% connector/channel routing for Slack, Google Suite, and Notion/MCP.
- 15% secret, network, approval, and host-runtime mediation.
- 15% operator diagnostics, health, and recovery after partial failure.

Bar: at least 0.90 on holdout, zero security-critical failures, zero new behavior implemented through a v1 fallback path. Score with `harness/score.sh`; holdout is aggregate-only with at most 3 calls per 24 hours. Acceptance is holdout-only.

## Eval design

Create 40 dev and 80 holdout Reborn QA traces. Each trace input is a task narrative plus fake external service events. Scorer-owned answers are expected route, run state, emitted events, tool or connector side effects, security redaction outcomes, and user-visible terminal state.

The eval must include happy paths, auth failures, connector revocation, duplicate event delivery, partial service outage, denied tool calls, routine scheduling, and operator recovery. Holdout answers must live outside the repo.

## Harness design

Extend existing Reborn QA fixture infrastructure where possible, especially `scripts/reborn_qa_matrix` and WebUI live QA helpers. Prefer event/state diffs over final text matching. `harness/score.sh` must:

- Run `harness/lint.sh` and checksum harness files before scoring.
- Start hermetic fake providers and the Reborn composition under test.
- Replay trace inputs and collect event envelopes, route decisions, persisted state, and side effects.
- Score both false negatives and false positives, including "responded when it should not" and "used v1 path" failures.
- Calibrate against one known-good trace bundle and one known-bad bundle before scoring live changes.

## Constraints

- Wall-clock budget: 16 hours.
- Spend ceilings: $25 in LLM/API spend; no live external SaaS calls except pre-approved canaries.
- Surface allowlist: `crates/ironclaw_reborn_cli`, `crates/ironclaw_reborn_composition`, `crates/ironclaw_product_workflow`, `crates/ironclaw_webui_v2`, `crates/ironclaw_webui_v2_static`, host runtime, connector adapters, Reborn QA scripts, and tests. Legacy `src/` is read-only except for deletion evidence or parity checks.
- Capacity caps: shared caps from `../COMMON.md`; additionally, rollout scenario fixtures visible to product code are capped at 20.
- Documentation: update `FEATURE_PARITY.md` or Reborn subsystem docs if readiness status changes.

## Cycle protocol

Follow the common cycle protocol. Before each change, record which readiness dimension should move and which trace should distinguish real progress from fixture overfitting. Every cycle must run Stage 0 tests plus dev scoring; holdout can be called only after a meaningful dev/probe gain.

## Entropy rules

- Every third cycle must target a different readiness dimension than the previous two cycles.
- If the dev score improves but the probe gap worsens, remove eval-shaped routing, fixture, or prompt artifacts before adding new behavior.
- After a flat cycle, the next change must alter a boundary or integration seam, not just tune assertion wording or retries.

## Cheat audit

Lane-specific cheap wins to block:

1. Route every trace through a single "Reborn success" path; detect by route and side-effect diffs.
2. Keep v1 fallback alive for failing cases; lint bans new Reborn behavior implemented through legacy `src/`.
3. Make final prose claim success while events fail; scorer ignores prose unless state also matches.
4. Hardcode QA trace ids; lint rejects visible trace-id branches.
5. Stub connector success without side effects; fake providers assert actual requests.
6. Hide failed approvals by not emitting events; scorer expects denial and recovery events.
7. Disable failing routines; scheduler state is part of the score.
8. Loosen auth or origin checks in tests; lint checks security-sensitive config diffs.
9. Count live canary success as full acceptance; holdout hermetic score is authoritative.
10. Delete or skip flaky tests; Stage 0 fails on skipped required Reborn checks.

## Stop conditions

Stop when the holdout bar is hit with green Stage 0 tests, any budget is exhausted, readiness score is flat for 3 cycles, or a critical auth, secret, tenant, approval, or data-loss issue is discovered. On stop, write the final report to `LOG.md` with per-dimension scores and remaining rollout risks.

