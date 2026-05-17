# ScopedFilesystem-centric Tenant Isolation

**Date:** 2026-05-16
**Owner:** TBD
**Status:** consumer-store migration shipped in PR #3679; per-invocation
composition wiring (HostRuntimeServices factory) is the remaining
follow-up — see "Open Question 1".

## Why

Two findings on PR #3679 (universal-FS dispatch) surfaced a systemic
issue: the migrated consumer stores (`ironclaw_processes`,
`ironclaw_secrets`, `ironclaw_outbound`, `ironclaw_authorization`,
`ironclaw_engine`) all take a raw `Arc<F: RootFilesystem>` at
construction. They bypass the existing `ScopedFilesystem` /
`MountView` permissions layer, so:

- Tenant isolation is either manually threaded into paths (processes:
  `/engine/tenants/{tenant_id}/users/{user_id}/...`) or absent
  (engine: `/engine/threads/<thread_id>.json`).
- The `MountPermissions { read, write, list, delete, execute }` ACL
  exists but is unused — no consumer differentiates read-only from
  write-allowed mounts.
- Shared system data (capability defs, system prompts) has no
  canonical "read-only for everyone, write for root" place.

Two reviewer findings on commit `4eccad56d` (`serrrfirat`) made this
concrete:

1. **HIGH**: `ironclaw_engine::FilesystemStore` is not tenant-scoped.
   Path layout omits tenant; engine record types omit `tenant_id`;
   composition builds a single shared root. Two tenants with the
   same `user_id`/`project_id` collide on `/engine/projects/<id>` etc.
2. **HIGH regression**: byte-only `LocalFilesystem` backends fail
   under filesystem-backed store contracts because the stores write
   `CasExpectation::Version` and record-shaped entries that the
   byte-only backend rejects — already addressed via per-store
   `put_with_byte_fallback` helpers (commit `199137b57`), but the
   underlying ScopedFilesystem layer would have caught this once
   uniformly via `BackendCapabilities` declaration at mount time.

## The Design

### Filesystem-layer enforcement (already exists)

`ironclaw_filesystem` already implements a Linux-like permissions
model:

```text
MountView                                        # per-invocation
  └── Vec<MountGrant>
        ├── MountAlias (consumer-visible — "/engine")
        ├── VirtualPath (storage target — "/tenants/X/users/Y/engine")
        └── MountPermissions { read, write, list, delete, execute }

ScopedFilesystem<F>                              # wraps RootFilesystem + MountView
  ├── put / get / query / append / tail / delete / list_dir / stat / begin
  ├── resolve_with_permission() — ACL check, alias→VirtualPath rewrite
  └── ScopedStorageTxn — carries ACL across txn boundaries
```

### Migration shape

**1. Consumer stores accept `Arc<ScopedFilesystem<F>>`, not `Arc<F>`.**

Each migrated consumer crate's `FilesystemXxxStore::new` changes:

```rust
// Before
pub fn new(filesystem: Arc<F>) -> Self;

// After
pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self;
```

**2. Path helpers return `ScopedPath`, not `VirtualPath`.**

Path strings stay the same (`/engine/threads/<id>.json`). They are
alias-relative under the consumer's canonical mount alias. The
MountView wired by composition resolves the alias to the
tenant/user-scoped VirtualPath.

**3. Composition wires per-invocation `MountView`.**

A single helper builds the canonical MountView per `ResourceScope`:

```rust
pub fn invocation_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    MountView::new(vec![
        // Per-user, per-tenant private state — full r/w/l/d
        MountGrant::new(
            MountAlias::new("/engine")?,
            VirtualPath::new(&format!(
                "/tenants/{}/users/{}/engine",
                scope.tenant_id, scope.user_id
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/secrets")?,
            VirtualPath::new(&format!(
                "/tenants/{}/users/{}/secrets",
                scope.tenant_id, scope.user_id
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/processes")?,
            VirtualPath::new(&format!(
                "/tenants/{}/users/{}/processes",
                scope.tenant_id, scope.user_id
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/outbound")?,
            VirtualPath::new(&format!(
                "/tenants/{}/users/{}/outbound",
                scope.tenant_id, scope.user_id
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/authorization")?,
            VirtualPath::new(&format!(
                "/tenants/{}/users/{}/authorization",
                scope.tenant_id, scope.user_id
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        // Tenant-shared (between users/agents in same tenant)
        MountGrant::new(
            MountAlias::new("/tenant-shared")?,
            VirtualPath::new(&format!("/tenants/{}/shared", scope.tenant_id))?,
            MountPermissions::read_write_list(),
        ),
        // System-globally readable (capability defs, system prompts)
        MountGrant::new(
            MountAlias::new("/system")?,
            VirtualPath::new("/system")?,
            MountPermissions::read_only(),
        ),
    ])
}
```

**Net effect:** consumer code is **tenant-agnostic**. It uses paths
like `/engine/threads/<id>.json`. The MountView (built once per
invocation in composition) handles tenant prefixing and ACL.

### What this gives us

- **One place to mess up:** the composition `invocation_mount_view`
  helper. If it's wrong, every consumer's data is misrouted —
  but it's wrong _consistently_, which a single cross-tenant
  isolation test catches.
- **Cross-tenant isolation by construction:** two `ScopedFilesystem`
  instances over the same `RootFilesystem` cannot see each other's
  data because their MountViews resolve to disjoint VirtualPath
  prefixes.
- **Defense in depth:** every consumer's `put` carries a `tenant_id`
  indexed projection (alongside the path-prefix scope) so an
  admin-tier query can filter and a path-rewriting bug surfaces as
  a query-time mismatch. **Scope:** this projection applies to
  `ironclaw_processes`, `ironclaw_secrets`, `ironclaw_outbound`,
  and `ironclaw_authorization`. `ironclaw_engine` is deliberately
  excluded — its `Store` trait is single-tenant-by-construction
  (per Open Question 2 below; the engine never sees `tenant_id`
  internally), so it relies on path-prefix scoping alone and has
  no `ResourceScope` available at write sites to drive the
  projection.
- **Read-only carve-out:** the `/system` mount has `read_only` perms.
  Engine and other consumers reading capability definitions or
  system prompts go through the same `ScopedFilesystem`; writes are
  rejected at the ACL layer.
- **Eliminates duplicated tenant-prefixing logic:** `ironclaw_processes`
  currently has manual `/engine/tenants/{tenant_id}/...` formatting
  in 30+ path builders; that goes away.

## Migration Status

| Crate | Status | PR |
|---|---|---|
| `ironclaw_engine` | **Done** — FilesystemStore takes `Arc<ScopedFilesystem<F>>`; paths return `ScopedPath`; cross-tenant isolation regression test | #3679 (ac8e677f9) |
| `ironclaw_processes` | **Done** — drops manual `/engine/tenants/.../users/...` prefixing; `ScopedFilesystem` does the rewriting | #3679 (81664dd29) |
| `ironclaw_secrets` | **Done** — drops manual `/secrets/tenants/.../users/...` prefixing; AAD owner-scope aligned with path | #3679 (4ae56769b) |
| `ironclaw_outbound` | **Done** — paths alias-relative under `/outbound`; tenant_id moves to MountView | #3679 (6ecca195d) |
| `ironclaw_authorization` | **Done** — drops manual tenant prefix; CAS-Version retry + Unsupported→Any fallback preserved | #3679 (5e7688d3b) |
| `ironclaw_reborn_composition` | **Done** — wires `default_singleton_mount_view()` for long-lived composition; `invocation_mount_view(scope)` helper available for per-invocation construction | #3679 (c60ff0af5) |
| `MountPermissions::read_write_list_delete()` helper | **Done** — `ironclaw_host_api::mount` | #3679 (0a51286d1) |
| `/tenant-shared` and `/system` mount aliases | **Done** — wired in both `default_singleton_mount_view` and `invocation_mount_view`; `read_only()` permissions on `/system` | #3679 |

## Legacy per-backend store cleanup

The universal-FS dispatch design says backend choice should be at the
`RootFilesystem` layer, not at the consumer-store layer. The legacy
per-backend `Filesystem*Store` siblings (`LibSql*Store`,
`Postgres*Store`) are vestigial and should be deleted.

| Crate | Legacy stores | Status |
|---|---|---|
| `ironclaw_outbound` | `LibSqlOutboundStateStore`, `PostgresOutboundStateStore` | **Deleted** in `d4bf7b3c2` — no production callers, contract tests removed because durability across reopen is now a `RootFilesystem` property. |
| `ironclaw_secrets` | `LibSqlSecretsStore`, `PostgresSecretsStore` | **Deleted** in `f0abb79d6` — composition rewired to `FilesystemSecretStore` and the master-key decryptability check ported to `FilesystemSecretStore::verify_can_decrypt_existing_secrets`. |
| `ironclaw_authorization` | `LibSqlCapabilityLeaseStore`, `PostgresCapabilityLeaseStore` | **Deleted** in `251f6fa5d` — composition routes leases through `Arc<FilesystemCapabilityLeaseStore<F>>`. |
| `ironclaw_run_state` | `LibSqlRunStateStore`, `PostgresRunStateStore`, `LibSqlApprovalRequestStore`, `PostgresApprovalRequestStore` | **Deleted** in `a238050a2` — run-state is now FS-scoped via `FilesystemRunStateStore` / `FilesystemApprovalRequestStore`. |
| `ironclaw_memory` | `LibSqlMemoryDocumentRepository`, `PostgresMemoryDocumentRepository` | **Deleted** in `440be242c` — no production callers (composition already used `FilesystemMemoryDocumentRepository`). Substrate semantics (versioning, chunk replace, metadata cascade, hybrid search fusion) keep coverage via `FilesystemMemoryDocumentRepository` over `InMemoryBackend`; backend-specific durability moves to `ironclaw_filesystem`'s own backend contract tests. The `libsql` / `postgres` features and `deadpool-postgres` / `libsql` / `pgvector` / `tokio-postgres` deps drop off `ironclaw_memory/Cargo.toml`. |
| `ironclaw_reborn_event_store` | `LibSqlDurableEventLog`, `LibSqlDurableAuditLog`, `PostgresDurableEventLog`, `PostgresDurableAuditLog` | **Deleted** — `RebornEventStoreConfig::Libsql{...}` / `::Postgres{...}` variants now build a `LibSqlRootFilesystem` / `PostgresRootFilesystem` and route the durable log through `FilesystemDurableEventLog` / `FilesystemDurableAuditLog`. SQL-table corruption fixtures dropped; durable-log contract coverage shifts to the backend-agnostic `FilesystemDurableEventLog` contract suite plus the public-surface `RebornEventStoreConfig` rebuild tests. |

The remaining three cleanups are each independent and each requires
~200-400 lines (migration + composition rewiring + deleting the SQL
impl + updating tests that exercise the SQL store directly). They are
best tackled as focused follow-up PRs rather than piling onto #3679.

## Trait collapse (further follow-up)

After the legacy stores are gone, each consumer crate would have
exactly one `XxxStore` impl (`Filesystem*Store`). At that point the
`XxxStore` trait is purely a type-erasure wrapper used by
`HostRuntimeServices` to hold `Arc<dyn XxxStore>` without taking on
additional generic parameters. Collapsing the trait means:

- Renaming `FilesystemXxxStore<F>` → `XxxStore<F>` (concrete struct).
- Deleting the trait.
- Threading `F` (or each store's concrete type) through
  `HostRuntimeServices` as additional generic parameters, OR using
  `Arc<dyn Any>` style erasure that doesn't require the trait.

This is genuinely the design endpoint but it widens type signatures
across `host_runtime`, `capabilities`, `obligations`, and `production`
significantly. It's a deeper structural change than the legacy
deletion above and deserves its own focused PR.

## Composition entry points

Two public helpers in `ironclaw_reborn_composition`:

- `default_singleton_mount_view()` — the long-lived single-tenant
  default. Every consumer alias resolves to a top-level VirtualPath
  root (`/processes` → `/processes`). Production composition uses this
  today. Cross-tenant isolation in single-tenant deployments comes
  from there being only one tenant; in multi-tenant deployments it
  comes from constructing a per-invocation view (next).
- `invocation_mount_view(scope: &ResourceScope)` (pub) — rewrites
  every per-user alias to `/tenants/<tenant>/users/<user>/<alias>`.
  Used by per-request handlers that build tenant-scoped consumer
  stores via `wrap_scoped_for_invocation`. Tests cover the rewriting
  contract and prove two scopes with the same `user_id` produce
  disjoint target paths.

## Tests required

Each migrated crate adds one regression test of the shape:

```rust
#[tokio::test]
async fn store_isolates_two_tenants_with_same_user_project_ids() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped_a = Arc::new(ScopedFilesystem::new(
        backend.clone(),
        // tenant "a" — same user "u1" / project "p1"
        invocation_mount_view(&scope_with(tenant_id_a(), "u1", "p1"))?,
    ));
    let scoped_b = Arc::new(ScopedFilesystem::new(
        backend.clone(),
        // tenant "b" — same user/project
        invocation_mount_view(&scope_with(tenant_id_b(), "u1", "p1"))?,
    ));

    let store_a = FilesystemStore::new(scoped_a);
    let store_b = FilesystemStore::new(scoped_b);

    store_a.save_thread(&thread_with_id(thread_id())).await?;
    assert!(store_b.load_thread(thread_id()).await?.is_none());
}
```

## Open Questions

1. **Per-tenant `FilesystemStore` lifetime.** Engine `ThreadManager`
   currently holds one `FilesystemStore`. With ScopedFilesystem,
   each invocation has a different MountView. Two options:
   - Per-tenant long-lived: `HashMap<TenantId, Arc<ThreadManager>>`
     in the host, one ThreadManager per tenant.
   - Per-invocation: rebuild ThreadManager per request.
   The composition layer's choice. Per-tenant long-lived is cheaper
   and matches how single-tenant deployments work today.

2. **Engine `Store` trait does not carry tenant.** Methods take
   `user_id: &str` / `project_id: ProjectId` but no `tenant_id`. The
   trait is single-tenant-by-construction. Multi-tenancy lives at
   the wiring layer (one Store impl per tenant). This means the
   engine internally never sees `tenant_id` — which is the goal
   ("minimize the places we need to carry tenant around").

3. **Audit / observability cross-tenant queries.** When operators
   want to see "all jobs across all tenants", they need a path that
   bypasses ScopedFilesystem (or constructs an admin MountView with
   `/tenants/*/...` access). Out of scope for this design; tracked
   as a separate operator-tooling concern.

## References

- ADR: `docs/reborn/2026-05-14-universal-fs-dispatch.md` —
  "Per-tenant routing is a mount table choice, not a code change."
- `ironclaw_filesystem` CLAUDE.md invariant 7 — "Multi-tenant
  deployments rely on the path prefix to route to per-tenant mounts."
- PR #3679 review comments by `serrrfirat` (2026-05-16) — original
  finding.
