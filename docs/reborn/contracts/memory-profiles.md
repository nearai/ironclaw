# Reborn Memory Capability Profiles

**Status:** Active — catalog, host ports, native manifest, and binding landed;
SQL storage-port-backed persistence + default flip gated (see "Implemented" /
"Deferred")
**Issue:** #3537

Memory profile contracts are host-defined portability targets. Extensions may claim they implement these profiles, but claims do not grant trust or authority by themselves. Certification of third-party providers and SQL-backed native storage remain later work.

## Profiles

| Profile | Required operation | Visibility | Required host ports |
| --- | --- | --- | --- |
| `memory.context_retrieval.v1` | `memory.context.retrieve.v1` | `host_internal` | `host.storage.sql_transaction.first_party`, `host.events.audit` |
| `memory.interaction_log.v1` | `memory.interaction.record.v1` | `host_internal` | `host.storage.sql_transaction.first_party`, `host.events.audit` |
| `memory.document_store.v1` | `memory.document.read.v1`, `memory.document.write.v1` | `api` / model-facing tools layered separately | `host.storage.sql_transaction.first_party`, `host.events.audit` |

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
- **Native v2 manifest**: `ironclaw.memory.native` (HostBundled, `first_party`
  runtime) declares the four provider-prefixed capabilities with `implements`
  mappings, schema refs, `host_internal` visibility, and the required host ports;
  its `service` must match the host-registered native provider identity.
  Conformance tests prove the native capabilities satisfy every profile.
- **Profile binding**: the `[memory]` config section (`profile_bindings` +
  `admin_overrides`) resolves through a fail-closed
  `MemoryBindingPolicy` (`profile_id -> extension_id`, default-native; production
  rejects `memory.disabled` and unverified third-party bindings absent an
  `(extension_id, profile_id, deployment_profile)` override). The memory-tools
  dispatch site consults the binding instead of hardwiring the native provider;
  startup lists active overrides (redacted).

## Deferred

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
- no Honcho/mem0 provider implementation;
- no migration of legacy `memory_documents` rows from this crate.
