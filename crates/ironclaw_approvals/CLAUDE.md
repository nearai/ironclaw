# ironclaw_approvals guardrails

- Own approval resolution workflow: pending approval record to scoped lease or denial.
- Do not prompt users, dispatch capabilities, manage processes, reserve resources, or import runtime/dispatcher/capability workflow crates.
- Approve fail-closed: persist `approve` (the authority record) first, then issue the lease. If the lease store fails after approval is persisted, the request stays `Approved` and the caller surfaces the lease error — no rollback to `Pending`. The approval record is the durable decision; lease re-issuance against an already-decided request is recoverable.
- Denials issue no lease.
- Audit emission is metadata-only and best-effort. Failures are logged at `debug!` and never alter resolution outcomes.
