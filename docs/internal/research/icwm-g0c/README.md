# ICWM G0C architecture-spike dispositions

Governing Lighthouse G0C status: **Revisions Required**.

Publication-package status: **contribution-grade; pending exact-head review**.
The underlying investigation has review history, but that history does not
approve this tracked package or convert it into accepted Gate 0, G0C, or ADR
evidence.

This directory preserves the minimum reproducible conclusions from the ignored
Gate 0C candidate work. Raw command transcripts, dependency graphs, SBOMs,
databases, credentials, keys, ciphertext, live state, and generated logs remain
outside Git.

The executable neutral contracts and fixtures live in
[`harness/icwm-g0c`](../../../../harness/icwm-g0c/README.md).

## Candidate summary

| Candidate | Contribution-grade disposition | Meaning |
| --- | --- | --- |
| Full `matrix-sdk::Client` 0.18.0 | Infeasible under the frozen ownership envelope | Its supported API does not expose IronClaw-owned mediated HTTP, G0A cursor, durable three-state crypto-effect, or prepared-ciphertext handoff seams. This is an architecture-envelope result, not criticism of SDK production quality. |
| `matrix-sdk-base` + `matrix-sdk-crypto` 0.18.0 | Bounded construction feasible | Base-managed `OlmMachine` construction and initial transport-free request enumeration ran. Sync, durable reopen, encryption, crash recovery, and multi-writer behavior did not run. |
| Direct `OlmMachine` 0.18.0 + SDK SQLite control | Bounded native feasibility with adapter gap | Native construction/reopen and limited lock/replay probes support a deeper spike. The neutral adapter remains blocked on async boundaries, complete store instrumentation, and typed response correlation. Sidecar remains unproven; current WASI placement is disqualified. |
| Complement / Complement Crypto live tier | Plan and scenario allowlist only | Use Complement for homeserver lifecycle/federation and wrap Complement Crypto's workflow boundary. No live-tier execution or G0C interop pass is published here. |

See [candidate dispositions](candidate-dispositions.json) and the minimized
[Complement live-tier plan](complement-live-tier.md).

## Authority boundary

These artifacts may inform a later independently reviewed G0C result. They do
not select placement, approve a production graph/store, waive advisories, or
authorize Matrix implementation. Exact-head review of this package remains
pending. The containing source commit is bound by the repository's exact-head
review record and post-merge receipt, not embedded in these files.
