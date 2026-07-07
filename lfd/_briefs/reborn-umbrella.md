# LFD Brief: reborn-umbrella — Reborn is the production path (lane 01)

**State**: meta-rollup, NOT a code-editing loop (LANE-ADDENDA lane 01, REVIEW
finding 5). **Bar**: 0.90 holdout, zero security-critical failures, zero new
behavior through a v1 `src/` path. **Profile**: `reborn_umbrella`. **Wave**: 4
— runs last / as a standing dashboard once pillars have holdout numbers.

## Outcome

A single readiness score proves the Reborn stack (`crates/ironclaw_reborn_cli`
serve graph, binary `ironclaw-reborn`) is the production path. The umbrella
does NOT re-build any pillar: it rolls up the pillar lanes' sealed holdout
aggregates into the goal's five readiness dimensions, and adds 15–20 of its own
hermetic cross-cutting scenarios (authenticated WebUI turn + event-stream
recovery, routine execution, connector route, secret mediation, operator
recovery after partial failure) that exercise the composition end to end.

## Spec sources (generator: synthesize spec.md — umbrella acceptance spec)

- `docs/lfd/roadmap-blue-lanes-2026-07-07/01-reborn-umbrella/goal.md` (five weighted dimensions + cheat audit are binding).
- `scripts/reborn_qa_matrix/` (`run_hermetic_qa.py`, `report_coverage.py`) + `tests/fixtures/llm_traces/reborn_qa/` (routine_*, web_*, connect_gmail) — fixture infra the own-scenarios extend, not fork.
- `crates/ironclaw_reborn_cli/src/commands/{serve,serve_sso}.rs`, `runtime/mod.rs`, `operator_env.rs` — production entrypoint booted hermetically.
- Pillar goal.md + briefs for the dimension mapping only: lanes 02, 03, 04, 05, 08, 11.
- **Path fix (LANE-ADDENDA)**: goal.md's allowlist names `crates/ironclaw_webui_v2_static` — it does not exist; static assets are `crates/ironclaw_webui_v2/{static,frontend}/`. Record the substitution in LOG.md. The umbrella touches neither — see the surface fence.

## Rollup mapping (weights = goal.md dimensions → pillar lanes)

Dimension score = `α·mean(mapped pillar holdout aggregates) + (1−α)·(own-scenario
cluster)`. Weights and α are PINNED in the rollup config (pins.json); an edit
breaks pins → holdout rejected.

| Dim (weight) | Pillar holdout rollup | Own scenarios (α<1) |
| --- | --- | --- |
| WebUI auth turn/stream/recovery (30%) | lane **05** webui_v2 auth | auth turn + SSE recovery |
| Routines / long-running (20%) | lane **11** missions | routine execute + schedule state |
| Connector/channel routing (20%) | lane **03** Slack + lane **02** bundle (Google/Notion·MCP) | route + revocation |
| Secret/network/approval/host-runtime (15%) | lane **04** secrets + lane **08** approvals | secret-mediation turn |
| Operator diagnostics/health/recovery (15%) | none → **α=0**, own only | partial-failure → recovery |

Pillar aggregates are read ONLY from the pinned off-repo audit log
(`$LFD_STATE_ROOT/audit/<pillar>.log`) latest sealed holdout entry; a
missing/stale number scores that rollup term **0**, never skipped. The umbrella
never reaches into another lane's `lfd/` package.

## Stage 0 inner suite

`cargo test --features integration --test integration` (full Reborn integration
target) + webui-v2 JS tests. The umbrella writes no crate code, so Stage 0 is a
green-gate on the composition it scores; skipped required Reborn checks fail
Stage 0 (goal cheat 10). Green every cycle.

## Eval themes (own cross-cutting scenarios: dev ~18 / holdout ~10)

Dev set is the 15–20 own scenarios; pillar rollup terms come from sealed pillar holdouts, not authored here.

1. WebUI authenticated turn (5): login → submit → stream → recovery after mid-stream disconnect (required TurnCompleted + resumed-stream; forbidden `used_v1_path`).
2. Routine execution (3): create/execute, scheduler state persisted (state_pred), duplicate delivery deduped.
3. Connector route (4): Slack + Google/Notion·MCP route to the right capability; revocation → typed denial (forbidden egress-after-revoke).
4. Secret mediation (3): credentialed tool → lease/inject/redact; `leak` forbidden across replies+events+tool outputs; denied approval → no secret.
5. Operator recovery (3): inject partial outage → degraded diagnostics surface the failing subsystem (required recovery event) WITHOUT leak (forbidden `leak`), then recover.

Failure-direction ≥ 25%: revocation, auth failure, partial outage, denied tool, duplicate delivery → fail-soft (`status:"ran"` + typed events, never crash).

## Feature-specific cheats → fences

- **Umbrella re-implements pillar behavior to inflate the rollup** (critical) → surface allowlist = rollup/scoring config + its own scenario cases + `tests/integration/lfd/profiles/reborn_umbrella.rs` ONLY; any `crates/**` or `src/**` diff = VOID (caps below). A dimension moves only via the pillar's own sealed holdout, never by the umbrella fixing pillar code.
- **Cherry-pick a stale/high pillar aggregate** → rollup reads only the latest sealed holdout entry per pillar from the pinned audit log; stale/missing → 0; rollup config + reader pinned (pins.json).
- **Weight-gaming** (down-weight a failing dimension) → weights + α pinned; any edit breaks pins → holdout rejected from a non-pristine runner.
- **Own-scenario prose-success while state fails** → scorer ignores prose unless state/events match (goal cheat 3); every own scenario has ≥1 required state/egress/event matcher, none reply-only.
- **Operator scenario always-healthy stub** → operator cases REQUIRE an injected partial failure AND a recovery event AND a non-leaking diagnostic (paired required-recovery-event + forbidden `leak`).

## caps.json extras

`{"name":"umbrella-no-product-code","paths":["crates/**","src/**"],"pattern":".","max_count":0}`
(any added product-code line VOIDs — the umbrella is scoring config only), plus
rollout-scenario fixtures visible to product code capped at 20 (goal.md), plus COMMON caps.

## Live mode

2 live cases: real model drives the authenticated WebUI turn + operator-recovery
own-scenarios over the hermetic composition (scripted external events, live LLM).
Stage LAST, after the dev own-scenario bar is near and pillar holdouts exist.
Ceiling $25 (goal.md); no live external SaaS except pre-approved canaries.
