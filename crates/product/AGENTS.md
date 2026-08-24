# `crates/product/` — first-party userland above the kernel; product asks, kernel decides

**Layer(s):** `products`, except `ironclaw_host_ingress` at `substrates` (WS2 re-layer, #7092) · **Crates:** 5 · **May depend on:** `contracts/`, `substrates/`, `events/`, `domains/`, `kernel/` (read-model/resolution ports only — never `ironclaw_host_runtime`, dispatch, or lanes), `loop/` (downward), siblings · **Depended on by:** `ironclaw_composition` and the `ironclaw` binary (app); `ironclaw_extension_manager` consumes `ironclaw_assistant`; `ironclaw_extension_host` consumes `ironclaw_host_ingress`

## What this family is

The supported first-party experience: the one place that turns validated
channel and HTTP traffic into admitted, idempotent, durably bound turns, and
turns kernel and domain state back into redacted, product-safe views and
deliveries. Three front doors — the browser SPA, OpenAI-compatible API
clients, and (one layer down, in the extensions family) channel adapters —
share one implementation of binding resolution, idempotency, command grammar,
and delivery policy instead of three. The family holds no standing authority:
its trust jobs are host authentication at the listener, admission and
idempotency at the product boundary, and delivery semantics that hand off to
an at-most-once reservation it does not own (`ironclaw_outbound`).

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_assistant`](./ironclaw_assistant) | The `ProductSurface` implementation: admission, binding, idempotency, delivery semantics, click-approval/auth interaction services | You are changing what a product command, view, or delivery *means* |
| [`ironclaw_webui`](./ironclaw_webui) | The host-owned WebChat v2 HTTP gateway: routes, SPA, middleware, serve loop, host authentication | You are changing the browser-facing surface or host auth |
| [`ironclaw_openai_compat`](./ironclaw_openai_compat) | The OpenAI-shaped ingress adapter: wire DTOs, route descriptors, sanitized errors, ref/idempotency store | You are changing the OpenAI-compatible wire contract |
| [`ironclaw_operator`](./ironclaw_operator) | The deployment-operator control plane: LLM provider admin, operator log ring, OS service lifecycle | You are changing operator administration, not conversation |
| [`ironclaw_host_ingress`](./ironclaw_host_ingress) | Route-mount carriers pairing a prebuilt Axum router with ingress policy descriptors | You need to hand a router across a crate boundary without a web-framework dep |

## What never belongs here

- **Authority decisions of any kind** — authorizing a capability, minting an
  authorization witness, holding ambient secrets or network handles. Kernel
  family. Product narrates and asks through resolution ports it does not
  implement (click-approval/click-auth forward a human's decision; they never
  execute a tool or mutate an approval record).
- **Minting trusted inbound evidence.** Sealed to two places, neither of them
  general product code: `ironclaw_webui`'s authentication middleware (the only
  product-family constructor of authenticated-caller evidence, minted via
  `ironclaw_host_api`'s sealed constructor) and the extension family's ingress
  verifier for webhooks. A handler never fabricates evidence.
- **Reaching around `ProductSurface`/capability dispatch to mutate domain
  stores.** Transports consume `ironclaw_product_contracts` — the trait, DTOs,
  and descriptor *types* — never `ironclaw_assistant`'s behavioral mass. (The
  surviving `webui`/`openai_compat` → `ironclaw_assistant` manifest edges are
  the frozen descriptor-*constant* inventory §6.1.3 keeps in product — 104 and
  3 symbols, exact-match and shrink-only — not a license to import behavior;
  §12.11 D-B rules webui's edge charter-sanctioned and permanent.)
- **Kernel stage or lane mechanics** — no `ironclaw_host_runtime`, no
  `ironclaw_wasm`/`ironclaw_mcp`/`ironclaw_sandbox`, no direct network
  substrate, no extension registry.
- **Vendor protocol**, beyond exactly two named exceptions: LLM-vendor
  administration inside `ironclaw_operator`, and OAuth login providers inside
  `ironclaw_webui`'s `src/auth/` module. Nowhere else.
- **Assembly or wiring logic** — factories constructing another crate's
  concrete stores, deployment-mode branching. That is `crates/app/`'s job;
  anything assembly-shaped in a product crate is a boundary violation.
- **Raw secrets, raw host paths, backend error strings, unredacted user
  content** in any product-family error, event, snapshot, or log.

## The rules, and what enforces them

- **The frozen-surface discipline.** The product surface's method set
  (`invoke` / `query` / `stream_events`) is a stable contract; feature work
  adds a capability or view descriptor, never a product-local service method:
  `cargo test -p ironclaw_architecture_tests --test reborn_service_method_freeze_ratchet host_product_surface_method_set_is_frozen`
- **Transports consume contracts.** Each transport's `ironclaw_assistant`
  residue is pinned exact-match and shrink-only (webui 104 symbols,
  openai_compat 3 command constants):
  `cargo test -p ironclaw_architecture_tests --test reborn_transport_product_boundary`
- **Ports invert, never re-declare.** Product-facing ports live in
  `ironclaw_product_contracts`; `ironclaw_operator` implements them without
  naming `ironclaw_assistant` (proved through `cargo metadata`):
  `cargo test -p ironclaw_architecture_tests --test reborn_operator_port_inversion`
  — and no crate re-exports a contracts-declared trait:
  `cargo test -p ironclaw_architecture_tests --test reborn_product_contract_location_scan`
- **Only `ironclaw_webui` binds a listener or owns a web framework** as a
  first-class dependency; the lower product/API tier stays socket-free:
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries reborn_product_api_crates_do_not_bind_http_ingress`
- **BoundaryRules** for `ironclaw_assistant`, `ironclaw_webui`,
  `ironclaw_openai_compat`, `ironclaw_operator`, and the all-kinds allowlist
  for `ironclaw_host_ingress` (`{ironclaw_host_api}` only —
  `assert_host_ingress_names_no_other_workspace_crate`), plus its re-layer
  consumer freeze (`DOWNGRADE_PINS`):
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries reborn_crate_dependency_boundaries_hold`
  and `--test reborn_same_layer_edge_inventory`
- **Gate-pinned guidance.** Two crate files are contracts, not prose:
  `ironclaw_webui/CONTRACT.md` (route table + `handlers.rs` charter map;
  `cargo test -p ironclaw_webui --test handlers_module_charter`) and
  `ironclaw_assistant/AGENTS.md` (`reborn_services` charter map;
  `cargo test -p ironclaw_assistant --test reborn_services_module_charter`).
  Edit them only with the owning suite green; do not reflow or renumber.

## Crossing out of this family

- **Up to `crates/app/`:** never as a dependency — composition constructs this
  family's owners and hands `ironclaw_webui`/`ironclaw_openai_compat` the
  finished surface; the binary mounts and serves.
- **Down to `contracts/ironclaw_product_contracts`:** to change the surface's
  *shape* — the trait, wire DTOs, descriptor types, inverted ports.
- **Down to `kernel/`:** through narrow admission and approval/auth resolution
  ports (`ironclaw_turns`, `ironclaw_approvals`, `ironclaw_authorization`) —
  read models and coordinators, never dispatch.
- **Down to `domains/`:** the crates that own conversation, thread, and
  outbound state; the at-most-once delivery reservation is
  `ironclaw_outbound`'s.
- **Sideways to `extensions/`:** extension *management* UX
  (`ironclaw_extension_manager`) sits at the same layer but is a different
  family, organized around extension lifecycle; it consumes product's
  contracts. `ironclaw_webui`'s `ironclaw_extension_host` edge stops at the
  pairing service and never reaches lifecycle authority or installation
  stores.

## Sources

`docs/internal/reborn/target-architecture/families/product.md` · PROPOSAL §6.9.1–6.9.5,
§8, §12.11 D-B/D-H · gates: `crates/app/ironclaw_architecture_tests/tests/`
(`reborn_dependency_boundaries.rs`, `reborn_transport_product_boundary.rs`,
`reborn_service_method_freeze_ratchet.rs`, `reborn_operator_port_inversion.rs`,
`reborn_product_contract_location_scan.rs`,
`reborn_same_layer_edge_inventory.rs`) · module specs:
`ironclaw_webui/CONTRACT.md` (root `AGENTS.md` Module Specs table) · conventions:
`docs/internal/reborn/guidance-conventions.md`.
