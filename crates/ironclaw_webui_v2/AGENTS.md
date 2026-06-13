# Agent Map — ironclaw_webui_v2

## Start Here

- Read `CLAUDE.md` first; it lists the route table and boundary rules.
- Read `Cargo.toml` for the feature gate and dependency shape.
- Use these contracts as the source of truth before changing behavior:
  - `src/descriptors.rs`
  - `tests/webui_v2_descriptors_contract.rs`
  - `tests/webui_v2_handlers_contract.rs`

## What This Crate Owns

- The native WebChat v2 routes and their host-owned ingress
  descriptors.
- Axum handler functions that dispatch to `RebornServicesApi`.
- The HTTP wire shape (`WebUiV2HttpError`) for redacted error
  responses.

## Do Not Move In Here

- Bearer-token validation, CSRF/origin enforcement, body/rate-limit
  middleware. Those live in host composition, gated by the
  `IngressPolicy` the descriptor declares.
- Direct access to dispatcher, `HostRuntime`, run-state, DB stores,
  capability hosts, or any runtime lane.
- Product adapter transport/rendering logic, storage backend details,
  or unredacted user content in responses, logs, or docs.

## Agent Notes

- Adding a new route requires both a handler **and** a matching entry
  in `webui_v2_routes()`. The descriptor test will fail otherwise.
- Handlers receive `WebUiAuthenticatedCaller` via `axum::Extension`.
  If the extension is missing, axum surfaces 500 — exercise this with
  the regression test in `webui_v2_handlers_contract.rs` rather than
  hand-rolling a fallback.
- All HTTP errors must travel through `WebUiV2HttpError`, never
  hand-built `StatusCode` returns. That keeps the redacted-error
  vocabulary intact.

## Validation

- Fast: `cargo test -p ironclaw_webui_v2 --features webui-v2-beta`
- Lint: `cargo clippy -p ironclaw_webui_v2 --all-features --tests -- -D warnings`
- Boundary: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
