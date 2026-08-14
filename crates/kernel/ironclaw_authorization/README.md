# ironclaw_authorization

The authorization-decision stage of the kernel pipeline: matches a caller's
grants and active leases to the requested effect under the effective trust
ceiling, default-deny. It also owns the capability-lease state machine — the
single-winner claim protocol that lets a resumed, previously-approved call
re-enter safely without granting a second dispatch in parallel. It is a
separate crate from `ironclaw_approvals` because "does a grant cover this" and
"did a human agree to this" are different questions with different failure
modes, and only one of them stores leases.

- **Family / layer:** `kernel/` / `kernel` · **Package:**
  `ironclaw_authorization` · **Manifest:**
  `crates/kernel/ironclaw_authorization/Cargo.toml`
- **Use this when:** you need an allow/deny/require-approval verdict for a
  concrete effect, or you need to issue, claim, or expire a capability lease.
- **Don't use this when:** you want to *resolve* a pending approval into a
  lease (→ `ironclaw_approvals`), dispatch anything (→
  `ironclaw_capabilities`), or reason about ceilings (→ `ironclaw_trust`).

## Public surface

- Authorizer ports and impls: `CapabilityDispatchAuthorizer` /
  `TrustAwareCapabilityDispatchAuthorizer` traits, `GrantAuthorizer`,
  `LeaseBackedAuthorizer`, `grant_exceeds_authority_ceiling` (`src/lib.rs`).
- Lease state: `CapabilityLease` — carrying
  `invocation_fingerprint: Option<InvocationFingerprint>` (`src/lib.rs:187`) —
  `CapabilityLeaseStatus`, `CapabilityLeaseError`, the `CapabilityLeaseStore`
  trait, and the one production filesystem-backed store (bounded
  compare-and-swap, `CasExpectation::Version` with a retry budget, over
  versioned roots; per-owner in-process mutation locks on top).
- The claim protocol: atomic fingerprint-matched claim (`lib.rs:268-291`) and
  the CAS-version retry that keeps one-shot fingerprinted leases single-winner
  (`lib.rs:740-751`).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_host_api`, `ironclaw_trust` (the
  ceiling every grant must satisfy — pinned same-layer edge),
  `ironclaw_filesystem` (lease persistence through `ScopedFilesystem`; tests
  reuse the same store over an in-memory *backend*, not a bespoke store).
- **Normal consumers (7):** `ironclaw_approvals`, `ironclaw_capabilities`,
  `ironclaw_host_runtime` (kernel), `ironclaw_assistant`,
  `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_extension_manager`.

## Invariants

- Default-deny: no matching grant means deny; there is no fail-open branch.
- Fingerprinted leases are resume-only authority: the ambient-grant conversion
  filters them out (`lib.rs:308-313`), so a lease issued for one exact input
  can never become a standing permission.
- Claim is single-winner and fingerprint-matched before consume
  (`lib.rs:268-291`); only byte-only/`Unsupported` filesystem roots degrade to
  process-local serialization, and those are documented as unsafe for real
  cross-process concurrency (see `AGENTS.md`).
- Dependency boundary: the `BoundaryRule` for `ironclaw_authorization` in
  `reborn_dependency_boundaries.rs` forbids `ironclaw_approvals`,
  `ironclaw_capabilities`, `ironclaw_host_runtime`, `ironclaw_processes`,
  `ironclaw_resources`, `ironclaw_secrets`, `ironclaw_network`, and the lane
  crates — approval resolution depends on this crate, never the reverse.

## Tests

```bash
cargo test -p ironclaw_authorization
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules and guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family; the lease's place in the
  sealed-mint table.
- `docs/internal/reborn/contracts/capability-access.md`,
  `docs/internal/reborn/contracts/kernel-boundary.md`.
