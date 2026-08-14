# ironclaw_event_streams

The transport-neutral stream manager: subscription authorization, RAII
admission control, bounded live/replay stitching, lag and rebase handling, and
read-only outbound push-candidate lookup. It never sends. It is a separate
crate because it alone is trusted to read the outbound-delivery domain's push
candidates — so the other three events crates can be reasoned about, and
depended upon, without ever considering delivery semantics — and because
keeping it apart from projections means subscription state can never leak into
fold logic.

- **Family / layer:** `crates/events/` / `substrates` · **Package:** `ironclaw_event_streams` · **Manifest:** `crates/events/ironclaw_event_streams/Cargo.toml`
- **Use this when:** a product surface needs a projection subscription
  authorized, admitted, and fed — snapshot, replay, or live — without knowing
  its transport.
- **Don't use this when:** you need SSE/WebSocket/channel framing (→ the
  transport crates in `crates/product/`), projection folding (→
  `ironclaw_event_projections`), or outbound *delivery* (→
  `ironclaw_outbound` and the product delivery coordinator — this crate only
  reads candidates).

## Public surface

- `EventStreamManager` — the construction entry point, generic over its
  injected collaborators: projection access policy, subscription admission
  policy, live-update source, redaction validator, outbound-state lookup.
- The RAII admission permit family (an abandoned subscription always releases
  its slot) and the bounded-buffer subscription types with explicit lag and
  rebase signals (`types`, `admission`).
- `ProjectionRedactionValidator` + `NoExposureProjectionRedactionValidator` —
  the stream-boundary redaction check that runs before anything crosses a
  wire.
- `ProjectionStreamError`; the update-source seam (`update_source`).

## Depends on / consumed by

- **Internal deps (normal):** `ironclaw_event_projections`,
  `ironclaw_host_api` (including its turn vocabulary), and
  `ironclaw_outbound` — the last for exactly one read-only push-candidate
  lookup (`ironclaw_event_log` appears only as a dev-dependency).
- **Consumed by 1 workspace crate** (reproduce:
  `grep -rl '^ironclaw_event_streams = ' --include=Cargo.toml crates`):
  `ironclaw_assistant`, which adapts subscriptions into product transports.

## Invariants

- **Authorize before anything.** The actor/scope/view/target check must pass
  before any snapshot, replay, or live delivery is returned; tenant/user scope
  is authority-bearing.
- **Watch and push are two decisions.** Subscription authorization never
  implies delivery eligibility; the outbound dependency is read-only by
  design, and a write path into outbound from here is always a defect.
- **Fails closed at the boundary** on raw prompts, tool input/output, secrets,
  host paths, provider errors, invocation fingerprints, approval reasons,
  lease material, and backend diagnostics (redaction validator).
- **No transport, no sends, no durable store, no direct evidence access** —
  the `BoundaryRule` in `reborn_dependency_boundaries.rs` forbids
  `ironclaw_event_log`, `ironclaw_event_store`, `ironclaw_filesystem`, and
  every transport/product crate; same-layer edges pinned in
  `reborn_same_layer_edge_inventory.rs` (`event_streams →
  {event_projections, outbound}`).
- **Test through `EventStreamManager`** when a helper gates access, admission,
  redaction, source subscription, or outbound lookup — not the helper alone.

## Tests

```bash
cargo test -p ironclaw_event_streams
cargo test -p ironclaw_architecture_tests
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance;
  `CLAUDE.md` points here).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.3.4;
  `docs/internal/reborn/target-architecture/families/events.md`.
