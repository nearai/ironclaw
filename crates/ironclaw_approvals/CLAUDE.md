# ironclaw_approvals guardrails

- Own approval resolution workflow: pending approval record to scoped lease or denial.
- Do not prompt users, dispatch capabilities, manage processes, reserve resources, or import runtime/dispatcher/capability workflow crates.
- Approve fail-closed: issue durable lease first, then mark approved; revoke issued lease best-effort if status write fails.
- Denials issue no lease.
- Audit emission is metadata-only and best-effort.
