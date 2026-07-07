# LFD Brief: slack-channel — Slack as the main channel

**State**: partial — `crates/ironclaw_slack_v2_adapter` is tracer-bullet scope
(#3857). **Bar**: 0.90 holdout. **Profile**: `slack_channel`.

## Outcome

The Slack v2 ProductAdapter reaches product parity as a primary channel:
inbound events (DM, app_mention, threaded replies) become correctly-scoped
turns; outbound replies deliver with thread continuity and terminal-state
handling for blocked delivery; admission permits and the channel-connection
gate govern traffic; Slack identities map through the identity resolver.

## Spec sources (generator: synthesize spec.md from these)

- `docs/plans/2026-06-25-slack-admission-permit.md`
- `docs/plans/2026-06-25-slack-delivery-blocked-terminal.md`
- `docs/plans/2026-07-02-channel-connection-gate.md`
- `crates/ironclaw_slack_v2_adapter/` (current tracer scope = the floor)
- `crates/ironclaw_reborn_composition/src/slack_*.rs` — the real wiring
  surface is rich: actor_identity, channel_connection, channel_routes,
  delivery, egress, personal_binding*, serve, setup
- v1 `src/channels/slack/` (behavioral reference for parity, NOT code to extend)
- Blue-lane 03 (docs/lfd/roadmap-blue-lanes-2026-07-07/) — Model-1 scope
  governs: one Slack app, bot speaks as the selected agent; takeover and
  user-token posting are out of scope
- Root CLAUDE.md extension/auth invariants (`credential_name` slack_bot_token
  vs `extension_name` slack)

## Stage 0 inner suite

Existing `ironclaw_slack_v2_adapter` crate tests + `tests/reborn_*` parity
tests + new integration tests the spec demands. Full command list goes in
goal.md.

## Eval themes (dev ~40 / holdout ~14)

1. Inbound routing & scoping (10): DM vs mention vs thread reply → turn with
   correct (tenant, agent, owner_user_id) via identity resolver; unknown
   Slack user → identity created/mapped per spec, not dropped.
2. Outbound delivery (8): reply posts to correct channel + `thread_ts`;
   stubbed `chat.postMessage` 429 → retry then success; permanent failure →
   blocked-terminal state persisted (state query), NOT silent drop or
   infinite retry (forbidden: >N egress attempts).
3. Admission permits (6): un-admitted workspace/channel traffic → denied
   with typed outcome; admitted → flows; permit revocation mid-stream.
4. Connection gate & onboarding (6): connect flow stores
   `slack_bot_token` under the credential_name, routes setup by
   extension_name (state_pred on wire contract fields), gate blocks traffic
   until connected.
5. Thread continuity (5): multi-turn conversation keeps one thread scope
   across inbound events with same thread_ts.
6. Isolation (5): two tenants with overlapping Slack team ids stay isolated
   (forbidden: cross-tenant thread/state access).

Failure-direction share ≥ 25% (429s, revoked permits, bad signatures,
malformed payloads → fail-soft contracts: `status:"ran"` with typed error
events, never crash).

## Feature-specific cheats → fences

- **Hardcode Slack ids from eval** (`U…`/`T…`/`C…` literals) → caps.json:
  regex `\b[UTC][0-9A-Z]{7,}\b` in `crates/**` diff, max 0; probe renames
  all Slack ids via the map.
- **Always-ok egress in profile** → delivery-failure cases REQUIRE the 429
  retry sequence and terminal state in persisted storage; profile stubs are
  keyed per-case from visible inputs, extraction is pinned.
- **Reply without permit** → forbidden gate/egress matchers on un-admitted
  cases; required typed-denial events.
- **Skip identity resolver** (parse user from payload ad hoc) → state_pred
  contracts assert resolver-produced stable UserIds, holdout uses
  colliding external ids across tenants.

## caps.json extras

`{"name":"slack-id-literals","paths":["crates/**"],"pattern":"\\b[UTC][0-9A-Z]{7,}\\b","max_count":0}`
(count against git diff added lines — lint semantics), plus the COMMON caps.

## Live mode

4 live cases (cases-live/): real model drives a Slack conversation over the
mocked adapter (scripted egress stubs, live LLM) — checks the adapter's
tool/reply surface composes with a real model. Stage AFTER scripted bar ≥ 0.85.
