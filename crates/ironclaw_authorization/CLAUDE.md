# ironclaw_authorization guardrails

- Own grant matching, lease state, and dispatch/spawn authorization decisions.
- Do not execute capabilities, persist run-state, resolve approvals, reserve resources, prompt users, or import runtime/process/dispatcher/capability workflow crates.
- Authorization is default-deny and resource-owner/invocation scoped (tenant/user/agent/project/mission/thread plus invocation where applicable).
- Filesystem-backed leases must use async filesystem calls, not nested `block_on`.
- The filesystem lease store is an early/local backend: its per-owner keyed mutation locks are process-local and not a cross-process transaction/CAS mechanism. Production shared roots must use a transactional backend or explicit compare-and-swap before real concurrent callers.
- Fingerprinted approval leases are resume-only authority and must not become ambient grants.
