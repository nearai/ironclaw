# Slack Channel LFD Pilot Spec

## Scope

This package demonstrates how a real feature can use the shared LFD infrastructure. It covers four Slack behaviors from the previous Slack roadmap dev set:

- DM routes to a paired user's personal agent.
- App mention in a configured channel routes to the channel agent.
- Duplicate Slack `event_id` is idempotent.
- Unmentioned public channel message is a no-op.

## Harness Boundary

The profile parses each raw Slack event with `ironclaw_slack_v2_adapter::parse_slack_event`. Parsed `UserMessage` payloads are submitted to the Reborn integration harness as scripted turns. Parsed `NoOp` payloads and duplicate event ids are skipped before turn admission.

This is not yet the full production Slack ingress path. The next stack should replace this parser-backed admission with the signed Slack host route and recorded outbound Slack delivery sink.

## State Queries

- `slack_parse`: parser result, actor, channel, thread, stripped text, and trigger.
- `slack_route`: deterministic fixture-backed route resolution for this pilot.
- `slack_delivery`: synthetic delivery count/status for admitted messages.
- `slack_dedupe`: accepted, duplicate, turn, and delivery counts per event id.
- `slack_isolation_audit`: tenant isolation counters for the fixture scope.

## Expected Behaviors

DM messages in IM channels become `direct_chat` user messages and route to the configured personal agent for the Slack user.

App mentions strip the leading bot mention, become `bot_mention` user messages, preserve the Slack channel thread timestamp, and route to the configured channel agent.

Duplicate inbound payloads with the same Slack `event_id` admit exactly one turn and one delivery.

Unmentioned public channel messages parse as `NoOp`, admit no turn, and produce no delivery.
