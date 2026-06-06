# ironclaw_authorization guardrails

- Own grant matching, lease state, and dispatch/spawn authorization decisions.
- Do not execute capabilities, persist run-state, resolve approvals, reserve resources, prompt users, or import runtime/process/dispatcher/capability workflow crates.
- Authorization is default-deny and resource-owner/invocation scoped (tenant/user/agent/project/mission/thread plus invocation where applicable).
- Filesystem-backed leases must use async filesystem calls, not nested `block_on`.
- The filesystem lease store writes via bounded compare-and-swap (`CasExpectation::Version` with a retry budget) over versioned roots, giving cross-process safety; its per-owner keyed mutation locks add in-process serialization on top. Only byte-only/`Unsupported` roots (no version support) degrade to process-local serialization alone — those are not safe for real concurrent cross-process callers.
- Fingerprinted approval leases are resume-only authority and must not become ambient grants.
