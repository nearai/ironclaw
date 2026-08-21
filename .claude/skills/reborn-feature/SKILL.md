---
name: reborn-feature
description: Use when building or extending a user-facing WebUI feature, endpoint, ProductSurface command/capability, or product-level API surface in the Reborn stack, or when adding/changing trigger and automation domain behavior (schedules, run-now, trigger history).
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

- `ironclaw_assistant`: product DTOs, the concrete `ProductView` and
  command/capability descriptor instances, and product orchestration.
- `ironclaw_product_contracts`: the `ProductSurface` contract, the
  `ProductView`/`ProductSurfaceCommandDescriptor`/`ProductCapabilityDescriptor`
  types, and the `ProductSurfaceCaller`/`BoundProductSurface` caller binding.
- `ironclaw_host_api`: shared host-facing error vocabulary
  (`ProductAdapterError`).
- `ironclaw_webui`: route descriptors, handlers, gateway/listener/auth, and
  the Vite frontend under `frontend/`.
- `ironclaw_composition`: production assembly and dependency wiring.
- `ironclaw_cli`: boot and serve command wiring.

## Before editing

Run the graph status check once. If it is missing or stale, use targeted
`rg` searches and verify the result against live code.

```bash
bash scripts/codebase-graph.sh status
rg -n "ProductSurface|ProductView|ProductSurfaceCommandDescriptor|ProductCapabilityDescriptor" crates/product/ironclaw_assistant crates/contracts/ironclaw_product_contracts crates/product/ironclaw_webui
rg -n "descriptor|webui_v2_routes|ProductSurface" crates/product/ironclaw_webui/src/webui_v2
```

Read the owning crate's `AGENTS.md`, then `CLAUDE.md` or `CONTRACT.md` when
present. Find the nearest existing descriptor and copy its narrow pattern.

## Default implementation

1. Add or reuse a typed `ProductView<Params, Output>` in
   `crates/product/ironclaw_assistant/src/reborn_services.rs` or its owning submodule.
2. Add or reuse a `ProductSurfaceCommandDescriptor` for typed product
   commands, or a `ProductCapabilityDescriptor` for API-only side effects.
3. Implement the backing behavior inside `ironclaw_assistant` or the owning
   service. Keep authorization, approval, persistence, and runtime mediation
   in their existing stages.
4. Add the route descriptor and thin handler in `ironclaw_webui`. Handlers
   receive `ProductSurfaceCaller` and use `BoundProductSurface`; they do not
   reach into composition, stores, dispatchers, or runtime lanes.
5. Add the frontend code under `crates/product/ironclaw_webui/frontend/src` and use
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

## Automations/triggers work

Trigger/automation domain work (cron/once schedules, run-now, trigger history
and settlement) crosses `crates/domains/ironclaw_triggers`, its trusted-submit
wiring in `crates/app/ironclaw_composition/src/automation/`, and the WebUI
automations surface (`crates/product/ironclaw_webui/frontend/src/pages/automations/`).
Two things to get right before touching this path:

- **Sealed ingress is a hard invariant, not a convention.** Read root
  `AGENTS.md` → "Host-trusted trigger ingress is sealed by..." before writing
  any code here. Product adapters, product workflow, first-party
  capabilities, and host-runtime handlers use untrusted inbound requests and
  must never mint `TrustedInboundTurnRequest` or call trusted trigger
  submitter factories — only trigger-worker-owned minting and private
  conversation-owned trusted construction may. Verify with
  `rg -n "TrustedInboundTurnRequest|ConversationTrustedTriggerSubmitter" crates/` (today
  the only hits are the allowed owner, `crates/domains/ironclaw_conversations`, plus the
  architecture test itself — any hit outside that crate is a prohibited caller); the
  boundary is enforced by
  `untrusted_ingress_paths_cannot_submit_host_trusted_inbound` in
  `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`.
- **Settlement, fire identity, and run-history ordering are specified, not
  improvised.** `crates/domains/ironclaw_triggers/AGENTS.md` and
  `docs/internal/reborn/contracts/triggers.md` are the source of truth — read
  both before changing schedule, claim, settlement, or history behavior;
  this class of invariant has a history of needing multiple follow-up fixes
  to get right, so treat first-pass changes here as review-heavy.

## Boundary rules

- WebUI handlers consume `ProductSurface` only. `ironclaw_assistant` imports in
  WebUI are limited to wire DTOs and descriptors.
- Composition assembles dependencies; it does not own product policy.
- External input is validated and bounded at the HTTP or adapter boundary.
- Mutations use the capability path and report authoritative evidence; durable
  state is read back when the contract requires it.
- Identity and scope come from the authenticated caller, never the request
  body.

## Verification

```bash
cargo test -p ironclaw_assistant
cargo clippy -p ironclaw_assistant --all-targets --all-features -- -D warnings
cargo test -p ironclaw_webui --all-features
cargo clippy -p ironclaw_webui --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture_tests  # when ownership or dependencies change
pnpm --dir crates/product/ironclaw_webui/frontend test
```

Use a caller-level test for every new route or side effect. Add a whole-path
integration test when the feature changes turn execution or cross-layer
behavior. Do not add a new test tier solely because a recipe lists it.
