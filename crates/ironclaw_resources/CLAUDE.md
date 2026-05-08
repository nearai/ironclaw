# ironclaw_resources guardrails

- Own resource reservation, reconciliation, release, and quota accounting.
- No costed or quota-limited work should execute without an active reservation or explicit documented exception.
- Do not import runtimes, dispatcher, capabilities, approvals, processes, events, or product workflow crates.
- Preserve tenant/user/project scope in every reservation and receipt.
- Keep accounting deterministic and safe under concurrent reservations.
