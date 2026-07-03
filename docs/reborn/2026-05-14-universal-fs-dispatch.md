# 2026-05-14 — Universal Filesystem Dispatch Fabric

**Status:** Accepted. Supersedes
`docs/reborn/2026-04-25-storage-catalog-and-placement.md`.
**Drives:** The kernel storage rework plan (promoted into
`docs/plans/2026-05-14-kernel-storage-rework.md` when the foundation
PR lands).

## Context

Before this ADR every persistence concern in the IronClaw workspace owned
its own `Store`/`Repository` trait with a per-backend dispatch for libSQL
and PostgreSQL:

- `ironclaw_secrets`, `ironclaw_authorization`, `ironclaw_memory`,
  `ironclaw_processes`, `ironclaw_run_state`, `ironclaw_outbound`,
  `ironclaw_conversations`, `ironclaw_reborn_event_store`,
  `ironclaw_engine` (`Store` trait), plus `src/db/` (composite `Database`
  trait, 7 sub-traits, ~78 methods), `src/secrets/`, `src/workspace/`,
  `src/history/`.

Adding a new backend (HSM, TEE-resident KMS, S3 object store) meant
touching 8–12 crates. Each crate also duplicated feature-flag
dispatch, dialect-difference handling, and scope-key plumbing. The
storage-catalog ADR of 2026-04-25 mitigated this by drawing a "bytes
mount; typed stays typed" placement boundary — explicitly *not*
unifying the dispatch.

The user's direction for the kernel storage rework asks for the
opposite: collapse everything onto a single mount fabric so the kernel
stays small and every backend is interchangeable behind one trait.

## Decision

There is **one universal filesystem dispatch fabric**:

1. **One trait:** `RootFilesystem` (in `crates/ironclaw_filesystem/`).
   Every backend (local file, libSQL, PostgreSQL, HSM, in-memory,
   encrypted-decorator, object-store, …) implements it. The composite
   dispatcher (`CompositeRootFilesystem`) also implements it; it *is*
   a backend that routes by longest-prefix mount. No parallel "backend
   trait" sits next to `RootFilesystem`.
2. **One entry type:** `Entry { body, content_type, kind, indexed }`.
   A bytes-only "file" is an `Entry` with `kind = None` and an empty
   indexed projection. A "record" is an `Entry` with `kind = Some(_)`
   and a populated indexed projection. The same `put`/`get`/`query`/CAS
   machinery serves both.
3. **One set of ops:** `put` / `get` / `delete` / `list_dir` / `query`
   / `ensure_index` / `stat` / `begin` / `append` / `tail`. CAS +
   versioning is universal — every successful `put` returns a
   `RecordVersion` and every successive write can require a CAS match.
4. **Capabilities declared at mount time.** `BackendCapabilities`
   enumerates what each backend serves
   (`{ read, write, append, list, stat, delete, records, query, index,
   txn, events }`). Mount-time validation refuses backends that cannot
   satisfy the indexes a consumer declares; runtime `Unsupported`
   errors are the fallback when a capability is conditional on the
   underlying server.
5. **Encryption-at-rest is a backend decorator.** `EncryptedBackend`
   (planned) wraps an inner backend, encrypting `Entry::body` and any
   `IndexValue::Bytes` projection while letting scalar indexed
   projections (`scope`, `status`, …) pass through so query still
   works. `SecretStore` and other sensitive-data stores never own
   encryption code — they write plaintext `Entry` values through a
   `ScopedFilesystem` whose mount happens to be wrapped in encryption.

## How this overrides the 2026-04-25 catalog ADR

The 2026-04-25 ADR drew a hard line:

> "Secrets/leases/processes/events stay typed, no mount. If callers think
> in paths & bytes → mount. If they need leases, transactions, queries,
> redaction → typed repository."

That boundary was correct only because the filesystem trait at that time
was bytes-only and couldn't carry leases, transactions, queries, or
redaction-friendly typed data. **This ADR widens the filesystem trait so
those features ARE first-class on the mount fabric**:

| 2026-04-25 reason for "typed, no mount"                | 2026-05-14 mechanism that absorbs it onto the mount fabric                                                                |
|--------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------|
| Leases need atomic claim/consume                       | `put` with `CasExpectation::Version(v)` + retry on `VersionMismatch`. `begin`/`StorageTxn` for backends that have it natively. |
| Transactions for multi-step state changes              | Same — CAS is the floor; multi-key transactions are an opt-in capability.                                                 |
| Query/index needs (search by scope, status, time)      | `query(prefix, filter, page)` + `ensure_index(prefix, IndexSpec)`. Indexed projection is queryable; payload is opaque.    |
| Redaction (never expose ciphertext or scope keys)      | `EncryptedBackend` decorator handles ciphertext transparently; redaction stays in the consumer's domain error types.       |
| Event logs need append/tail                            | `append(path, payload) -> SeqNo` and `tail(path, from)` on the same trait. Narrow, opt-in; only event-log mounts wire it. |

Result: structured/control-plane records *can* go through the filesystem
when (a) the backend declares the matching capabilities and (b) the
consumer chooses to.

## Consequences

**Wins:**

- Adding a new backend means implementing one trait. HSM-backed secrets,
  S3-backed artifacts, and TEE-backed KMS all become single-mount
  swaps.
- Per-tenant routing is a mount table choice, not a code change.
- CAS + versioning is available to memory documents and project files
  for free.
- The audit / observability story is uniform — every write goes through
  one method.
- Feature-flag dispatch (`#[cfg(feature = "libsql")]` /
  `#[cfg(feature = "postgres")]`) concentrates in
  `crates/ironclaw_filesystem/`; consumer crates lose their per-backend
  branches.

**Costs:**

- The composite/router becomes a load-bearing piece of infrastructure.
  Bugs in `CompositeRootFilesystem::matching_mount` have system-wide
  blast radius.
- Existing consumer crates (10+ of them) need to migrate from their
  own `Store` trait to taking a `ScopedFilesystem`. This is intrusive,
  but the plan sequences it crate-by-crate.
- Backends that don't natively support records (local file, S3) need to
  decide whether to store records as JSON files in a sidecar or to
  reject records explicitly via declared capabilities. We default to
  declaring `records: false` and rejecting at mount time.

**Mitigations:**

- Legacy bytes ops (`read_file`/`write_file`/`append_file`/`list_dir`)
  remain on `RootFilesystem` during the migration window with default
  impls that route through the new `put`/`get`. Existing consumer code
  keeps compiling.
- `InMemoryBackend` ships with the foundation as a reference impl that
  serves the full unified surface. It replaces the N per-crate
  `InMemoryStore` implementations.
- The crate's `CLAUDE.md` was rewritten alongside this ADR to encode the
  invariants new code must preserve.

## Out of scope for this ADR

- The engine v2 sandbox `MountBackend` (per-project Docker filesystem
  isolation) converges with `RootFilesystem` in a follow-up. Different
  blast radius — sandbox is about isolation, not backend selection.
- Cross-mount transactions. Single-mount CAS is the floor; cross-mount
  atomicity is explicitly rejected.
- The real HSM/TEE/S3 backend implementations. The trait seam exists;
  the implementations land in follow-up PRs.
- Migration of existing libSQL/Postgres rows into new record-shaped
  tables. A separate data-migration plan handles that once the trait
  lands and consumers begin to migrate.

## References

- Rework plan: `docs/plans/2026-05-14-kernel-storage-rework.md` (to be
  promoted from the working plan file when the foundation PR opens).
- Superseded ADR:
  `docs/reborn/2026-04-25-storage-catalog-and-placement.md`
- Architecture sprawl rule: `.claude/rules/architecture.md` — "duplicate
  dispatch pipelines" is the smell this rework eliminates at the
  storage layer.
- Filesystem crate guardrails: `crates/ironclaw_filesystem/CLAUDE.md`.
