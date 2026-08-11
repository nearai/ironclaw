# Agent Map — ironclaw_host_ingress

Working rules for the route-mount carrier crate. Orientation lives in
`README.md`; family rules in `crates/product/AGENTS.md`.

## What This Crate Owns

- Host HTTP route mount carriers that bind concrete Axum routers to
  `ironclaw_host_api::ingress::IngressRouteDescriptor` policy descriptors.
- Public/protected/split route mount structs and public-route drain hooks used
  by host ingress assembly.

## Do Not Move In Here

- Host API authority vocabulary, route policy descriptors, IDs, scopes, or
  product DTOs — route/policy *vocabulary* stays in `ironclaw_host_api`; this
  crate consumes those descriptors and pairs them with concrete routers.
- Listener binding, authentication enforcement, middleware construction,
  product workflow, runtime composition, persistence, provider clients, or
  WebUI-specific policy.

## Boundaries

- This crate may depend on Axum and `ironclaw_host_api`, and on nothing else
  in the workspace — **in any dependency kind**, `[dependencies]`,
  `[dev-dependencies]`, and `[build-dependencies]` alike. That is **enforced
  as an allowlist**, not just stated here:
  `assert_host_ingress_names_no_other_workspace_crate` in
  `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`
  derives the forbidden set as "every workspace `ironclaw_*` crate except
  `ironclaw_host_api`", so a dependency nobody thought to blocklist still
  fails, and it reads every kind rather than only `normal`. The all-kinds
  scope is deliberate and unlike the layer matrix beside it: a dev-dependency
  on a higher layer is legal elsewhere in the workspace, but a dev- or
  build-dependency here still resolves and builds, which defeats the one
  property this crate exists to provide.
- **Layer `substrates`** (WS2 re-layer, #7092 — it was `products` until then).
  The layer matrix will not hold the rule above on its own: `substrates →
  substrates` is legal, so the matrix would permit `ironclaw_filesystem` or
  `ironclaw_secrets` here. The allowlist is what refuses them. Do not "fix" a
  new dependency by adding it to the allowlist — the crate exists so that a
  consumer wanting only the carrier shapes never compiles a listener's
  dependency cone.

## Validation

- `cargo check -p ironclaw_host_ingress`
- `cargo test -p ironclaw_architecture_tests` after dependency changes.
