# `ironclaw_reborn_identity` — Contract

The canonical Reborn identity layer. It maps every external identity — WebUI
OAuth logins (`google`, `github`, …) and external channel/product actors
(`telegram`, `slack`, triggers, …) — to a stable Reborn [`UserId`] **before**
any runtime state (conversation binding, thread ownership) is touched. Identity
provisioning lives here, not in WebUI ingress and not in `ironclaw_conversations`
(which consumes an already-resolved `UserId`).

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
