# ironclaw_host_api — working rules

Canonical crate guidance (the crate's `CLAUDE.md` is a pointer here).
Orientation and public surface: [`README.md`](./README.md). Family boundary
and admission test: [`../AGENTS.md`](../AGENTS.md).

## Start here

- Read `README.md` for what the crate is; read `Cargo.toml` for the real
  dependency and feature shape.
- Contract docs that outrank intuition before changing behavior:
  `docs/reborn/contracts/host-api.md`,
  `docs/reborn/contracts/kernel-boundary.md`,
  `docs/reborn/contracts/capability-access.md`.
- If the contract and code disagree, stop and treat the task as a
  contract-change request instead of silently changing ownership.

## Working rules

- **Own shared authority vocabulary only.** Keep behavior to
  validation/serialization helpers on a type's own shape; no runtime
  execution, persistence, HTTP clients, policy engines, or product workflow —
  and no dependency on any other `ironclaw_*` crate (the zero-internal-dep
  assert in `reborn_crate_dependency_boundaries_hold` is the whole system's
  safety property).
- **`turn` is the complete canonical turn language, not a partial one.** If a
  crate needs to *name* a turn — scope, ids, refs, status, gate kind, event
  cursor, origin adapter — it depends on this crate, never on
  `ironclaw_turns`. When a turn type must be named outside the turn kernel,
  the answer is to finish moving it here, never to re-export it from
  `ironclaw_turns`.
- **No wildcard re-exports.** `lib.rs` exposes modules, never a flat prelude;
  consumers import `ironclaw_host_api::<module>::<Type>` (e.g.
  `scope::ExecutionContext`, `ids::ExtensionId`). The only crate-root item is
  the `Timestamp` alias. A per-module glob hides which module a consumer
  depends on — exactly what carving a vocabulary family out of this crate
  needs to see.
- **HTTP ingress contracts are route/policy vocabulary only.** Listener
  binding, router mounting, auth enforcement, scope extraction, body/rate
  limits, CORS/Origin checks, audit emission, and effect dispatch belong to
  host composition and the transports.
- **Serializable API types must not contain raw `HostPath`, secrets, or
  backend-specific error details** — in errors, events, snapshots, logs, or
  docs. The narrow exception is bounded `ModelDiagnostic`: producers must
  scrub credential values and fence injection-shaped text before carrying a
  cause needed for model recovery.
- **Host-port catalog:** a new host-port constant is added to
  `host_port::default_host_port_catalog` *there, beside its name* — not in a
  kernel caller. The catalog is a validation helper, not authority.
- Prefer strong enums/newtypes over strings when the shape is known
  (`.claude/rules/types.md`).
- Keep edits inside this crate unless a contract explicitly requires a
  neighboring crate change.

## Traps

- **`turn::TurnGateRef` and `ids::GateRef` are different types for different
  jobs.** `TurnGateRef` is the loop-facing *routing* ref: a `bounded_ref!`
  string validated only as non-empty, <= 256 bytes, control-character-free.
  Production mints `gate:approval-{id}` / `gate:auth-{id}` and predicates like
  `is_auth_gate_ref` match that prefix, but **the prefix is a convention the
  type does not enforce** — do not "fix" a caller or fixture that passes an
  unprefixed value, and do not tighten the constructor without migrating every
  persisted ref. The prefix-validated family is `LoopGateRef`
  (`loop_ref!(..., "gate:")`). `ids::GateRef` is an opaque uuid GateRecord
  *key*. Neither is an alias of the other; no crate may re-alias one to the
  other's name.
- **`HostPortGrant` is intentionally a thin scoped-view grant token** over
  `HostPortId`. Do not add attenuation/scope/expiry fields to that wire shape;
  introduce a distinct scoped/attenuated grant type if that behavior lands.

## Sealed evidence — do not widen

- **Do not implement `HostProtocolAuthenticator` or `ChannelIngressVerifier`,
  and do not add a third grant.** Each has exactly one permitted production
  implementor — `ironclaw_webui` (bearer/session, trust stage T1) and
  `ironclaw_extension_host` (channel/webhook, T2) — pinned by
  `reborn_sealed_evidence_mint_ratchet`, because a second implementor can
  forge a verified claim for a request nothing authenticated.
- The grants are the same witness-token pattern as
  `authorized::AuthorizationGrant`: the field is private to this crate, so the
  provided trait body is the sole source and an override cannot construct one.
  Keep them non-`Clone`/non-`Default`/non-`Deserialize`.
- A **test** that needs verified evidence uses
  `ProtocolAuthEvidence::test_verified` (the `test-support` seam); a test
  double that must hold a grant goes under `tests/` — never an inline
  `#[cfg(test)]` module, which the ratchet scans like production.
- This replaced a `host-auth-mint` cargo feature. Do not reintroduce a feature
  gate here: cargo unifies features across a build, and one consumer's opt-in
  reopened the family workspace-wide.

## Validation

- Fast local check: `cargo test -p ironclaw_host_api`
- Boundary/seal/ceiling gates after dependency or API changes:
  `cargo test -p ironclaw_architecture_tests`
- Prefer caller-level tests when a helper gates dispatch, persistence,
  network, secrets, approvals, resources, events, or process side effects
  (`.claude/rules/testing.md`).
