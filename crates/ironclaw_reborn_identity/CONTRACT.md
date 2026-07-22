# `ironclaw_reborn_identity` — Contract

The canonical Reborn identity layer. It maps every external identity — WebUI
OAuth logins (`google`, `github`, …) and external channel/product actors
(`telegram`, `slack`, triggers, …) — to a stable Reborn [`UserId`] **before**
any runtime state (conversation binding, thread ownership) is touched. Identity
provisioning lives here, not in WebUI ingress and not in `ironclaw_conversations`
(which consumes an already-resolved `UserId`).

This crate is **also the durable home of the minimal user profile** (email,
display name, timestamps), not only an identity→`UserId` map. Resolving an
identity persists a `StoredUser` record keyed by `UserId`, so "what do we know
about this user, and where is it stored" is answered *here* — there is no
separate users table elsewhere in the Reborn stack. Any future enumeration or
admin surface extends this store; it does not stand up a new one. See
[Persisted records](#persisted-records) for the exact shapes.

## Position in the stack

Bottom-of-stack, downstream-facing. Among internal `ironclaw_*` crates it
depends **only** on `ironclaw_host_api` (identity/scope newtypes, `ScopedPath`)
and `ironclaw_filesystem` (the durable substrate). It must never reach upstream
(`ironclaw_reborn_composition`, `ironclaw_product_workflow`) or onto the v1
legacy enclave. This is machine-enforced: `reborn_crate_dependency_boundaries_hold`
in `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` allows
exactly those two edges. The only external consumer is
`ironclaw_reborn_composition`, which re-exports a curated subset via its facade.

## Canonical key

An external identity is keyed by
`(tenant_id, surface_kind, provider_kind, provider_instance_id, external_subject_id)`.
Two tenants, two adapter installations, or two surfaces cannot collide on the
same subject id. Key parts cross the boundary as validated newtypes
(`ProviderKind`, `ProviderInstanceId`, `ExternalSubjectId`: non-empty, no control
chars) and are persisted **separately path-segmented** (each base64url-encoded
into its own path segment, never flattened) so a delimiter-like id cannot collide
with a key boundary. A `None` provider instance maps to the `_` sentinel — a
value no base64 encoding produces.

## Persisted records

The store persists three JSON record shapes under the
`/tenant-shared/reborn-identity` root. Shapes are defined in
`src/filesystem_store/record.rs`; path construction is in
`src/filesystem_store/paths.rs` (every opaque segment is base64url-encoded into
its own path segment — `surface` renders via its stable `as_str()`, and an empty
segment maps to the `_` sentinel).

| Record | Path (opaque segments base64url-encoded) | Fields |
|---|---|---|
| `StoredUser` — the canonical **user profile** | `…/users/{user_id}.json` | `email`, `display_name`, `created_at`, `updated_at` |
| `StoredExternalIdentity` — one bound external login | `…/external/{tenant}/{surface}/{provider}/{instance}/{subject}.json` | `user_id`, `email`, `email_verified`, `created_at` |
| `StoredVerifiedEmailIndex` — cross-provider link | `…/verified-email/{tenant}/{lower_email}.json` | `user_id` |

`StoredUser` is written by `resolve_or_create` on first contact (a returning
login upserts the profile); this is why a user's email and display name are
durably captured on SSO login without any separate directory. The record fields
are `pub(super)` — the on-disk JSON is an implementation detail, and upstream
consumers read through the resolver surface below rather than the raw records.
(Known gap: `adopt_migrated_identity` does **not** write `StoredUser` today —
tracked as #5616.)

## Resolver surface (`RebornIdentityResolver`)

- `resolve_or_create` — mint-capable. Resolves the identity, links by verified
  email, or creates a new user. **`SurfaceKind::ChannelActor` is rejected**
  (`ChannelActorNotMintable`): channel actors are never mint-capable and must
  fail closed, not auto-provision.
- `lookup` — link-only; returns the bound user or `None`, never creates.
- `bind` — links an external identity to an **already-existing** user (upsert,
  last-writer-wins). The caller must have authenticated the user first.
- `adopt_migrated_identity` — seeds a pre-existing identity carried from a legacy
  store, preserving its `user_id` and (for a verified email) the verified-email
  index. Never mints. Idempotent — existing identity/index records win.

## User directory surface (`RebornUserDirectory`)

A **separate** trait from `RebornIdentityResolver`, implemented by the same
`FilesystemRebornIdentityStore`, for the operator/admin surface that enumerates
and manages the `StoredUser` records. It is kept apart from the resolver so admin
CRUD cannot perturb the mint/link/create invariants above, and so the resolver's
contract tests are not entangled with admin methods. Its only production
consumer is `ironclaw_reborn_composition`, which adapts it up to the
product-workflow admin service (the port stays defined at the bottom of the
stack; the boundary tests still allow no new edge).

- `list_users` / `get_user` — enumerate (via `list_dir` over the non-partitioned
  `users/` directory) or fetch. `list_users` filters by the record's own
  `tenant_id`; a record with **no** persisted tenant is treated as belonging to
  the deployment's single configured tenant (only single-tenant deployments have
  such pre-admin records).
- `create_user` — creates an active user with **no external identity** and an
  immutable content-access policy. `Private` is the compatibility-safe default
  and uses the verified login/claim path; it never returns an administrator-
  minted credential. `TenantAdminManaged` creates a non-login, Member-only
  subject for administrator management. A private user's email reserves the
  same tenant-scoped claim index used by SSO, but only a provider-verified
  OAuth identity can bind to it; managed subjects write no login index.
- `update_profile` / `update_status` / `update_role` — partial mutations through
  the shared `ironclaw_filesystem::cas_update` helper (never a per-record mutex;
  `ironclaw_filesystem/CLAUDE.md` invariant 2). Each bumps `updated_at`.
- `record_last_login` — sets `last_login_at` only; deliberately does **not** bump
  `updated_at`, which tracks profile edits rather than login activity. Managed
  subjects reject this operation.
- `authorize_admin_managed_target` — the canonical administrator-on-behalf
  decision. It requires an active Admin/Owner actor, explicit same-tenant
  ownership for actor and subject, `TenantAdminManaged` target policy, and an
  explicitly modeled operation. Tenantless legacy records fail closed here.
- `delete_user` — **cascades** (see invariant 5 below).
- `count_active_admins` — supports last-admin protection in the facade.

A malformed persisted `user_id` / `created_by` / `tenant_id` surfaces
`InvalidUserId` / `Backend` on read-back (a backend inconsistency, never
silently dropped); a mutation of an absent user surfaces `UserNotFound` so the
facade can map it to a 404.

## Invariants (and where each is enforced)

1. **Verified-email linking is gated to OAuth + verified + non-empty.**
   `verified_email_key` (`filesystem_store.rs`) is the single source of truth:
   it returns `Some(lowercased email)` only on `SurfaceKind::Oauth` with
   `email_verified` and a non-empty address, feeding **both** `resolve_or_create`
   and `adopt_migrated_identity`. The surface gate is a **security** boundary: the
   verified-email index carries no surface dimension, so restricting linking to
   the allowlist-gated browser-SSO surface stops a channel actor that asserts a
   verified email from reading or overwriting an OAuth user's index (cross-surface
   account collapse). The empty-email guard stops `Some("")` from indexing on the
   `_` sentinel.
2. **Tenant scoping is by path**, not by the store's fixed host-caller
   `ResourceScope`. `tenant_id` is the first encoded path segment of every
   identity and verified-email record; the mount is `/tenant-shared`. Isolation
   rests on path construction.
3. **Index-before-identity write ordering** in `resolve_or_create`: the
   verified-email index is written (`CasExpectation::Absent`) before the identity
   record, so "a verified identity record exists" always implies "its index
   exists", and the read-only fast path never returns an identity whose index is
   missing. **This ordering guarantee is scoped to `resolve_or_create`** —
   `adopt_migrated_identity` writes identity-then-index (safe for its
   same-identity fast path; see the migration race note below).
4. **Channel actors never mint** — enforced at the top of `resolve_or_create`;
   `bind`/`adopt_migrated_identity` take an explicit authenticated `user_id`.
5. **`delete_user` cascades, and is the one sanctioned unwind of invariants 1/3.**
   Deleting a user removes, in order: every external-identity record in the
   tenant subtree bound to that `user_id` (walked iteratively over the
   fixed-depth `external/{tenant}/…` tree), then the user's verified-email index
   (keyed by the user's own stored email, deleted only if it points at them),
   then the `users/` record. Removing the external identities is **load-bearing
   for correctness**: leaving one would let a later re-login through that
   identity resolve the tombstoned id back to life via the read-only fast path.
   This is the only place the "a verified identity implies its index exists"
   ordering (invariant 3) is deliberately torn down — identity and index are
   removed together. Known limitation: only the index under the user's *stored*
   email is swept; an index under a different email is not (no reverse
   user→emails map). Acceptable for the current surface; revisit if multi-email
   accounts land.

## Concurrency model

Relational guarantees are reconstructed on the filesystem's compare-and-swap
primitive:

- A per-identity-key **process-local async lock** serializes concurrent
  first-contacts for one identity within a process. Serializing on the identity
  key (not the email) is deliberate: it also catches two first-logins for the
  same key presenting divergent verified emails, which an email-scoped lock would
  let run concurrently.
- `CasExpectation::Absent` on every create is the **cross-process** backstop: the
  per-key lock does not serialize across runtime replicas, so a racing creator
  gets `VersionMismatch` and reconciles by re-reading. The verified-email index
  CAS is the cross-process arbiter for cross-provider linking on a shared email.

### Accepted, bounded leak

On a **lost** cold first-contact race, `resolve_or_create` may leave an
unreferenced ("orphan") user row — never an orphan index. It occurs only on a
lost race (the returning-login fast path never mints), the record is tiny, and
there is no steady-state growth. Minting the user first is deliberate: writing it
last would, in the divergent-email cross-process race, leave a verified-email
index pointing at an id with no user record (a phantom) — strictly worse. GC of
unreferenced user rows is out of scope for this crate.

## Trust assumptions (load-bearing, external to this crate)

- **Upstream admission gate.** The security of verified-email linking rests on an
  upstream allowlist (the email-domain allowlist) that this crate cannot see.
  This crate **trusts** that any `SurfaceKind::Oauth` + `email_verified: true`
  identity handed to it was already admission-gated.
- **`RebornIdentityError` carries storage paths.** `Backend(_)` wraps
  `FilesystemError`, whose Display includes the `ScopedPath` (base64 of tenant /
  subject / email). It is below the channel boundary; consumers that surface it
  toward a user must map/scrub it per `.claude/rules/error-handling.md` (no paths
  in user-facing errors).

## Known gaps (tracked as issues, not yet closed)

Filed from the de-slop review:

- **#5614** — cross-process divergent-email logins can split a principal.
- **#5615** — `bind()` has no OAuth-surface guard (defense-in-depth).
- **#5616** — `adopt_migrated_identity` never writes `StoredUser` and reverses the
  index/identity write order.
- **#5617** — the login seam is tested only with fakes on both sides.
- **#5618** — decide the `ExternalIdentityKey` + `lookup`/`bind` public surface.
