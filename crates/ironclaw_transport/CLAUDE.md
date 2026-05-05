# ironclaw_transport guardrails

- Own protocol translation contracts only: normalize channel/webhook/gateway/IDE ingress and deliver normalized egress to named adapters.
- Do not import authorization, approvals, dispatcher, events, engine, gateway, concrete runtime crates, secrets, network, resource governor, or durable persistence.
- Adapter metadata is transport-owned context and must never override typed scope, thread, user, or adapter fields.
- Error surfaces must stay stable and redacted; do not expose backend paths, tokens, secrets, headers, or provider raw errors.
- Keep adapters policy-free. Approval, auth, prompt assembly, transcript durability, projection reduction, and capability dispatch belong in their owning Reborn services.
