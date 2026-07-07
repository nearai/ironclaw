# Goal: make Slack the main channel surface without breaking identity or routing

Source page: https://app.notion.com/p/36e29a6526bf8063b148c05ff5d36f16

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for Model 1 only: one Slack app where the bot speaks as the selected agent. Slack takeover, posting as the user, and user-token impersonation are out of scope.

The spec must cover:

- Admin installs one app and maps channels to agents.
- DM reaches the user's personal agent.
- Admin assigns a team agent to a channel.
- Member mentions the bot in a channel and the configured agent replies.
- Thread context, duplicate Slack delivery, approvals, user identity, and memory/tool scope.

Stage 0 tests must drive Slack payloads through the adapter and product workflow, not only helper parsing.

## Target (outer loop)

Optimize Slack routing F1:

- Recall: eligible Slack events route to the correct agent, conversation/thread, and response target.
- Precision: ineligible Slack events do not trigger responses.
- Security: identity, credential, approval, and memory scope match the asking user or configured channel agent.
- UX: response threading and failure messages are correct for DMs, mentions, and channel threads.

Bar: at least 0.95 holdout aggregate, zero cross-user, cross-team, or cross-channel leakage. Holdout is aggregate-only, max 3 calls per 24 hours.

## Eval design

Create 80 dev and 160 holdout Slack event fixtures. Inputs include DMs, public mentions, private channel mentions, thread replies, unmentioned public messages, wrong-team events, duplicate deliveries, channel-agent mappings, removed mappings, unknown users, and approval resumes.

Answers include accept/reject, target agent subject, user subject, team/workspace, conversation id, thread id, Slack response target, approval resume target, and emitted audit events.

## Harness design

Feed Slack payloads through `ironclaw_slack_v2_adapter` into product workflow with a fake turn coordinator. Do not score adapter parsing in isolation. `harness/score.sh` must:

- Start from raw Slack event payloads.
- Assert idempotency on duplicate deliveries.
- Assert no response for unmentioned public-channel messages.
- Assert channel-agent routing only when configured.
- Assert personal-agent routing in DMs.
- Capture approval continuation and thread context.

## Constraints

- Wall-clock budget: 12 hours.
- Spend ceilings: $15 LLM/API spend; $0 live Slack calls except disposable canary if a human enables it.
- Surface allowlist: `crates/ironclaw_slack_v2_adapter`, `crates/ironclaw_product_workflow`, `crates/ironclaw_product_adapters`, `crates/ironclaw_reborn_composition/src/slack_*`, Slack QA helpers, tests, and docs.
- Capacity caps: shared caps; Slack routing branch tables capped at 8 entries.
- Do not mint trusted inbound requests from product adapters. Preserve host-trusted ingress ownership.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include one positive Slack route and one negative or adversarial Slack route. If a route is fixed in DMs, run the equivalent channel-thread variant before committing.

## Entropy rules

- Every fourth cycle must use a different Slack surface: DM, public channel, private channel, thread, approval resume, or admin mapping.
- If duplicate/idempotency behavior is flat, the next change must alter event identity or persistence handling, not retry timing.
- If probe variants fail after text-only changes, remove text special cases and fix structured routing.

## Cheat audit

Lane-specific cheap wins to block:

1. Reply to every Slack message; scorer penalizes unmentioned public-channel responses.
2. Route all channel events to a default personal agent; eval requires configured channel agent.
3. Ignore team/workspace id; probe reuses channel ids across teams.
4. Collapse thread and channel conversations; scorer checks thread target.
5. Skip duplicate detection; duplicate delivery fixtures must produce one turn.
6. Treat unknown users as admins; unknown-user events must deny or pair.
7. Use team-wide credentials for personal actions; security score checks subject binding.
8. Bypass approval resume target; approval continuation is in the answer schema.
9. Hardcode Slack fixture ids; lint rejects literal fixture id branches.
10. Implement Slack takeover/user-token posting; out-of-scope paths get no score and may void if they weaken safety.

## Stop conditions

Stop when Slack holdout F1 is at least 0.95 with zero isolation failures and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or any Slack event can access another user's context or credentials.

