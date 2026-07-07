# Goal: make onboarding lead to a working channel-first experience

Source page: https://app.notion.com/p/36e29a6526bf80a5a5fdf07e8ffe1396

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

The Notion row is in design and sparse. First write `spec.md` from product discovery before implementing. The spec must define the shortest safe flow:

1. User authenticates.
2. User connects or pairs a channel.
3. User confirms a first usable channel turn.
4. WebUI gets out of the way unless a recovery action is needed.

The spec must cover failed auth, expired state, wrong workspace, duplicate pairing, revoked channel, unpaired user, and a user who wants WebUI-only usage.

## Target (outer loop)

Optimize onboarding funnel task score:

- 30% OAuth/session creation and recovery are correct.
- 30% channel connection, pairing, and identity binding work.
- 20% first Slack or channel turn completes through the normal turn pipeline.
- 10% recovery is clear for failed auth, expired state, wrong workspace, duplicate pairing, and revoked channel.
- 10% no unnecessary WebUI detours after channel is ready.

Bar: at least 0.90 holdout, zero auth-state confusion, zero cross-workspace pairing.

## Eval design

Create 30 dev and 90 holdout scripted browser/channel journeys using fake OAuth and fake Slack or channel APIs. Include new user, returning user, workspace switch, declined consent, expired OAuth state, duplicate install, revoked bot, private channel invite, and channel-first completion.

Answers include session state, channel binding state, next action, route target, first-turn result, and visible recovery state.

## Harness design

Use Playwright-style WebUI tests where practical and Rust route/handler tests for authenticated state transitions. The scorer must validate backend state and channel behavior, not screenshots alone.

`harness/score.sh` must:

- Start each journey from a clean user/workspace/channel fixture.
- Run fake OAuth and fake channel pairing callbacks.
- Submit the first channel message through product workflow.
- Assert visible next actions only where they correspond to backend state.
- Run probe variants that randomize provider, workspace name, channel id, and failure step.

## Constraints

- Wall-clock budget: 10 hours.
- Spend ceilings: $10 LLM/API spend; no live OAuth or Slack calls.
- Surface allowlist: `crates/ironclaw_webui_v2`, `crates/ironclaw_webui_v2_static`, `crates/ironclaw_reborn_webui_ingress`, `crates/ironclaw_reborn_composition`, auth/channel connection facades, channel adapter tests, and docs.
- Capacity caps: shared caps; visible onboarding decision tables capped at 12 rows.
- Do not mark onboarding complete until a channel turn works or the user explicitly chooses WebUI-only.

## Cycle protocol

Follow the common cycle protocol. Each cycle must name the funnel stage it targets and include a recovery path. A UI-only change must be paired with backend state verification before commit.

## Entropy rules

- Every third cycle must exercise a different provider or failure step.
- If happy-path onboarding passes while recovery stays below 0.80, all cycles shift to recovery until the gap closes.
- If a text tweak improves screenshot assertions only, the next cycle must add or fix a state assertion.

## Cheat audit

Lane-specific cheap wins to block:

1. Mark onboarding done after OAuth only; scorer requires first usable channel turn.
2. Use UI text as the only success signal; backend state and channel route are scored.
3. Ignore wrong-workspace callbacks; probe swaps workspaces.
4. Accept duplicate pairing silently; duplicate cases require deterministic state.
5. Skip revoked-channel recovery; holdout includes revocation.
6. Route first channel turn through a test-only bypass; scorer drives product workflow.
7. Store auth state in prompt-visible text; lint checks state storage and logs.
8. Trap channel-first users in WebUI setup pages; no-detour metric penalizes it.
9. Hardcode Slack only; spec can start with Slack, but harness reserves provider variants.
10. Hide failed pairing with generic success; recovery state and audit event must match.

## Stop conditions

Stop when holdout score is at least 0.90 with zero auth-state confusion and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or a pairing path can bind a user to the wrong workspace or channel.

