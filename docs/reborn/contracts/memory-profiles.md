# Reborn Memory Capability Profiles

**Status:** Draft zero-behavior contract
**Issue:** #3537

Memory profile contracts are host-defined portability targets. Extensions may claim they implement these profiles, but claims do not grant trust or authority by themselves. Runtime binding, certification, and dispatch are later work.

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

## Deferred

- `memory.semantic_search.v1` — issue #3537 lists this profile, but it depends on a host-mediated embedding/vector port that does not exist yet. It is intentionally omitted from this zero-behavior prep slice; it must be added before semantic search ships, either as its own profile with a host-mediated embedding port or kept behind a separate optional feature/fallback.

## Non-goals

- no production manifest v2 parser changes;
- no profile binding configuration;
- no native memory implementation;
- no runtime dispatch changes;
- no third-party certification flow.
