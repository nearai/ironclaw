# Pending Gate Projection

`PendingGateProjection` materializes a Reborn pending-gate read model from
`TurnLifecycleEvent`. It is the only Reborn path that should write derived
pending-gate rows.

The legacy root `src/gate/PendingGateStore` remains engine-owned until product
composition adapts `PendingGateProjectionSink` into that store. Existing legacy
writer audit:

- `src/bridge/gate_controller.rs` inserts pending gates for the legacy inline
  engine pause path.
- `src/bridge/router.rs` requeues and removes pending gates through legacy web
  bridge routes, including `/api/chat/gate/resolve`.
- `src/gate/store.rs` owns the legacy `insert`, verified-resolution removal,
  thread cleanup, and key discard helpers.

Follow-up composition work must either adapt this projection into the root UI
store or retire those legacy writers for Reborn-backed turns. Do not add a new
direct writer for Reborn blocked turns.
