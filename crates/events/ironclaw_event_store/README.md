# ironclaw_event_store

Durable backend selection and fail-closed production-profile policy for event
and audit logs — the concrete `DurableEventLog`/`DurableAuditLog` adapters over
the storage fabric, and the composition-facing factory that pairs them. It is a
separate crate because it is the only member of the family permitted a
database/TLS driver cone: isolating it here means every producer and consumer
of evidence — and everything that depends on them — never compiles that cone at
all.

- **Family / layer:** `crates/events/` / `substrates` · **Package:** `ironclaw_event_store` · **Manifest:** `crates/events/ironclaw_event_store/Cargo.toml`
- **Use this when:** composition needs durable logs constructed by profile, or
  a backend adapter itself needs work.
- **Don't use this when:** you produce events (→ `ironclaw_event_log` traits,
  with the log handed to you by composition), read models (→
  `ironclaw_event_projections`), or subscriptions (→
  `ironclaw_event_streams`). No crate outside composition should call the
  factory.

## Public surface

- The backend-selection entry point:
  `build_reborn_event_stores_from_root_filesystem` over
  `RebornEventStoreConfig` + `RebornProfile`, returning `RebornEventStores`
  (a paired durable event + audit log), failing with `RebornEventStoreError`.
- Concrete adapters: `FilesystemDurableEventLog` / `FilesystemDurableAuditLog`
  (routed through `RootFilesystem`, which is where libSQL/Postgres dispatch
  happens), and `JsonlDurableEventLog` / `JsonlDurableAuditLog`.
- `CoalescingEventSink` + `EventBatchConfig` for high-frequency producers.
- Postgres pool plumbing that never leaks the driver:
  `open_postgres_pool_with_tls_options` returns
  `ironclaw_filesystem::PostgresConnectionPool`;
  `PostgresPoolTlsOptions` / `RebornPostgresSslMode` validate TLS fail-closed.
  The crate declares **no cargo features**.

## Depends on / consumed by

- **Internal deps (normal):** `ironclaw_event_log`, `ironclaw_filesystem`,
  `ironclaw_host_api` (`ironclaw_common` is dev-only). External: the family's
  entire DB/TLS cone — `deadpool-postgres`, `tokio-postgres(-rustls)`,
  `libsql`, `rustls` — lives here and nowhere else in the family.
- **Consumed by 5 workspace crates** (reproduce:
  `grep -rl '^ironclaw_event_store = ' --include=Cargo.toml crates`):
  `ironclaw_composition` (the sanctioned factory caller), `ironclaw_host_runtime`,
  `ironclaw_processes`, `ironclaw_turn_runner`, and
  `ironclaw_event_projections` (dev).

## Invariants

- **Fail-closed backend selection is policy, not convention.** A production
  profile must explicitly accept single-node durability and must reject
  cleartext or ambiguous remote targets; there is no implicit fallback to a
  non-durable backend.
- **The driver never appears in the public API**, and may be *named* only
  inside the private `postgres_backed` module body — scanned brace-matched
  over every `src/` file by
  `reborn_persistence_driver_boundary.rs::event_store_names_the_driver_only_inside_its_private_backend_module`
  (the module going `pub` fails by name; an unterminated body panics rather
  than exempting the rest of the file). Driver linkage itself is chartered by
  `only_chartered_crates_link_the_postgres_driver` /
  `only_chartered_crates_link_the_other_persistence_drivers`.
- **Errors are redacted and backend-generic** regardless of which backend
  produced them.
- **No projections, no transports, no workflow** — `BoundaryRule` in
  `reborn_dependency_boundaries.rs`; same-layer edges pinned in
  `reborn_same_layer_edge_inventory.rs` (`event_store → {event_log,
  filesystem}`).

## Tests

```bash
cargo test -p ironclaw_event_store
cargo test -p ironclaw_architecture_tests --test reborn_persistence_driver_boundary
cargo test -p ironclaw_architecture_tests
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance — this
  crate has no `CLAUDE.md`, by design).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.3.2;
  `docs/reborn/target-architecture/families/events.md`; storage-placement
  rule: `docs/reborn/contracts/storage-placement.md`.
