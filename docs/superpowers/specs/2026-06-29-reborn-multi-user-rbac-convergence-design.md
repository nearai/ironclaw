# Reborn multi-user RBAC — convergence design (review of #5385)

**Status:** proposal for review — high-level direction, not a line-level redesign.
**Date:** 2026-06-29
**Relates to:** PR #5385 (multi-user capability policy)

## Guiding principle

> This is the idea. **Avoid adding another layer unless we have to.**

Every noun in the product brief — *tenant, user, admin, owner, project* — already
maps to a concept that exists in the codebase. The goal of this design is to **converge
onto what we have and delete duplicates**, not to introduce a new authorization layer.
Each new scope, role enum, or store multiplies the permutations a reader (and the
compiler) must reason about; we want fewer moving pieces, not more.

## Problem: #5385 introduces a third parallel auth model

The product wants a multi-tenant instance where admins configure shared tools/skills
that all users inherit, users authenticate individually to those shared tools, and
users can add private tools on top. PR #5385 is a working **single-tenant capability
gate**, but as the foundation for that product it is misaligned on three counts, and
the root cause is that it stands up a **third** user/role/store system next to two that
already exist.

| | role enum | store | tenant-aware? | live? |
|---|---|---|---|---|
| v1 gateway | `src/ownership::UserRole {Owner,Admin,Regular}` | SQL `users` table | no | yes |
| **#5385** | `product_workflow::UserRole {Member,Admin,Owner}` | `/authorization/user-directory.json` (`ResourceScope::system()`) | carried but **unused** | yes |
| projects | `ironclaw_projects::ProjectRole {Owner,Editor,Viewer}` | `ProjectMemberRecord` rows | yes | built, **not hot-path-wired** |

Evidence:
- `crates/ironclaw_reborn_composition/src/capability_policy.rs:45` — directory path is
  `/authorization/user-directory.json`, stored at `ResourceScope::system()` (line ~78),
  i.e. deployment-global with **no tenant key**.
- `crates/ironclaw_product_workflow/src/reborn_services.rs` (`capability_policy_caller_role`
  and all `admin_*` methods) — `caller.tenant_id` is **never read**; authority is decided
  on `user_id` alone. Single-tenant today; a latent cross-tenant admin leak the instant a
  second tenant shares a process.
- `src/ownership/mod.rs` (`UserRole {Owner,Admin,Regular}`, `UserId {id, role}`) and the
  `/api/admin/users` gateway surface (`src/channels/web/...`, `crates/ironclaw_gateway/static/js/surfaces/users.js`)
  — a fully live, separate user/admin system with no `TenantId`.
- `crates/ironclaw_projects/src/lib.rs` — `ProjectRecord`, `ProjectMemberRecord`,
  `ProjectRole {Owner,Editor,Viewer}`, `ProjectRepository::resolve_access`; REST routes
  mounted in `crates/ironclaw_webui_v2/src/router.rs`. But `resolve_access` has **zero
  callers on the turn-execution path** — a Viewer can still submit turns.

There is no bridge between these. `"member"` ↔ `"regular"` mis-round-trips between the
two Rust enums. This is the `types.md` "same shape, different type, the compiler can't
enforce agreement" anti-pattern — at the *subsystem* level.

## Target model: map each product noun to one canonical home

| Product noun | Canonical home (already exists) | Action |
|---|---|---|
| **tenant** | `TenantId` (host_api), threaded through `ResourceScope`, credentials, projects | keep; nothing to build |
| **user** | `ironclaw_host_api::UserId` — opaque identity, **no role embedded** | keep; never encode role/provenance into the id string |
| **user ↔ tenant role** | one `TenantRole` in the canonical identity layer | **converge** the 2–3 existing enums into this one |
| **admin / owner** | `TenantRole::Admin` + a protected bootstrap `Owner` | one enum, one owner-protection guard |
| **project + sharing** | `ProjectRecord` + `ProjectMemberRecord` + `ProjectRole` | keep; **wire onto the hot path** |
| **shared tools for all** | a tenant-level **capability default** | the single new thing — and it fits the resolver #5385 already added |
| **private tools** | per-user installs (already user-scoped) | keep |
| **per-user auth to a shared tool** | extension install is tenant/deployment-global; credential scope is per-user | **already solved** — this answers the brief's open question |

The canonical scope set is already `ResourceScope { tenant, user, agent, project,
mission, thread, invocation }`. **This design adds no new scope.** Capabilities attach
to *tenant* (the shared default) and *user* (private installs) — deliberately **not** to
a new "capability-policy scope" and **not** to *project* (per-project toolsets are
net-new and unrequested).

## Authority = two orthogonal axes, never multiplied

There are exactly two authority questions. Keeping them separate means a user has *one*
tenant role plus *N* per-project roles — the axes add, they don't cross-multiply.

1. **Tenant administration — "can this user run the company?"** → `TenantRole`.
   `is_admin()` ⇒ provision users and configure the shared toolbox. `Owner` is simply
   the env-bootstrapped admin that REST cannot delete; model that as **one immutability
   guard**, not the per-method delete-rank matrix #5385 spreads across
   `admin_delete_user` / `set_role` / `set_capability`.

2. **Project access — "what can this user do in *this* project?"** →
   `ProjectRepository::resolve_access(tenant, project, user) -> ProjectRole`. This
   already exists and is correct; it is just never consulted during a turn. Gate
   `submit_turn` on at least `Editor`.

## Capability surface = tenant_default ⊕ user_private

"Admin configures once, everyone inherits" is a **tenant default**, not a per-user grant
map. The resolver #5385 added (`CapabilitySurfaceProfileResolver`) is the right seam, and
it **already receives** `tenant_id`, `user_id`, and `project_id`
(`crates/ironclaw_loop_support/src/capability_allow_set.rs`). Collapse the logic to:

```text
resolve(ctx):
    if tenant_role(ctx.tenant, ctx.user).is_admin(): return All
    return tenant_capability_defaults(ctx.tenant)  ∪  user_private_installs(ctx.user)
```

This **deletes**: the per-user `grants: BTreeMap`, `CapabilityAvailability::{Available,
Hidden}`, `admin_set_user_capability`, and the hardcoded `ESSENTIAL_MEMBER_CAPABILITIES`
constant (it becomes the *seed* of the tenant default, which admins can edit). The
product never asks for per-user hide/grant — only shared-default plus private additions —
so the entire per-user delta machinery is complexity the brief does not justify.

## Per-user auth to shared tools — already solved

The brief's open question ("how does per-user auth bind to an admin-configured tool?")
is already answered by existing architecture: extension installation is
tenant/deployment-global (`/system/extensions/.installations/state.json`, no per-user
field), while credential scope is keyed per user
(`crates/ironclaw_product_workflow/src/reborn_services/extension_credentials.rs`,
`AuthProductScope` keeps `user_id`). Admin installs Slack once; each user connects their
own account. No new mechanism required.

## What to delete / converge (the code-judo list)

- **Delete** the `UserDirectory` JSON-at-`system()` store; replace with a
  `(tenant_id, user_id)`-keyed tenant-membership store (reshape an existing concept, **not**
  a new scope).
- **Delete** per-user capability grants and `admin_set_user_capability` (see above).
- **Converge** `product_workflow::UserRole` and `src/ownership::UserRole` into one
  `TenantRole` in the canonical identity crate — one enum, one wire serialization.
- **Delete or fully implement** `delete_user`: today it removes only the directory row,
  orphaning the user's threads/memory/credentials, and it is not in the product brief.
  Either make it a real cascade or drop it. (Note the `LLM data is never deleted`
  invariant before choosing cascade.)
- **Stop encoding meaning into `UserId`** (the `sso-` prefix; the by-comment `:` ban).
  Keep the email→id derivation, but store provenance as a membership field; the id stays
  opaque.
- **Make tenant authority real**: consult `caller.tenant_id` in every admin guard.

## What's worth keeping from #5385

- The `CapabilitySurfaceProfileResolver` seam — correct abstraction, already scope-aware.
- Per-user credential scoping (each user does their own OAuth) — aligns with the brief.
- The instinct that members shouldn't see everything — realized here as a tenant default
  rather than per-user denies.

## Suggested sequencing (high level)

1. Converge to one `TenantRole` + a tenant-scoped membership store (removes the third system).
2. Make tenant authority tenant-aware (consult `tenant_id`).
3. Add the tenant-default capability set behind the existing resolver; delete per-user grants.
4. Wire `resolve_access` onto `submit_turn` + production composition for projects.
5. *(Larger, separate design)* project-scoped shared filesystem.

Steps 1–3 are mostly deletion and reshaping; step 4 is a few edits onto code that already
exists; step 5 is the only genuinely large net-new piece and warrants its own design.

## Open decisions for reviewers

1. **Owner vs Admin tiers.** Keep two tenant-admin tiers (`Owner` protected + `Admin`),
   or collapse to a single `Admin` tier with the env identity as the immutable bootstrap?
   Leaning to the latter unless there is a real "an admin who cannot remove other admins"
   requirement — fewer tiers, fewer permutations.
2. **v1 deprecation.** Is the SQL/gateway user model (`src/ownership` + `/api/admin/users`)
   being retired in favor of the Reborn identity layer? The convergence target depends on
   which store is canonical going forward.

## Non-goals

- A full line-level redesign or implementation plan — this is direction for review.
- Per-project capability sets (a net-new scope; unrequested).
- Changing the credential/extension-install scoping, which is already correct.
