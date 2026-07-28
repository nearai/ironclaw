---
paths:
  - "crates/**/*.rs"
  - "migrations/**"
---
# Reborn persistence rules

## One storage plane

New persistence uses the `RootFilesystem` mount catalog. Consumers receive a
`ScopedFilesystem` and typed domain wrappers; they do not choose backends or
maintain parallel backend-dispatch traits.

Read `crates/ironclaw_filesystem/CLAUDE.md` and the owning domain contract before
changing storage. Re-verify the core surface with:

```bash
rg -n "trait RootFilesystem|struct ScopedFilesystem|fn cas_update" crates/ironclaw_filesystem
```

## Ownership

- `ironclaw_filesystem` owns paths, mounts, containment, versions, and CAS.
- Domain crates own record schemas, serialization, and invariants.
- Composition chooses concrete backends and mounts them.
- Product workflow consumes typed stores and never reaches through them to a
  backend.

Do not add a domain DTO or policy branch to the filesystem crate merely because
it is persisted.

## Adding a persistence operation

1. Identify the domain owner in `crates/AGENTS.md` and read its local contract.
2. Define the typed operation and record shape in that domain crate.
3. Use an existing scoped mount, or add a mount through the filesystem catalog
   and composition wiring when the domain is genuinely new.
4. Keep backend selection in composition. Domain stores do not branch on
   PostgreSQL, libSQL, or local-filesystem configuration.
5. Use `cas_update` for versioned filesystem mutation. Use a backend transaction
   for a backend-native multi-statement invariant.
6. Add contract tests at the public domain-operation or typed-wrapper seam, then
   a production-composition test when mount selection, restart, or cross-domain
   behavior is involved.

Review flags:

- a new `Store`/`Repository` trait whose only purpose is choosing a backend;
- a consumer opening a concrete backend connection;
- a typed store accepting `RootFilesystem` when `ScopedFilesystem` is enough;
- a write that reads a version and later overwrites without CAS;
- a per-record async mutex held across filesystem/backend I/O.

## Atomicity and concurrency

Every read-modify-write uses the shared bounded CAS update path. Do not hold a
process-local mutex across backend I/O: it cannot coordinate multiple processes
and can convoy the runtime. Multi-record invariants require an owning domain
operation with an explicit failure and recovery contract.

Backend-native stores that perform multiple SQL statements (`INSERT` plus
`INSERT`, `UPDATE` plus `DELETE`, or read-then-write) must wrap the full invariant
in one transaction. Sequential awaited calls are not atomic. Where both
PostgreSQL and libSQL implementations exist, test the same commit/rollback and
concurrency behavior through a shared conformance suite.

Persisted state must remain reconstructible after interruption. Test conflict,
retry exhaustion, restart, and partial-failure behavior at the public
domain-operation or typed-wrapper seam.

## Backend parity

When a domain explicitly supports multiple durable backends, keep behavioral
parity for ordering, uniqueness, timestamps, indexes, transactions, and error
classification. Put adversarial parity cases in a shared conformance suite
instead of copying tests per implementation.

Parity is behavioral, not merely schema-shaped. Compare uniqueness and indexes,
timestamp precision and ordering, JSON/enum serialization, transaction rollback,
concurrent-writer outcomes, seed/default records, migration replay, and error
classification. When fixing one implementation, search its peers and the shared
conformance suite for the same pattern.

## Data safety

Never silently discard model output, audit events, transcripts, or user data.
Destructive operations require an explicit product contract, authorization, and
tests for scope isolation. Storage errors must retain their server-side cause
while returning a sanitized boundary error.

Cache eviction is not durable deletion. A cache miss reloads from the owning
store. Retention or deletion features require explicit tenant/user scope,
auditable evidence, restart-safe behavior, and tests proving unrelated records
survive.
