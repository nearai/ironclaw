# ironclaw_network guardrails

- Own network policy evaluation and scoped network permits only; actual egress enforcement happens at the runtime/network execution boundary.
- Do not perform HTTP I/O, DNS/redirect revalidation, proxying, response limiting, secret injection, resource reservation, audit/event emission, or product workflow here.
- Preserve tenant/user/agent/project scope in requests, permits, and errors.
- Fail closed when no target pattern matches or no allowed targets are configured.
- Keep host matching intentionally simple: exact host or one leading wildcard label (`*.example.com`), never arbitrary regex.
- Do not depend on runtime, workflow, secret, filesystem, resource, event, approval, or authorization crates.
