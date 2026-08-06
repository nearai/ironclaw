# ironclaw_network guardrails

- Own network policy evaluation, DNS/private-IP enforcement, redirect policy, response limits, and outbound HTTP transport for host-mediated Reborn runtime egress.
- Do not perform secret injection, resource reservation, audit/event emission, authorization/approval decisions, or product workflow here.
- Preserve tenant/user/agent/project scope in requests, permits, and errors.
- Fail closed when no target pattern matches or no allowed targets are configured.
- Keep host matching intentionally simple: exact host or one leading wildcard label (`*.example.com`), never arbitrary regex.
- Do not depend on runtime, workflow, secret, filesystem, resource, event, approval, or authorization crates.
