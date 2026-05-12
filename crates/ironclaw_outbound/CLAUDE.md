# ironclaw_outbound guardrails

- Own outbound egress policy, delivery-status metadata, and projection subscription cursor checkpoints only.
- Do not send transport messages, validate concrete Slack/Telegram/Web payloads, or mutate canonical transcript/projection state.
- Persist metadata/refs/cursors only: no raw prompts, message bodies, tool inputs/outputs, secrets, host paths, or backend error details.
- External push targets are candidates only; product adapters must revalidate reply-target binding authorization before every push.
- Delivery failure records are separate from canonical transcript/projection state and must not mark turns/runs failed.
