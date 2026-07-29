# ADR 0002: Native memory uses host storage ports

**Status:** Accepted (storage-port-backed persistence gated; see "Gated")
**Issue:** #3537

## Context

`ironclaw.memory` is the host-bundled provider for the memory capability
profiles (`memory.context_retrieval.v1`, `memory.interaction_log.v1`,
`memory.document_store.v1`). It must persist memory documents, chunks, versions,
and (optionally) embeddings.

If the memory extension captured a raw database pool (`deadpool_postgres::Pool` /
`libsql::Database`) or a raw `Arc<dyn RootFilesystem>`, it would hold ambient
authority that bypasses the host's scoping, obligation, and audit pipeline — the
same anti-pattern host HTTP egress already avoids by going through a staged,
host-mediated port.

## Decision

Native memory accesses durable storage through a **host-mediated storage port**,
not a raw handle:

- A first-party SQL/transaction storage port,
  `host.storage.sql_transaction.first_party`, is **HostBundled FirstParty only**.
  In the (deferred) SQL-backed variant it is declared as a `required_host_port`
  on every native memory capability and validated against the host
  `HostPortCatalog` at manifest parse time (an unknown port fails closed). The
  live filesystem-backed manifest declares no host ports.
- The `CapabilityHost` constructs a scoped `HostPortView` after
  auth/approval/obligation preparation and hands it to the first-party memory
  handler; the handler must not capture a raw service for a declared port.
- Native memory uses dedicated `reborn_memory_*` tables with explicit
  `(tenant_id, user_id, agent_id, project_id)` scope columns and a
  `UNIQUE (tenant_id, user_id, agent_id, project_id, path)` constraint. Absent
  `agent_id`/`project_id` are stored as the empty-string sentinel, never
  `_none`. Both backends (PostgreSQL + libSQL) are honored. Legacy
  `memory_documents` rows are **not** migrated from this crate.
- A host-mediated embedding/vector port is required before
  `memory.semantic_search.v1` ships; until then semantic search stays optional
  with a lexical (FTS) fallback. The native provider is FTS-only today.

## Gated

The vocabulary and contract are landed: the storage and audit ports
(`host.storage.sql_transaction.first_party`, `host.events.audit`) are registered
in `default_host_port_catalog()`. The live `ironclaw.memory` manifest is
filesystem-backed and declares **no** host ports; these stay catalogued
vocabulary that the deferred SQL-backed variant will declare and validate against. The **concrete `reborn_memory_*` dual-backend SQL
repository behind the storage port** is delivered behind the non-default
`memory-native-*` feature gate. The reborn composition crates must not depend on
the root `ironclaw` crate where the Postgres/libSQL backends live, so the
concrete adapter lands in a reborn-stack persistence crate.

The native provider therefore remains filesystem-backed by default (as the M2
lift left it), and the default flip to SQL-backed native memory is **blocked**
until the existing `/memory` data + API compatibility story is decided and tested
across caller boundaries (filesystem-mount, gateway/API, prompt-write-safety) —
see #3537 step 6.

## Consequences

- Memory storage authority is scoped and auditable, like every other mediated
  host API.
- Swapping the storage implementation (filesystem → SQL tables) is a port-impl
  change behind the same `MemoryService`/`MemoryDocumentRepository` seam, not a
  change to the memory extension's contract.
