# ironclaw_filesystem

The universal storage-dispatch fabric: one `RootFilesystem` trait, scoped and
mount-checked access above it, a mount catalog for composite routing, a
compare-and-swap floor every durable record type is built on, and the concrete
backends (disk, libSQL, PostgreSQL, in-memory) that implement durable storage.
It is one contract with many production backends and a database-driver cone
wide enough that no other crate should acquire it by accident — isolating it
here is what lets the rest of the workspace stay backend-agnostic.

- **Family / layer:** `substrates` / `substrates` · **Package:**
  `ironclaw_filesystem` · **Manifest:**
  `crates/substrates/ironclaw_filesystem/Cargo.toml`
- **Use this when:** bytes or records need a durable home — hold a handle
  against the `RootFilesystem` trait (usually via `ScopedFilesystem`), never
  against a concrete backend.
- **Don't use this when:** you need a raw libSQL connection → that is
  `ironclaw_libsql_runtime`'s admission lane; you're deciding whether a caller
  *may* touch a path → mount grants come from the kernel; you're recording
  append-only evidence → `ironclaw_event_store` is the durable-events consumer
  built on this fabric; you're choosing which backend a deployment uses →
  `ironclaw_composition` decides by profile.

## Public surface

- `RootFilesystem` — the one trait; every backend *and* the composite
  dispatcher implement it (read/write/list/stat/append/transactional ops over a
  virtual path space, with capability negotiation).
- `ScopedFilesystem` + `MountViewResolver` — the caller-facing wrapper that
  checks a caller's `MountView` before any backend dispatch.
- `CompositeRootFilesystem`, `MountDescriptor`, `PathPlacement` — the mount
  catalog (longest-prefix routing).
- `cas_update`, `Entry`/`VersionedEntry`, `CasExpectation`, `RecordKind`,
  `ContentType` — the CAS floor and record vocabulary.
- `IndexSpec`/`IndexKey`/`Filter`/`Page` — declarative index/query primitives;
  no SQL strings cross this boundary.
- Backends: `DiskFilesystem`, `LibSqlRootFilesystem`, `PostgresRootFilesystem`,
  `InMemoryBackend`, and the `HsmBackend` demo backend (plus containment:
  symlink traversal, mount escape, and raw-host-path prevention).

## Depends on / consumed by

- **Depends on (workspace):** `ironclaw_host_api`, `ironclaw_libsql_runtime`
  (connection admission — this crate never builds its own pool),
  `ironclaw_safety` (a single sensitive-path predicate used when redacting a
  path for display), `ironclaw_observability`. External: the driver cone —
  `libsql`, `deadpool`, `deadpool-postgres`, `tokio-postgres` — which is
  precisely what this crate exists to contain.
- **Consumed by (measured 2026-08-05):** 28 normal-dep consumers spanning
  domains, events, kernel, loops, products, and app — plus 6 dev-only. The
  breadth is the point: every domain record owner holds the trait, never a
  driver.

## Invariants

- **Path containment and mount authority** are enforced on every call once a
  mount view is handed in; the *grant* comes from the kernel.
- **Upward edges forbidden by name:** the `ironclaw_filesystem` `BoundaryRule`
  in `reborn_dependency_boundaries.rs`
  (`reborn_crate_dependency_boundaries_hold`).
- **Driver charter:** this crate is on every driver allowlist it needs and no
  other substrate is — `reborn_persistence_driver_boundary.rs`
  (`only_chartered_crates_link_the_postgres_driver`,
  `only_chartered_crates_link_the_other_persistence_drivers`).
- **Backend parity:** the same observable contract across backends, pinned by
  the conformance suites in `tests/` (`filesystem_contract.rs`,
  `catalog_contract.rs`, and the backend parity tests).

## Tests

```bash
cargo test -p ironclaw_filesystem            # unit + contract/parity suites
cargo test -p ironclaw_architecture_tests    # boundary + driver gates
```

The crate's one feature is `test-support` (the `FaultInjecting` decorator for
downstream fault-injection tests); PostgreSQL-backed integration coverage runs
from the workspace root via `cargo test --features integration`.

## See also

**Module spec:** [`CONTRACT.md`](./CONTRACT.md) — this crate is in the root
`AGENTS.md` Module Specs table; the spec is the tiebreaker and this README does
not restate it. Family boundary: [`crates/substrates/AGENTS.md`](../AGENTS.md).
Contracts: `docs/reborn/contracts/filesystem.md`,
`docs/reborn/contracts/storage-placement.md`,
`docs/reborn/contracts/kernel-boundary.md`.
