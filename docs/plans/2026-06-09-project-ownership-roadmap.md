# Project Ownership Roadmap

Date: 2026-06-09

Status: follow-up roadmap for the project-scoped ownership model. The
companion implementation plan is
`docs/plans/2026-06-05-trigger-delivery-default-outbound-e2e-plan.md`; this
document owns the phases that come after the core model and surface-rename
PRs merge.

## Context

The 2026-06-09 architecture revision replaced shared-agent ownership with
ownership principals: `User`, `Project`, `System`. Agents are executors —
execution identity, audit metadata, routing detail — never owners. The
landed increment encodes project ownership transitionally (an explicitly
absent user owner plus a required `project_id` on the scope) because the
core scope types still carry `Option<UserId>`. This roadmap removes that
encoding and builds the membership layer that makes shared ownership real.

Decision record (2026-06-09):

- Proceed with shared ownership rather than parking after the personal E2E.
  The dormant-by-default authority gate (`allow_project_ownership = false`
  at every production call site) contains risk until membership lands.
- Project, not shared-agent, is the sharing unit. Every shared automation
  belongs to a project; each tenant bootstraps a default "Workspace"
  project that is shown as a normal project.
- The concept inventory is fixed at three: tenant (hard isolation, exists),
  ownership principal (one enum), membership (one relation; admin is a role
  value, not a concept). New sharing features must not introduce a fourth
  mechanism.

## Phases

Ordered by dependency; each ships standalone.

### Phase B — Membership store and port

The single intra-tenant authorization fact. One record shape:

```text
ProjectMembershipRecord {
    tenant_id, project_id, user_id,
    role: Member | Admin,          # v1 uses Member only
    added_by, added_at,
    state: Active | Removed,       # never hard-delete; audit trail
}
```

Port: `is_member`, `role`, `projects_for_user`, `members_of` (paged).
Versioned CAS writes like the communication-preference store.

v1 scope: the port plus virtual Workspace membership — every tenant user is
a member of the tenant's Workspace project without per-user rows — plus
wiring the consumers below. Explicit membership rows and a member-management
surface wait until a second project exists.

Consumers wired in this phase (one store, five call sites — if any consumer
grows its own check, shared ownership fragments again):

1. Fire-time trigger authorization (closes the standing security
   placeholder that blocks external delivery).
2. Project outbound-preference writes (replaces the operator-flag gate).
3. Project automation list/create.
4. Approval-gate resolution on project runs (any member; auth gates remain
   credential-owner only).
5. Projection visibility (consumed later by Phase E).

Workspace bootstrap rides this phase: tenant composition ensures the
Workspace project exists and the virtual-membership rule covers it.

### Phase C — OwnershipPrincipal on the core scope types

Replace `owner_user_id: Option<UserId>` plus the explicit-ownerless
convention with a typed principal on `ThreadScope`, `TurnScope`, the
conversation binding owner state, and milestone scopes:

```text
OwnershipPrincipal = User(user_id) | Project(project_id) | System
```

- Serde/wire compatibility: `Some(user)` decodes as `User`; legacy absent
  owner decodes as `System`; the explicit-ownerless-with-project encoding
  decodes as `Project`.
- Deletes the remaining actor-fallback sites (thread-scope resolver,
  milestone sink owner derivation, run resource-scope derivation) — the
  bug class behind the 2026-06-09 "unknown thread" and milestone-scope
  failures dies with the enum, not with per-site fixes.
- Resolves the `types.md` violation: an `Option<UserId>` that means two
  different things by context is exactly the stringly-typed identity
  hazard that rule exists to prevent. This phase is scheduled immediately
  after Phase B, not "someday", so the convention does not accumulate new
  callers.
- `TrustedOwnerScope` stays as the sealed trusted-ingress input and maps
  onto the principal; its variants should be named for ownership
  (`User`/`Project`), not for the encoding (`Ownerless`).

### Phase D — Principal-keyed storage placement

Move project-owned data out of the transitional placement (`__system__`
thread subtree; events filed under the run actor) into principal-keyed
paths (`/tenants/<t>/principals/project/<p>/...`). Riskiest phase (path
migration); sequenced after C so paths derive from the typed principal and
are migrated once. Lifecycle decisions ride here: projects archive rather
than delete (the LLM-data-is-never-deleted rule means last-member-leaves
keeps the project tenant-retained, never orphaned under a personal
subtree).

### Phase E — Membership-based projection visibility

Lifecycle events and projection streams filter by "principals the caller
belongs to" instead of `owner == me`. Creators see the project runs they
are members of; the `initiated_by` metadata band-aid is not built. With
virtual Workspace membership this is near-trivial for the single-project
case.

### Phase F — Roles

`Admin` role gates member management and any project mutations the product
decides are not member-grade. Retires the global operator flag for project
surfaces. Deliberately last: v1 sharing works with member-or-not.

## Feature work interleaved with the phases

These ship against the companion plan and consume the phases above:

- Trigger terminal delivery (companion plan PR F) requires Phase B for
  fire-time authorization before external delivery is enabled. Gate
  notifications embed the parseable `gate_ref` and a WebUI deep link
  (`thread_id`/`run_id`/`gate_ref`): personal reply-to-approve then works
  through the existing Slack `approve <gate_ref>` ->
  `ApprovalInteractionService` seam with no new resolution machinery;
  project-member reply-to-approve activates when consumer 4 above lands.
- Slack DM provisioning (companion plan PR G) is phase-independent.
- CX follow-up: an inbound message to a thread whose run is blocked on a
  gate is currently recorded as deferred-busy and answered with silence.
  The bot should reply with a short "waiting on approval — reply
  `approve <gate_ref>` or open IronClaw" notice. Independent of triggers;
  applies to live runs today.

## Out of scope

- Org SSO, nested groups, permission matrices: membership stays one
  relation with two roles.
- Cross-tenant anything: tenant remains the hard isolation wall.
- Per-automation delivery overrides, non-text modalities, interactive
  provider-side gate UIs: tracked in the companion plan's follow-ups.
