# LFD Brief: near-foundation-must-haves — the rollout bundle (lane 02)

**State**: bundle-verification loop with small code deltas, NOT feature-building
(LANE-ADDENDA lane 02). **Bar**: 1.00 must-have coverage AND ≥ 0.92 weighted
holdout; zero auth/tenant/secret/cross-user leakage defects. **Profile**:
`nf_bundle`. **Wave**: 3, after lanes 03 (Slack) and 05 (onboarding) — it
consumes their surfaces.

## Outcome

Six must-haves are provably rollout-ready end to end, each with a login →
connector setup → turn → approval/tool-call → recovery journey passing on
holdout. The bundle verifies the pillars compose; it does not re-build them.
Nice-to-haves (Slack team-agent context, admin skills, IronHub, admin-propagated
tools) appear only as distractors and earn ZERO until every must-have gate passes.

## The six must-haves (goal.md Stage 0 checklist)

1. Google Suite — Gmail, Calendar, Drive, Sheets, Docs (app-specific, not one op).
2. Slack relay (ingress → response routing; consumes lane 03).
3. Notion through MCP (configured product-capability path).
4. WebUI with Google OAuth.
5. Routines (create/execute).
6. Hosted deployment readiness (hosted-like config, not production creds).

## Spec sources (generator: synthesize spec.md — rollout checklist)

- `docs/lfd/roadmap-blue-lanes-2026-07-07/02-near-foundation-must-haves/goal.md` (weights, hard gates, cheat audit — binding).
- Lane 03/05 packages — the Slack + OAuth/onboarding surfaces this bundle drives.
- `tests/e2e/` Emulate fixtures: `fixtures/emulate/{google_gmail,slack,github}.yaml`, `emulate_provider.py`, `reborn_emulate_harness.py`, `scenarios/test_emulate_reborn_provider_contracts.py`, `scenarios/test_oauth_refresh.py`, and the `hosted_google_emulate_server` / `emulate_slack_server` conftest fixtures (`tests/e2e/CLAUDE.md` §fixtures).
- Notion MCP: `crates/ironclaw_first_party_extensions/assets/notion-mcp/` (manifest + prompts), `crates/ironclaw_extensions/src/hosted_mcp_discovery.rs`, `src/tools/mcp/factory.rs` (transport dispatch).
- Hosted config: `crates/ironclaw_reborn_cli/src/commands/{serve,serve_sso}.rs`, `config/init.rs`, `operator_env.rs`.
- Routines/turns: `crates/ironclaw_triggers`, `crates/ironclaw_turns`, `crates/ironclaw_run_state`.

## Stage 0 inner suite

e2e smoke subset (`test_emulate_reborn_provider_contracts.py`, `test_oauth_refresh.py`)
+ `cargo test --features integration` groups touched by the bundle. One
caller-level/E2E test plan per must-have and one recovery case per auth/connector
boundary (goal.md Stage 0). Green every cycle.

## Eval themes (dev ~12 / holdout ~18 — 2 dev + 3 holdout journeys per must-have)

The lane goal's 10/30 and larger counts are DESIGNER GROWTH TARGETS; launch set is
~12 dev / ~18 holdout. Every journey = login + connector setup + a turn + an
approval/tool-call + a recovery branch; some are mixed (e.g. "Slack request →
summarize a Drive doc → Notion page").

1. Per-must-have happy journey (6): backend connector state persisted (state_pred) + app-specific fake-provider egress at the seeded Emulate endpoint; NOT UI-button visibility (goal cheat 5).
2. Recovery branch per auth/connector boundary (≥25%): auth expiry, connector revocation, missing permission, external API failure → typed recovery, no dead-end setup state (required recovery event; forbidden silent-drop).
3. Mixed cross-connector journeys (3): touch ≥2 connectors; each connector's own egress required.
4. Over-permissive negatives: a call with another user's/tenant's token FAILS closed (required `status:"error"`/denial) — over-permission priced worse than a missing call (goal.md).

## Feature-specific cheats → fences

- **Mock-only connector counted ready** (critical) → journeys assert real request shape against the Emulate fixtures: egress must hit the seeded provider endpoint + mutation (`test_emulate_reborn_provider_contracts.py` shape — e.g. Gmail `messages/msg_emulate_unread`, Slack `auth.test`), not a blanket `{"ok":true}` http_stub. A connector with no matched seeded egress scores its must-have 0.
- **Single omnipotent test user** (critical) → journeys swap users/tenants; contracts pin the credential subject (state_pred on token owner / OAuth subject); the over-permissive negative (theme 4) zeroes any allow-any-token path.
- **Nice-to-have credit** → encoded as contract weights, not prose: the nice-to-have contribution coefficient is a PINNED gate-multiplier = 0 until all six must-have hard gates (≥1 passing holdout journey each) pass.
- **Collapse Google Suite into one fake op** → each sub-app (Gmail, Calendar, Drive, Sheets, Docs) carries ≥1 required egress matcher against its own Emulate endpoint; one-op journeys miss four numerators.
- **Hardcode Notion MCP tool/workspace ids** → probe swaps workspace + tool ids; caps below reject notion id literals in diff.
- **Bypass hosted config with local defaults** → harness launches hosted-like config; state_pred asserts hosted flags + proxy credential injection (`hosted_google_emulate_server`: `GOOGLE_OAUTH_CLIENT_ID=hosted-google-client-id`).
- **OAuth/connector token in logs** → leak scan over event + transcript artifacts (COMMON cheat 10); `leak` forbidden on every journey.

## caps.json extras

`{"name":"notion-id-literals","paths":["crates/**","src/**"],"pattern":"(notion.so/[A-Za-z0-9-]{16,}|ntn_[A-Za-z0-9]{8,})","max_count":0}`
(probe-renamable Notion workspace/tool ids), plus Emulate bearer/token literals
(`mock-refreshed-access-token`, `emulate-slack-token`, `ghp_emulate_github_token`)
in diff max 0, plus rollout-checklist fixtures capped at one seed per must-have per
provider (goal.md), plus COMMON caps.

## Live mode

3 live cases (cases-live/): real model drives one mixed cross-connector journey
(Slack → Drive → Notion) and two single-connector journeys over the Emulate
providers (scripted external state, live LLM). Supplemental evidence only —
holdout hermetic score is authoritative (goal cheat 10). Ceiling $20; $0 live
Google/Slack/Notion except disposable limited canary accounts. Stage LAST.
