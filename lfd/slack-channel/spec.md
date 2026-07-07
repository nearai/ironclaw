# Spec: slack-channel - Slack as a primary Reborn channel

Sources: `lfd/_briefs/COMMON.md`, `lfd/_briefs/slack-channel.md`,
`lfd/_shared/SCHEMA.md`, the Lane-03 roadmap goal/addendum, root
`CLAUDE.md`, `crates/ironclaw_slack_v2_adapter/AGENTS.md`,
`docs/plans/2026-06-25-slack-admission-permit.md`,
`docs/plans/2026-06-25-slack-delivery-blocked-terminal.md`, and
`docs/plans/2026-07-02-channel-connection-gate.md`.

## 1. Product scope

Build Slack as a first-class Reborn channel for Model 1:

- One Slack app installation is operator-managed.
- The bot speaks as the selected IronClaw agent.
- A DM reaches the bound user's personal agent.
- A configured shared channel routes app mentions and thread replies to the
  channel's configured team/managed subject.
- Thread context, duplicate Slack delivery, approval/auth replies, user
  identity, admission, channel connection, and memory/tool scope are preserved.

Non-goals: Slack takeover, posting as a human user, user-token impersonation,
multi-app routing policy beyond installation scoping, live Slack network calls
inside the eval, and new feature behavior in the legacy `src/` Slack channel.

## 2. Architecture rules

- `crates/ironclaw_slack_v2_adapter` owns Slack protocol parsing/rendering only.
  It receives raw event bytes plus verified auth evidence, emits
  `ParsedProductInbound`, and renders `FinalReply`, `GatePrompt`, and
  `AuthPrompt` envelopes to host-mediated `chat.postMessage` requests.
- The host/composition layer owns signature verification, trusted context,
  admission permits, tenant/installation scoping, identity lookup, channel route
  resolution, setup persistence, connection gates, delivery observation, and
  Slack HTTP egress credential injection.
- Product adapters and product workflow must not mint trusted inbound requests.
  Trusted ingress remains host-owned.
- Credential and extension identities stay distinct:
  `credential_name = "slack_bot_token"` and `extension_name = "slack"`.
- Egress to Slack must use the declared host-mediated HTTP port. The adapter may
  carry only an opaque credential handle, never raw Authorization headers.

## 3. Required behavior

### Inbound routing and scoping

- Slack `message` events in DMs become user-message turns for the bound Slack
  user. Conversation key uses installation, team, DM channel, and message/thread
  ids from the Slack payload.
- Slack `app_mention` events in shared channels strip the leading bot mention,
  route only if the workspace/channel is admitted and connected, and target the
  configured channel subject for that tenant, installation, team, and channel.
- Slack `message` events in threads route as `ReplyToBot` only when they belong
  to a known bot thread for that tenant/installation/channel/thread. Unmentioned
  public channel messages outside a tracked thread are no-ops.
- Duplicate Slack event ids are idempotent: one accepted turn and at most one
  outbound delivery for the same event id.
- Slack team/user/channel ids are never globally trusted. All lookups include
  tenant id and installation id.

### Identity

- Actor resolution uses provider `slack` and provider user id
  `<installation_id>:<slack_user_id>`.
- Bound users resolve to stable `UserId`s through the identity resolver.
- Unknown Slack users are not treated as admins or dropped silently. They either
  enter the pairing/connection flow or produce a typed denial with no agent turn.

### Channel setup and connection gate

- Setup stores bot token material under the `slack_bot_token` credential handle
  and routes setup/UI by extension name `slack`.
- Traffic is blocked until the Slack app is configured and the caller's Slack
  identity is paired for that installation. The block is represented on the
  existing auth/channel-connection gate rail, not a frontend-only heuristic.
- Permit revocation or route deletion takes effect before the next inbound event.

### Admission and delivery

- Admission permits gate fast intake only. Once a Slack inbound is durably
  accepted or rejected, the permit is released before any long delivery poll.
- Final replies and prompts post with `channel` and `thread_ts` matching the
  Slack conversation/thread target.
- Retryable Slack Web API failures such as HTTP 429 are retried within bounded
  policy. Permanent failures such as `channel_not_found` or revoked credentials
  persist terminal delivery state and do not loop forever.
- A run parked in approval/auth after an actionable Slack prompt has been
  delivered is terminal-for-delivery as delivered/blocked, not a failed delivery
  caused by the long wait backstop.

## 4. Stage-0 tests

The optimizer should add or extend caller-level coverage rather than helper-only
tests. Required seams:

- Adapter parse/render tests in `ironclaw_slack_v2_adapter` for DM, app mention,
  thread reply, approval/auth resolution, no-op, attachment, and render targets.
- Integration coverage through product workflow for Slack payloads reaching the
  correct subject, agent, turn, and reply target.
- Delivery tests for retry, permanent terminal state, blocked approval/auth
  terminal-for-delivery, and admission permit release after durable accept.
- Channel connection/setup tests proving `slack_bot_token` versus `slack` naming
  and gate blocking/resume behavior.
- LFD profile `slack_channel`, in
  `tests/integration/lfd/profiles/slack_channel.rs`, must execute all dev cases
  and emit outcomes from real persisted state plus recorder output.

## 5. LFD profile input schema

Every case uses `profile: "slack_channel"` and
`setup.profile_extra.slack_fixture`:

```jsonc
{
  "tenant_id": "tenant:blue-a",
  "installation_id": "slack-install-blue-a",
  "team_id": "T031BLUEA",
  "api_app_id": "A031BLUEA",
  "bot_user_id": "UBOTLANE03",
  "operator_user_id": "user:ops",
  "default_personal_agent_id": "agent:personal",
  "channel_routes": [
    {"channel_id": "C031ALPHA", "subject_user_id": "user:team-alpha", "agent_id": "agent:team-alpha"}
  ],
  "identity_bindings": [
    {"slack_user_id": "U031ALICE", "user_id": "user:alice", "dm_channel_id": "D031ALICE"}
  ],
  "connection": {"configured": true, "caller_connected": true},
  "admission": {"workspace_allowed": true, "channels_allowed": ["C031ALPHA"]},
  "delivery_script": [{"status": 200, "body": {"ok": true}}],
  "existing_threads": [
    {"channel_id": "C031ALPHA", "thread_ts": "1770000000.000100", "subject_user_id": "user:team-alpha"}
  ],
  "expectation_hint": "not scored directly; runner may use it to assemble fixture state"
}
```

The profile must not derive outcome truth from `expectation_hint`. Contracts
score only runner outcomes: events, gates, egress recorders, replies, leaks, and
`state_queries` after the scenario.

## 6. State-query contract

The runner executes `state_queries` after each scenario against durable stores
and recorder projections. Supported kinds:

- `slack_turn` params `{event_id}` -> `{status, tenant_id, installation_id,
  owner_user_id, agent_id, trigger, channel_id, thread_ts, text, accepted}`.
- `slack_route_resolution` params `{event_id}` -> `{permitted, route_type,
  subject_user_id, agent_id, deny_reason, tenant_id, installation_id}`.
- `slack_identity` params `{installation_id, slack_user_id}` -> `{status,
  provider, provider_user_id, user_id, created, tenant_id}`.
- `slack_delivery` params `{event_id}` -> `{status, channel_id, thread_ts,
  attempts, post_count, credential_name, extension_name, blocked_terminal,
  terminal_reason}`.
- `slack_admission` params `{event_id}` -> `{admitted, denied_reason,
  permit_released_before_delivery, max_in_flight_observed}`.
- `slack_gate` params `{event_id}` -> `{channel, connected, status,
  challenge_kind, credential_name, extension_name}`.
- `slack_thread_scope` params `{channel_id, thread_ts}` -> `{turn_count,
  subject_user_id, agent_id, delivery_thread_ts}`.
- `slack_dedupe` params `{event_id}` -> `{accepted_count, duplicate_count,
  turn_count, delivery_count}`.
- `slack_setup` params `{installation_id}` -> `{configured, credential_name,
  extension_name, bot_token_secret_present, signing_secret_present,
  routes_count}`.
- `slack_approval` params `{event_id}` -> `{decision, gate_ref, scope,
  source_trigger, resumed_run_id}`.
- `slack_auth` params `{event_id}` -> `{result, auth_request_ref,
  source_trigger, resumed_run_id}`.
- `slack_isolation_audit` params `{event_id}` -> `{tenant_id,
  touched_tenant_ids, cross_tenant_reads, cross_tenant_writes, same_slack_ids}`.

The profile may add extra fields, but contracts only depend on the fields
listed above. State comes from persisted records, not local variables inside the
profile.

## 7. Eval design

Dev set: 30 scripted cases with at least 25 percent failure/denial direction.
Holdout set: 12 off-repo cases under
`/Volumes/NVME/ironclaw-lfd/holdout/slack-channel/`, with a distinct canary.

Themes:

- Inbound routing and scoping: DMs, app mentions, private channels, thread
  replies, unknown users, and shared-channel managed subjects.
- Outbound delivery: correct `thread_ts`, retryable 429, permanent blocked
  terminal state, blocked approval/auth prompts, and no infinite retry.
- Admission and connection gates: unadmitted workspace/channel, revocation,
  missing setup, and connected setup.
- Thread continuity and idempotency.
- Tenant, installation, team, user, and channel isolation.

Every case has at least one state query and every contract includes state,
event, gate, egress, or delivery assertions beyond any reply assertion. Failure
cases require typed denial/terminal state and forbid spurious Slack egress.

## 8. Rollback and risk notes

- Slack ingress and egress are security-sensitive. Any regression in signature
  checks, tenant scoping, credential injection, or CORS/rate/body limits is a
  release blocker.
- Setup changes affect encrypted secrets. Rollback must leave prior setup and
  bot/signing secret handles intact if activation fails.
- Delivery state is user-visible and operationally important. Avoid recording
  worse terminal state after a successful actionable prompt.
- Channel-route changes can alter memory/tool scope. Use tenant and subject
  state queries to prove no cross-user or cross-channel access.
