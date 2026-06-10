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
- External channel identity is transport attribution, never a principal
  (see the identity ladder below). The only path to principal-grade
  capability is pairing to a registered user.
- Channel access is delegated by the binding act: a project member binding
  a channel to a project grants everyone in that channel conversational
  access and surface-local approval rights (see below). Authority that
  mutates project state (binding, membership, credential consent) stays
  registered-member only.
- Credentials are owned by a principal like everything else: personal
  OAuth stays `User`-owned forever; service credentials may be
  `Project`-owned; adapter/bot infrastructure is `System`-owned.

## Identity and access model

Decided 2026-06-09. Three actor classes resolve onto one principal model
plus a capability ladder — no guest principals, no shadow users.

```text
Tier 3  Registered user + Admin role   administers; owns nothing extra
Tier 2  Registered user (SSO)          full principal: owns, joins, consents
Tier 1  Paired external sender         IS a tier-2 user, via channel pairing
Tier 0  Unpaired external sender       speaker: converses, never owns
```

### Scope by surface

- DM with the app → thread owned `User(paired user)`. DMs from unpaired
  senders follow the channel `DmPolicy`; `DmPolicy::Open` is only valid in
  single-user deployments — multi-user tenants require `Pairing` for DMs
  because an open DM thread has no principal to own it.
- Message in a bound channel → thread owned `Project(p)`. Threads are keyed
  by external route (channel id + provider thread ref); the owner principal
  is an attribute of the binding, never part of the key. One channel hosts
  many threads, all owned by the bound project.

### Public/shared channels

Anyone in a bound channel — registered or not — converses with the agent
and shares the project-owned history. The member who bound the channel made
the disclosure decision for that channel; the provider's channel ACL does
admission. Unpaired senders are attributed by external identity
(`adapter`, `external_id`, display name) on their turns.

Surface-local approval: an approval gate raised by a run in a
channel-bound thread is resolvable by anyone who can speak in that channel.
The provider authenticated the sender; channel membership is the
authorization; the audit record carries the external actor. Containment:

- A gate is channel-approvable only in the channel whose thread raised it.
  Project gates raised on other surfaces stay member-resolvable.
- Auth gates (credential consent) are always owner-only: the owning user
  for `User`-owned credentials, a project member for `Project`-owned ones.
  Never channel-resolvable.
- A per-project strict mode (member-only approvals) can be added later if
  a team wants it; channel-suffices is the default.

Pairing is lazy and pulled, never required up front: an unpaired sender who
attempts a registered-only verb (personal DM scope, web access, project
binding/membership/credential management) receives a pairing prompt; the
existing pairing-code flow links their external identity to an SSO user.

### Context isolation invariants

- A project-owned thread assembles context from project scope (plus
  tenant-shared material) only — never from any member's personal memory or
  threads. A shared channel is a disclosure surface; personal context
  appearing there is a leak. Phase D makes this structural via
  principal-keyed placement; until then it is a review-enforced rule.
- The bound channel is the disclosure boundary for outbound delivery:
  anything delivered there is readable by every channel participant,
  including unregistered ones. Gate notifications to channels therefore
  remain non-authority (no auth URLs, no secrets) per the delivery
  resolution contract.

### Credentials by owning principal

- `User`-owned: personal delegated credentials (OAuth tokens, personal API
  keys). Identity-bound to a human; never re-owned to a project. Project
  runs use them via a grant — a revocable, audited lease referencing the
  user-owned credential — never a copy. Consent gates route to the owner.
- `Project`-owned: service credentials (shared API keys, service
  accounts). Added and managed by project members; usable by project runs
  without per-use consent from any individual member.
- `System`-owned: adapter/bot infrastructure (e.g. the Slack bot token),
  managed at tenant composition level.

### Agents and channel bindings

Agents are executors, never owners, so multi-agent wiring is pure routing:

- A binding pins exactly one `(project, agent)` per channel route. One
  project may have many bound channels; one channel never serves two
  projects. Personal DM bindings are per-user and unaffected.
- One adapter installation (one Slack app) can serve different agents in
  different channels via bindings. A separate app per agent is also
  supported (separate `adapter_installation_id`) when a distinct provider
  persona or its own DM surface is wanted; tenants can mix both styles.
- Binding, rebinding, and unbinding are member verbs, audit-recorded.
  An unbound channel auto-binds to the Workspace project and default agent
  on first message. Optional bind-create CX: bind a channel to a
  newly-minted project in one step (creator becomes first member).

## Phases

Ordered by dependency; each ships standalone.

### Phase B — Project registry, membership, and channel binding

Current state (audited 2026-06-09): Reborn has no project entity. A
`ProjectId` today is an unvalidated config-string tag
(`identity_section.default_project_id`, hardcoded slugs in composition);
there is no registry, no validation, and no lifecycle. The v1 engine
`Project` struct is dead code keyed by `user_id` and must not be revived.

This phase makes projects real. Registry record:

```text
ProjectRecord {
    tenant_id, project_id,            # project_id minted by the registry
    slug, display_name,
    created_by, created_at,
    state: Active | Archived,         # archive-only; never hard-delete
}
```

Registry invariants:

1. Project ids are registry-minted (UUID); config slugs are bootstrap
   references resolved through the registry, never used as keys. The
   Workspace project id is deterministic (UUIDv5 of tenant + "workspace").
2. Validate-on-write, fail-closed-read: writes naming a project id verify
   it exists and is Active; reads of scoped data tolerate archived
   projects (data stays readable) but reject unknown ids.
3. Do not revive the v1 `Project` struct or any user-keyed project shape.

Membership is the single intra-tenant authorization fact. One record shape:

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

v1 scope: the registry, the membership port, and virtual Workspace
membership — every tenant user is a member of the tenant's Workspace
project without per-user rows — plus wiring the consumers below. Explicit
membership rows and a member-management surface activate when a second
project exists: the project creator becomes its first member (Admin once
roles land), and an `add_member` port operation covers the rest. Only
registered tenant users are addable as members; external identities join
by pairing first.

The channel-bind verb also rides this phase: a member binds a channel
route to `(project, agent)` (see "Agents and channel bindings" above).
Until it ships, every channel binding lands on the Workspace default,
which is acceptable for the single-project tenant but blocks multi-project
channel routing.

Consumers wired in this phase (one store, five call sites — if any consumer
grows its own check, shared ownership fragments again):

1. Fire-time trigger authorization (closes the standing security
   placeholder that blocks external delivery).
2. Project outbound-preference writes (replaces the operator-flag gate).
3. Project automation list/create.
4. Approval-gate resolution on project runs: any member everywhere, plus
   surface-local channel approval for gates raised in a bound channel's
   threads (auth gates remain credential-owner only).
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
- `TrustedOwnerScope { Unspecified, User(UserId), Project }` is the sealed
  trusted-ingress input and maps onto the principal. The `Project` variant
  means owned by the binding's project scope, encoded as an explicitly
  absent user owner until this phase; it is the final vocabulary and is
  already in use in the implementation (see companion plan chunk 2).

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
  Surface-local approval extends reply-to-approve to unpaired channel
  participants for channel-raised gates.
- Slack DM provisioning (companion plan PR G) is phase-independent.
- Credential grants: project runs lease `User`-owned credentials through a
  revocable, audited grant (never a copy); consent gates route to the
  owner. Project-owned service credentials need no per-use consent.
  Sequenced after Phase B (grants reference membership for revocation on
  member removal).
- CX follow-up: an inbound message to a thread whose run is blocked on a
  gate is currently recorded as deferred-busy and answered with silence.
  The bot should reply with a short "waiting on approval — reply
  `approve <gate_ref>` or open IronClaw" notice. Independent of triggers;
  applies to live runs today.
- Perf note: project delivery-target listing still scans all subjects with
  pagination; if that shows up in practice, the fix is a project-scoped
  route-store query mirroring the subject-scoped one added for personal
  listing.

## Out of scope

- Org SSO, nested groups, permission matrices: membership stays one
  relation with two roles.
- Cross-tenant anything: tenant remains the hard isolation wall.
- Per-automation delivery overrides, non-text modalities, interactive
  provider-side gate UIs: tracked in the companion plan's follow-ups.
