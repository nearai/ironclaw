# ironclaw_authorization guardrails

- Own grant matching, lease state, and dispatch/spawn authorization decisions.
- Do not execute capabilities, persist run-state, resolve approvals, reserve resources, prompt users, or import runtime/process/dispatcher/capability workflow crates.
- Authorization is default-deny and tenant/user/invocation scoped.
- Filesystem-backed leases must use async filesystem calls, not nested `block_on`.
- Fingerprinted approval leases are resume-only authority and must not become ambient grants.
