# ironclaw_network guardrails

- Own network policy evaluation helpers, scoped network permits, and hardened runtime HTTP egress only.
- HTTP egress must policy-check, DNS-resolve, private-address-check, redirect-revalidate, pin validated resolution, and bound response size before returning data.
- Do not perform secret injection, resource reservation, audit/event emission, approval prompts, trace recording, OAuth repair, or product workflow here.
- Preserve tenant/user/project scope in requests, permits, and errors.
- Fail closed when no target pattern matches or no allowed targets are configured.
- Keep host matching intentionally simple: exact host or one leading wildcard label (`*.example.com`), never arbitrary regex.
- Do not depend on runtime, workflow, secret, filesystem, resource, event, approval, or authorization crates.
