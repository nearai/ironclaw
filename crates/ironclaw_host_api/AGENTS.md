# Agent Map — ironclaw_host_api

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/host-api.md`
- `docs/reborn/contracts/kernel-boundary.md`
- `docs/reborn/contracts/capability-access.md`

## What This Crate Owns

- Shared authority vocabulary and neutral host contracts, currently:
- Validated IDs (`ids`) and the per-invocation authority envelope `ExecutionContext` (`scope`).
- Host-internal/virtual/scoped paths (`path`) and mount permissions/grants/views (`mount`).
- Capability descriptors, grants, sets, constraints, `EffectKind`, `PermissionMode` (`capability`), plus capability-profile schema/operation/contract types (`capability_profile`).
- The neutral model-visible capability ceiling `CapabilitySurfacePolicy` and its capability-id scope algebra (`capability_surface`). It narrows disclosure and never grants dispatch authority.
- Requested effects, host decisions, obligations, and approval scopes (`action`, `decision`, `approval`).
- Budget/resource scopes, estimates, usage, and quota contracts (`resource`).
- Redacted durable audit envelopes (`audit`).
- HTTP vocabulary (`http`) and host-owned ingress route/policy descriptors — `IngressPolicy`, route/listener/auth/rate-limit/CORS/streaming enums (`ingress`).
- Dispatch port contracts (`dispatch`) and host-port catalog/grant/view types (`host_port`, incl. `HOST_RUNTIME_HTTP_EGRESS_PORT_ID`) plus the **default validation catalog** that enumerates every port name the module defines, `default_host_port_catalog` (moved down from `ironclaw_host_runtime` in WS3 row 3, PROPOSAL §6.5.9). It is a validation helper, not authority: a new host-port constant is added to the catalog *there*, beside its name — not in a kernel caller.
- Runtime vocabulary `RuntimeKind`/`TrustClass` (`runtime`) and deployment-mode/profile/effective runtime-policy types (`runtime_policy`).
- Requested-trust vocabulary and `PackageIdentity` (`trust`).
- The **complete** turn vocabulary (`turn`): typed turn/run/checkpoint/lease/runner ids, the bounded `AcceptedMessageRef`/`SourceBindingRef`/`ReplyTargetBindingRef`/`TurnGateRef`/`IdempotencyKey`/`RunProfileId`/`RunProfileRequest` refs and the `Loop*Ref` family, `TurnScope`/`TurnActor`/`TurnThreadOwner`/`TurnOwner`, `TurnStatus` with its `GateKind`/`BlockedReason` gate correspondence, `EventCursor`, `RunOriginAdapter`, and the sanitized failure/cancel shapes. A crate that only *names* turns depends on this crate, never on `ironclaw_turns`.
- Protocol-authentication evidence (`product_adapter::auth`): `ProtocolAuthEvidence`/`VerifiedAuthClaim`/`AuthRequirement`, the **bearer/session** mint family, and the two witness grants that gate all minting — `HostAuthenticationGrant` (from `HostProtocolAuthenticator`) and `VerifiedInboundGrant` (from `ChannelIngressVerifier`). The **channel/webhook** mint family is not here; it is `ironclaw_extension_contracts::verified_inbound`, which reaches the private verified variant through the grant-gated `ProtocolAuthEvidence::seal_verified_inbound`.
- The crate error type `HostApiError` (`error`) and the canonical `Timestamp` alias.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- runtime execution, persistence, HTTP clients, product workflow, policy engines, and dependencies on other service/runtime crates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_host_api`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Contracts are reached module-qualified: `ironclaw_host_api::scope::ExecutionContext`, `ironclaw_host_api::ids::ExtensionId`. There is no flat prelude and no per-module glob re-export in `lib.rs`; `Timestamp` is the sole crate-root item. When you add a type, put it in the module that owns its vocabulary family and let consumers name that module.
- `turn::TurnGateRef` and `ids::GateRef` are different types for different jobs. `TurnGateRef` is the loop-facing *routing* ref: a `bounded_ref!` string validated only as non-empty, <= 256 bytes, and control-character-free. Production mints it as `gate:approval-{id}` / `gate:auth-{id}` and predicates like `is_auth_gate_ref` match that prefix, but **the prefix is a convention the type does not enforce** — do not "fix" a caller or fixture that passes an unprefixed value, and do not tighten the constructor without migrating every persisted ref. The prefix-validated family is `LoopGateRef` (`loop_ref!(..., "gate:")`). `ids::GateRef` is a different thing again: an opaque uuid GateRecord *key*. Neither is an alias of the other, and no crate may re-alias one to the other's name.
- `HostPortGrant` is intentionally a thin scoped-view grant token over `HostPortId`. Do not add attenuation/scope/expiry fields to that wire shape; introduce a distinct scoped/attenuated grant type if that behavior lands later.
- **Do not implement `HostProtocolAuthenticator` or `ChannelIngressVerifier`, and do not add a third grant.** Each has exactly one permitted production implementor — `ironclaw_webui` (bearer/session, trust stage T1) and `ironclaw_extension_host` (channel/webhook, T2) — pinned by `reborn_sealed_evidence_mint_ratchet`, because a second implementor can forge a verified claim for a request nothing authenticated. The grants are the same witness-token pattern as `authorized::AuthorizationGrant`: the field is private to this crate, so the provided trait body is the sole source and an override cannot construct one. Keep them non-`Clone`/non-`Default`/non-`Deserialize`. A **test** that needs verified evidence uses `ProtocolAuthEvidence::test_verified` (the `test-support` seam), and a test double that must hold a grant goes under `tests/` — never an inline `#[cfg(test)]` module, which the ratchet scans like production. This replaced a `host-auth-mint` cargo feature; do not reintroduce a feature gate here, because cargo unifies features across a build and one consumer's opt-in reopened the family workspace-wide.
- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
