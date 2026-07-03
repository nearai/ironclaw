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
- Requested effects, host decisions, obligations, and approval scopes (`action`, `decision`, `approval`).
- Budget/resource scopes, estimates, usage, and quota contracts (`resource`).
- Redacted durable audit envelopes (`audit`).
- HTTP vocabulary (`http`) and host-owned ingress route/policy descriptors — `IngressPolicy`, route/listener/auth/rate-limit/CORS/streaming enums (`ingress`).
- Dispatch port contracts (`dispatch`) and host-port catalog/grant/view types (`host_port`, incl. `HOST_RUNTIME_HTTP_EGRESS_PORT_ID`).
- Runtime vocabulary `RuntimeKind`/`TrustClass` (`runtime`) and deployment-mode/profile/effective runtime-policy types (`runtime_policy`).
- Requested-trust vocabulary and `PackageIdentity` (`trust`).
- The crate error type `HostApiError` (`error`) and the canonical `Timestamp` alias.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- runtime execution, persistence, HTTP clients, product workflow, policy engines, and dependencies on other service/runtime crates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_host_api`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- `HostPortGrant` is intentionally a thin scoped-view grant token over `HostPortId`. Do not add attenuation/scope/expiry fields to that wire shape; introduce a distinct scoped/attenuated grant type if that behavior lands later.
- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
