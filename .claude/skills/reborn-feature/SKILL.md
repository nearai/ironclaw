---
name: reborn-feature
description: Build a user-facing feature in the Reborn stack with the smallest existing contract surface.
---

# Building a Reborn feature

Start by locating the existing ProductSurface descriptor, route, and caller
test. Most WebUI features are already represented by one of these two paths:

```text
read:    WebUI handler -> ProductSurface::query -> ProductView descriptor
write:   WebUI handler -> ProductSurface::invoke -> capability descriptor
         -> query read-back when the result is durable state
```

The owning crates are:

- `ironclaw_product`: product DTOs, `ProductView`, command/capability
  descriptors, and product orchestration.
- `ironclaw_host_api`: the `ProductSurface` contract, caller binding, and
  shared host-facing error vocabulary.
- `ironclaw_webui`: route descriptors, handlers, gateway/listener/auth, and
  the Vite frontend under `frontend/`.
- `ironclaw_reborn_composition`: production assembly and dependency wiring.
- `ironclaw_reborn_cli`: boot and serve command wiring.

## Before editing

Run the graph status check once. If it is missing or stale, use targeted
`rg` searches and verify the result against live code.

```bash
bash scripts/codebase-graph.sh status
rg -n "ProductSurface|ProductView|ProductSurfaceCommandDescriptor|ProductCapabilityDescriptor" crates/ironclaw_product crates/ironclaw_host_api crates/ironclaw_webui
rg -n "descriptor|webui_v2_routes|ProductSurface" crates/ironclaw_webui/src/webui_v2
```

Read the owning crate's `AGENTS.md`, then `CLAUDE.md` or `CONTRACT.md` when
present. Find the nearest existing descriptor and copy its narrow pattern.

## Default implementation

1. Add or reuse a typed `ProductView<Params, Output>` in
   `crates/ironclaw_product/src/reborn_services.rs` or its owning submodule.
2. Add or reuse a `ProductSurfaceCommandDescriptor` for typed product
   commands, or a `ProductCapabilityDescriptor` for API-only side effects.
3. Implement the backing behavior inside `ironclaw_product` or the owning
   service. Keep authorization, approval, persistence, and runtime mediation
   in their existing stages.
4. Add the route descriptor and thin handler in `ironclaw_webui`. Handlers
   receive `ProductSurfaceCaller` and use `BoundProductSurface`; they do not
   reach into composition, stores, dispatchers, or runtime lanes.
5. Add the frontend code under `crates/ironclaw_webui/frontend/src` and use
   the existing API client and page patterns.
6. Wire only genuinely new production dependencies through composition and
   the CLI. Do not add a builder or `Arc` field when an existing surface can
   carry the operation.

## Add an abstraction only when it earns its keep

Do not add a feature-specific port, facade method, DTO family, builder field,
or adapter by default. Add one only when it provides dependency inversion,
two production implementations, a real test seam, a required `dyn` injection
point, or an enforced security/ownership boundary. Record the reason in the
PR description and run the architecture test for dependency changes.

## Boundary rules

- WebUI handlers consume `ProductSurface` only. `ironclaw_product` imports in
  WebUI are limited to wire DTOs and descriptors.
- Composition assembles dependencies; it does not own product policy.
- External input is validated and bounded at the HTTP or adapter boundary.
- Mutations use the capability path and report authoritative evidence; durable
  state is read back when the contract requires it.
- Identity and scope come from the authenticated caller, never the request
  body.

## Verification

```bash
cargo test -p ironclaw_product
cargo clippy -p ironclaw_product --all-targets --all-features -- -D warnings
cargo test -p ironclaw_webui --all-features
cargo clippy -p ironclaw_webui --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture  # when ownership or dependencies change
pnpm --dir crates/ironclaw_webui/frontend test
```

Use a caller-level test for every new route or side effect. Add a whole-path
integration test when the feature changes turn execution or cross-layer
behavior. Do not add a new test tier solely because a recipe lists it.
