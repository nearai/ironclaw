# ironclaw_event_streams

Transport-neutral Reborn projection stream manager.

Keep this crate:

- above `ironclaw_event_projections` and `ironclaw_outbound`;
- transport-neutral: no Axum, WebSocket, SSE, Telegram, Slack, OpenAI/Responses, or channel framing;
- projection-safe: consume projection DTOs and never durable event rows directly;
- access-first: actor/scope/view/target authorization must run before snapshot, replay, or live subscription;
- bounded: long-lived subscriptions must pass admission policy and use bounded buffers;
- no-exposure: stream-boundary validation fails closed for raw prompts, tool I/O, secrets, host paths, provider errors, invocation fingerprints, approval reasons, lease material, and backend diagnostics;
- egress-separated: external push candidates are selected through outbound policy and are not implied by projection subscription access.
