# ironclaw_openai_compat

The OpenAI-shaped ingress adapter over the product surface: the wire contract
for Chat Completions and Responses, its route descriptor table, a sanitized
error taxonomy, the opaque-ref/idempotency store scoped to this surface, and —
since the WS6 eviction — its own route-mount assembly (`mount.rs`). It is a
separate crate so the assembly layer can mount the API skin without dragging
in `ironclaw_webui`'s embedded SPA and listener stack, and so the
wire-stability promise rides on its own contract tests.

- **Family / layer:** `product` / `products` · **Package:** `ironclaw_openai_compat` · **Manifest:** `crates/product/ironclaw_openai_compat/Cargo.toml`
- **Use this when:** changing the OpenAI-compatible wire contract — DTOs,
  route descriptors, error envelope, ref/idempotency semantics, SSE
  translation, or the mount's builder order.
- **Don't use this when:** changing what a command *does* →
  `ironclaw_assistant` behind `ironclaw_product_contracts`; binding a
  listener or authenticating callers → `ironclaw_webui`; building the port
  *implementations* the mount consumes (they name `ironclaw_turns`/
  `ironclaw_event_streams`, on this crate's forbidden list) →
  `ironclaw_composition`.

## Public surface

- `openai_compat_route_mount` (`mount.rs`) — takes
  `OpenAiCompatRouteMountPorts` (product surface, ref store, projection
  readers, external-tool store/resume, optional `LlmConfigService`) and
  returns an `ironclaw_host_ingress::ProtectedRouteMount`.
- `openai_compat_router` / `OpenAiCompatRouterState` (`router.rs`) with
  `with_chat_completions(...)` / `with_responses(...)` injection — fail-closed
  (`501`) until composition injects workflows.
- Wire DTOs (`chat`, `responses`, `models`), `OpenAiCompatError` helpers
  (`error.rs`), the descriptor table (`descriptors.rs`), the
  `OpenAiCompatRefStore` port + durable adapter (`refs`, `refs_storage`), and
  `OpenAiCompatModelCatalog` (`models_catalog`, `mount::LlmConfigModelCatalog`).

## Depends on / consumed by

- **Normal workspace deps (8):** `ironclaw_product_contracts` (the membrane
  and everything it speaks), `ironclaw_extension_contracts` (one enum,
  `ProductTriggerReason`), `ironclaw_host_api`, `ironclaw_host_ingress` (its
  carrier), `ironclaw_filesystem` (the ref ledger), `ironclaw_common`,
  `ironclaw_threads` (the accept door's seed vocabulary + its
  `validate_prepared_seed_content`, never thread services) — and
  `ironclaw_assistant`, whose residue is exactly **three** frozen command
  constants (`SUBMIT_TURN_COMMAND`, `CREATE_THREAD_COMMAND`,
  `CANCEL_RUN_COMMAND`), pinned exact-match and shrink-only; §12.11 D-B names
  the one DTO (`RebornCreateThreadResponse`) whose hoist would close the edge
  outright.
- **Consumed by (2):** `ironclaw_composition` (fills the mount's ports for
  `ironclaw serve`) and `ironclaw_webui` (stamps caller scope onto the shared
  protected mounts).

## Invariants

- **The product residue is pinned** at 3 constants, shrink-only:
  `reborn_transport_product_boundary.rs`.
- **No socket, ever** — no `TcpListener::bind`/`axum::serve` here:
  `reborn_dependency_boundaries.rs::reborn_product_api_crates_do_not_bind_http_ingress`.
- **BoundaryRule** forbids `ironclaw_turns`, `ironclaw_event_streams`, and
  runtime/lane crates (`reborn_crate_dependency_boundaries_hold`) — which is
  why the projection adapters live in composition and arrive as ports.
  `ironclaw_threads` is the one domain carve-in: the prepared lane speaks the
  accept door's seed vocabulary and runs the door's own
  `validate_prepared_seed_content` pre-reservation (one validator); thread
  services still arrive as ports.
- Client-supplied `tools`/`tool_choice` are model hints only, never executed
  as capabilities from this crate; auth evidence is minted by host middleware,
  never here; unauthorized and nonexistent refs stay indistinguishable.
- No cargo features at all — everything compiles unconditionally.

## Tests

```bash
cargo test -p ironclaw_openai_compat
cargo clippy -p ironclaw_openai_compat --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold
```

## See also

Working rules (wire contract, workflows, DTO policy): `AGENTS.md` · family
rules: `crates/product/AGENTS.md` · design record:
`docs/internal/reborn/target-architecture/families/product.md` (§6.9.3).
