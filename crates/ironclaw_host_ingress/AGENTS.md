# Agent Map — ironclaw_host_ingress

## What This Crate Owns

- Host HTTP route mount carriers that bind concrete Axum routers to
  `ironclaw_host_api::IngressRouteDescriptor` policy descriptors.
- Public/protected/split route mount structs and public-route drain hooks used
  by host ingress assembly.

## Do Not Move In Here

- Host API authority vocabulary, route policy descriptors, IDs, scopes, or
  product DTOs.
- Listener binding, authentication enforcement, middleware construction, product
  workflow, runtime composition, persistence, or provider logic.

## Validation

- `cargo check -p ironclaw_host_ingress`
- `cargo test -p ironclaw_architecture` after dependency changes.

