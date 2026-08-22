# ironclaw_assistant

The product-facing orchestration crate — the personal assistant *is* the
product (renamed from the former product crate so the family's central crate
does not collapse into `product/product`). It implements the `ProductSurface`
end to end: binding resolution, command admission, the idempotency ledger,
delivery semantics, projection-to-view assembly, and the click-approval and
click-auth interaction services that stand between a human and a blocked run.
Every front door (browser SPA, OpenAI-compatible clients, channel adapters)
shares this one implementation instead of reimplementing it.

- **Family / layer:** `product` / `products` · **Package:** `ironclaw_assistant` · **Manifest:** `crates/product/ironclaw_assistant/Cargo.toml`
- **Use this when:** changing what a product command, view, capability, or
  delivery *means*; admission/binding/idempotency behavior; approval/auth
  interaction flows.
- **Don't use this when:** declaring a port or DTO other crates consume → it
  belongs in `ironclaw_product_contracts` (the trait's shape is contracts';
  this crate is the canonical implementation and the frozen descriptor-
  *constant* inventory); serving HTTP → `ironclaw_webui` /
  `ironclaw_openai_compat`; wiring dependencies → `ironclaw_composition`.

## Public surface

- `RebornServices` — the `ProductSurface` implementation (517 items, 19
  sub-owners; the charter map in `AGENTS.md` is pinned by
  `tests/reborn_services_module_charter.rs`).
- `DefaultProductSurface`, `InboundTurnService`, `ConversationBindingService`,
  `IdempotencyLedger`/`ProductInboundAction`, `ProductCommandAdmissionService`.
- `DeliveryCoordinator` + run-delivery drivers (delivery *semantics*; the
  at-most-once reservation itself is `ironclaw_outbound`'s).
- `RunOutcomeProcessCommitObserver` — materializes selected scheduled-run
  completion/failure facts from the authoritative process journal; completion
  requires the exact finalized assistant reply, while delivery failure remains
  a separate Inbox fact from the external-delivery caller.
- `ApprovalInteractionService` / `AuthInteractionService` and their redacted
  read models.
- The frozen command/view/capability descriptor constants (`*_COMMAND`,
  `*_VIEW`, `*_CAPABILITY`) — PROPOSAL §6.1.3's inventory, the reason
  transports still name this crate at all.

## Depends on / consumed by

- **Normal workspace deps (24):** contracts (`ironclaw_host_api`,
  `ironclaw_common`, `ironclaw_product_contracts`,
  `ironclaw_extension_contracts`, `ironclaw_loop_contracts`), the domain
  crates owning conversation/thread/outbound/trigger/identity state, kernel
  admission and approval-resolution ports (`ironclaw_turns`,
  `ironclaw_approvals`, `ironclaw_authorization`, `ironclaw_processes`),
  events (`ironclaw_event_log`, `ironclaw_event_projections`,
  `ironclaw_event_streams`), substrates (`ironclaw_filesystem`,
  `ironclaw_safety`, `ironclaw_secrets`, `ironclaw_attachments`,
  `ironclaw_auth`, `ironclaw_trace_commons`) — and `ironclaw_loop_host`, the
  measured `products → loops` edge PROPOSAL §6.10.1 scopes at 6 production
  files / 4 seams (input-enqueue, attachment-read port, synthetic-capability
  family, one doc comment); severing it is a design slice, not a move.
- **Consumed by (4):** `ironclaw_webui` and `ironclaw_openai_compat`
  (frozen-constant residue only — 100 and 3 symbols, pinned),
  `ironclaw_extension_manager`, `ironclaw_composition`.

## Invariants

- **The surface's method set is frozen** (`invoke`/`query`/`stream_events`):
  `reborn_service_method_freeze_ratchet.rs::host_product_surface_method_set_is_frozen`.
- **No foreign re-export facade**: `lib.rs` declares zero foreign `pub use`
  (`product_declares_no_foreign_re_export_facade`); ports this crate consumes
  but does not implement live in `ironclaw_product_contracts`
  (`reborn_extension_host_port_inversion.rs`,
  `reborn_operator_port_inversion.rs`, `reborn_product_contract_location_scan.rs`).
- **BoundaryRule**: must not depend on `ironclaw_extension_registry`,
  `ironclaw_host_runtime`, `ironclaw_mcp`, `ironclaw_wasm`,
  `ironclaw_sandbox`, `ironclaw_network`
  (`reborn_dependency_boundaries.rs::reborn_crate_dependency_boundaries_hold`).
- Admission is a durability decision, never an authority one; approval/auth
  interactions are strictly redacted and routed through canonical resolution
  ports (`ApprovalResolutionPort`, `AuthFlowManager`, `TurnCoordinator`).
- Run delivery publishes metadata-only approval/auth gate records to the
  actor's or trigger creator's durable notification inbox independently of
  external-channel availability. After external delivery settles, an
  Inbox-only observer gets one additional bounded `max_wait` window to resolve
  or replace the stable gate-derived id without retaining a delivery permit;
  abandoned gates never create process-lifetime polling tasks. An external
  preference/catalog outage must not be reclassified as "no channel
  configured."
- `AGENTS.md` here is **gate-pinned** (the `reborn_services` module-charter
  map) — edit only with `cargo test -p ironclaw_assistant` green.

## Tests

```bash
cargo test -p ironclaw_assistant
cargo clippy -p ironclaw_assistant --all-targets -- -D warnings
cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold
```

## See also

Working rules + charter map: `AGENTS.md` (gate-pinned) · family rules:
`crates/product/AGENTS.md` · design record:
`docs/internal/reborn/target-architecture/families/product.md` (§6.9.1).
