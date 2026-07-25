# Reborn Memory Capability Profiles

**Status:** Active — the native memory extension is **live**: its bundled v2 TOML
manifest is parsed and registered on the always-on first-party lane (like the
builtin toolset), implementing `memory.document_store.v1` through model-facing
tools. SQL storage-port-backed persistence and the host-managed
context/interaction flow remain gated (see "Implemented" / "Deferred").
**Issue:** #3537

Memory profile contracts are host-defined portability targets. Extensions may claim they implement these profiles, but claims do not grant trust or authority by themselves. Certification of third-party providers and SQL-backed native storage remain later work.

## Profiles

| Profile | Required operation | Visibility | Live implementer | Host ports |
| --- | --- | --- | --- | --- |
| `memory.context_retrieval.v1` | `memory.context.retrieve.v1` | `host_internal` | none (deferred host-managed flow) | deferred¹ |
| `memory.interaction_log.v1` | `memory.interaction.record.v1` | `host_internal` | none (deferred host-managed flow) | deferred¹ |
| `memory.document_store.v1` | `memory.document.read.v1`, `memory.document.write.v1` | `model` | `ironclaw.memory` `read`/`write` tools | none² |

¹ Required only by the deferred host-managed flow; no live implementer today.
² The live native provider is filesystem-backed and declares no host ports. The
`host.storage.sql_transaction.first_party` + `host.events.audit` ports remain
catalogued vocabulary for the deferred SQL-backed milestone (see ADR 0002).

## Schema refs

Profile contracts use extension-local relative schema refs. These draft refs are catalog names for validation and conformance scaffolding:

```text
schemas/memory/context-retrieve.input.v1.json
schemas/memory/context-retrieve.output.v1.json
schemas/memory/interaction-record.input.v1.json
schemas/memory/interaction-record.output.v1.json
schemas/memory/document-read.input.v1.json
schemas/memory/document-read.output.v1.json
schemas/memory/document-write.input.v1.json
schemas/memory/document-write.output.v1.json
```

## Host-port catalog notes

`HostPortCatalog` is a validation catalog for known `HostPortId` contract names. It is not a runtime implementation registry, dependency injection container, or adapter factory. Concrete storage, audit, embedding, and network adapters stay in host/runtime service crates.

## Implemented

- **Profile catalog**: the three contracts are authored as host-defined code in
  `ironclaw_host_runtime::memory_profiles` with repo conformance tests.
- **Host ports**: `host.storage.sql_transaction.first_party` and
  `host.events.audit` are registered in
  `ironclaw_host_runtime::default_host_port_catalog()`.
- **Native v2 manifest (live)**: `ironclaw.memory` (HostBundled,
  `first_party` runtime) is parsed from its bundled TOML and registered on the
  **always-on first-party lane** (like the builtin toolset), not the
  catalog/lifecycle lane — so its tools are unconditionally available with no
  install/enable step. It declares four `model`-visible memory tools
  (`read`/`write`/`search`/`tree`); `read`/`write` `implements`
  `memory.document_store.v1` (their schema refs match the profile's operation
  refs), while `search`/`tree` are native conveniences that implement no profile.
  The live provider is filesystem-backed and declares no host ports; input
  schemas are served inline (`include_str!`) on the always-on lane rather than
  materialized. Its `service` must match the host-registered native provider
  identity. Conformance tests prove `read`/`write` satisfy
  `memory.document_store.v1`.
- **Profile binding**: the `[memory]` config section (`profile_bindings` +
  `admin_overrides`) resolves through a fail-closed
  `MemoryBindingPolicy` (`profile_id -> extension_id`, default-native; production
  rejects `memory.disabled` and unverified third-party bindings absent an
  `(extension_id, profile_id, deployment_profile)` override). The memory-tools
  dispatch site consults the binding instead of hardwiring the native provider;
  startup lists active overrides (redacted).

## Deferred

- **Host-managed context/interaction flow** — the `memory.context_retrieval.v1`
  and `memory.interaction_log.v1` profiles are defined (and conformance-testable)
  but have **no live implementer**. Wiring the host turn pipeline to invoke
  `memory.context.retrieve` before model calls and record sanitized interactions
  via `memory.interaction.record` afterward (the issue's "Host-managed memory
  flow") is deferred; the live native surface ships only the model-facing
  document-store tools.
- `memory.semantic_search.v1` — depends on a host-mediated embedding/vector port
  that does not exist yet. Must be added before semantic search ships, either as
  its own profile with a host-mediated embedding port or kept behind a separate
  optional feature with a lexical (FTS) fallback.
- **SQL storage-port-backed native persistence** — the concrete
  `reborn_memory_*` dual-backend tables behind
  `host.storage.sql_transaction.first_party`, and the scoped `HostPortView`
  threaded into the memory handler in place of a raw `RootFilesystem`. Gated
  behind non-default `memory-native-*` features (the reborn composition crates
  cannot depend on the root `ironclaw` crate where the Postgres/libSQL backends
  live). See `docs/adr/0002-native-memory-uses-host-storage-ports.md`.
- **Default flip** — blocked until `/memory` data + API compatibility is decided
  and tested across caller boundaries (filesystem-mount, gateway/API,
  prompt-write-safety). Legacy `memory_documents` migration stays deferred.

## Non-goals

- no third-party certification flow;
- no Honcho provider implementation (a third-party `mem0` provider now exists in
  `crates/ironclaw_memory_mem0`, but it is off by default and feature-gated behind
  `memory-mem0`; it binds to `memory.document_store.v1` and, in production-shaped
  deployments, requires an explicit admin override);
- no migration of legacy `memory_documents` rows from this crate.
