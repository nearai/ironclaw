# ironclaw_projects guardrails

First-class **Project** entity, membership, and access control for the IronClaw
Reborn stack. Plan: `docs/plans/2026-06-17-reborn-projects.md`.

> Not to be confused with `ironclaw_engine`'s legacy `Project` type. This crate
> serves the Reborn stack (`ironclaw_product_workflow` → composition →
> `ironclaw_webui_v2`).

## W2 crate-count decision

Keep `ironclaw_projects` as a standalone substrate crate for W2. It owns the
durable project entity, live membership ACL, and repository contract over
`ScopedFilesystem`; folding that into composition would put domain persistence
behind the wiring layer. If this boundary is revisited later, the only plausible
consumer-side target is `ironclaw_product_workflow` (which owns the
`ProjectService` facade), and only if the project repository/domain contract can
move there without forcing lower substrate crates to depend upward.

## What this crate owns

- `ProjectRecord` — the durable project entity. `metadata: serde_json::Value` is
  an **extensible bag** (goals, GitHub links, …); add new soft fields there
  rather than new columns unless they need to be queried/indexed.
- `ProjectMemberRecord` + `ProjectRole` (`Owner > Editor > Viewer`) +
  `ProjectMemberStatus` — the ACL model.
- `ProjectRepository` — the persistence contract.
- `FilesystemProjectRepository` — the **sole** implementation, persisting over
  the Reborn `ScopedFilesystem` substrate. There is no SQL in this crate.

## Invariants

- **Identity is typed.** Use `ProjectId` / `TenantId` / `UserId` from
  `ironclaw_host_api`; never raw `String`. Enums are wire-stable
  (`#[serde(rename_all = "snake_case")]`) with `as_str` / `parse` helpers — do
  not `format!("{:?}", ...)` an enum onto the wire.
- **Authorization is live.** `resolve_access` is the read primitive; callers
  must call it per request and must not cache the result (revocation is
  immediate). The owner always resolves to `Owner`; otherwise the active
  grant wins; unknown project ⇒ `None`.
- **No silent failures.** Backend errors carry their cause
  (`ProjectError::backend("op", e)`); do not `map_err(|_| …)` away the source
  (see `.claude/rules/error-handling.md`).
- This crate persists data; it does **not** authorize callers, expose HTTP, or
  know about the facade. Authorization gating that combines `resolve_access`
  with a required role lives in the composition adapter (`RebornProjectService`),
  not here.

## Storage

`FilesystemProjectRepository` persists JSON records over a `ScopedFilesystem`
under a control-plane mount the agent cannot reach (the same substrate the
`ironclaw_reborn_identity` store rides). Backend selection — Postgres / libSQL /
JSONL / in-memory — is the host's `RootFilesystem` concern, so this crate is
backend-agnostic and carries no SQL or `libsql`/`postgres` features.

Layout (opaque key parts base64url-encoded per segment):

```text
/tenant-shared/reborn-projects/<tenant>/records/<project_id>.json
/tenant-shared/reborn-projects/<tenant>/members/<project_id>/<user_id>.json
```

Tenant isolation is twofold: a per-call `ResourceScope` carries the tenant
(so a real mount resolver maps to a per-tenant virtual path) **and** the tenant
is a path segment (so isolation also holds under a fixed-view resolver, as in
tests). Concurrency uses the substrate's compare-and-swap: create uses
`CasExpectation::Absent` (conflict ⇒ `AlreadyExists`); delete is keyed off the
record's presence so a losing racer observes `None`. `created_at` is immutable
across updates.

Why not the agent workspace VFS or raw SQL: the ACL is authorization data the
agent must not be able to write, so it lives on the control-plane substrate, not
the `/workspace` mount; and routing through `ScopedFilesystem` (rather than raw
`deadpool_postgres`/`libsql` handles) keeps one backend-dispatch seam for every
durable Reborn store.

## Tests

`tests/repository_contract.rs` runs the full contract against
`FilesystemProjectRepository` over an in-memory `RootFilesystem`. Backend
correctness (Postgres / libSQL / JSONL) is `ironclaw_filesystem`'s concern, so a
single in-memory run covers all repository logic.

```bash
cargo test  -p ironclaw_projects
cargo clippy -p ironclaw_projects --tests
```
