# Goal: deliver the NEAR Foundation must-have rollout bundle

Source page: https://app.notion.com/p/36e29a6526bf80b3b02ef6d4fbd3f47f

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` as a rollout checklist for the must-have bundle:

- Google Suite: Gmail, Calendar, Drive, Sheets, and Docs.
- Slack relay.
- Notion through MCP.
- WebUI with Google OAuth.
- Routines.
- Hosted deployment readiness.

The spec must explicitly mark Slack team-agent context, admin skills, IronHub, and admin-propagated tools as nice-to-have unless a human re-scopes them. Stage 0 must include one caller-level or E2E-style test plan per must-have and one recovery case per auth or connector boundary.

## Target (outer loop)

Optimize bundle acceptance:

- 60% must-have journeys complete end to end.
- 20% recovery paths are correct for auth expiry, connector revocation, missing permission, and external API failure.
- 10% operator visibility shows the failing subsystem without leaking secrets.
- 10% user experience has no dead-end setup state after a recoverable failure.

Hard gates: every must-have has at least one passing holdout journey; zero auth, tenant, secret, or cross-user leakage defects. Bar: 1.00 must-have coverage and at least 0.92 weighted holdout score.

## Eval design

Create 10 dev and 30 holdout rollout journeys. Each journey combines login, connector setup, a turn, an approval or tool call, and a recovery branch. Include mixed journeys that touch more than one connector, such as Slack request to summarize a Drive document into a Notion page.

Holdout answers are expected auth state, connector state, route, external fake-provider calls, final user-visible state, and audit events. Nice-to-haves appear only as distractors and must not improve the score.

## Harness design

Use hermetic fake providers for CI and live canaries only as supplemental evidence. `harness/score.sh` must drive:

- WebUI or API login and Google OAuth fake state.
- Slack relay ingress and response routing.
- MCP Notion invocation through configured product capability paths.
- Routine creation or execution.
- Hosted readiness probes through local deployment config, not production credentials.

The scorer must fail both missing behavior and over-permissive behavior. Example: a Drive call that works with another user's token is worse than a failed call.

## Constraints

- Wall-clock budget: 14 hours.
- Spend ceilings: $20 LLM/API spend; $0 live Google/Slack/Notion calls unless using disposable, limited canary accounts.
- Surface allowlist: Reborn composition, WebUI v2, product workflow, auth, connector adapters, routine/turn paths, hosted config, QA scripts, and tests.
- Capacity caps: shared caps; rollout checklist fixtures capped at one seed per must-have per provider.
- Do not let nice-to-have implementation displace a must-have. The scorer gives no credit for nice-to-haves before all must-have gates pass.

## Cycle protocol

Follow the common cycle protocol. Each cycle must name the must-have it targets and the recovery path it protects. After every passing journey, run a negative variant that denies auth, switches tenant/user, or revokes the connector.

## Entropy rules

- Rotate between connector, UI/auth, routine, and hosted readiness dimensions every 3 cycles.
- If a path passes only in one provider, the next cycle must add a provider-agnostic contract or abstraction check.
- If recovery cases trail happy paths by more than 15 points, all further cycles target recovery until the gap closes.

## Cheat audit

Lane-specific cheap wins to block:

1. Count a mock-only connector as rollout-ready; fake providers assert real request shape.
2. Give credit for nice-to-haves; scorer ignores them until must-haves pass.
3. Use a single omnipotent test user; eval swaps users and tenants.
4. Store OAuth or connector tokens in logs; lint scans event and transcript artifacts.
5. Treat UI button visibility as connector success; scorer validates backend state and fake-provider call.
6. Skip recovery branches; weighted score includes recovery and hard gates leakage defects.
7. Bypass hosted config by running local defaults; harness launches with hosted-like config.
8. Collapse Google Suite apps into one fake operation; eval requires app-specific calls.
9. Hardcode Notion MCP tool names; probe swaps workspace and tool ids.
10. Claim readiness from live canary only; holdout hermetic score is authoritative.

## Stop conditions

Stop when every must-have holdout gate passes and weighted holdout score is at least 0.92, budget is exhausted, no score improves for 3 cycles, or a security-critical connector/auth issue is discovered.

