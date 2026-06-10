# Trigger Delivery Scoped Default Outbound E2E Plan

Date: 2026-06-05. Rewritten 2026-06-09 as the compacted source of truth after
a full codebase audit, a product-decision review, and the project-scoped
architecture revision. Earlier revisions of this file (including all
`shared_agent` vocabulary) are git history; this document is authoritative.

## Goal

Trigger (automation) results become a user-visible end-to-end flow:

1. A triggered run resolves its delivery default from the run's ownership
   principal.
2. Personal automations deliver to the owning user's personal target (e.g.
   paired Slack DM).
3. Project automations deliver to the project's configured shared target
   (e.g. admin-managed Slack channel).
4. Trigger completion sends final replies — and approval/auth gate
   notifications — through `OutboundResolutionEngine` ->
   `OutboundPolicyService` -> product-adapter render. Never through WebUI
   projections.

Slack is the first provider because its adapter already renders outbound
`FinalReply`, `GatePrompt`, and `AuthPrompt` via `chat.postMessage`.

## Ownership Model (Architecture Revision, 2026-06-09)

Ownership principals: `User { user_id }`, `Project { project_id }`,
`System`. There is no shared-agent principal. Agents are executors —
execution identity, audit metadata, routing detail — never owners.

- Every shared automation belongs to a project. `ownership_scope = project`
  requires `project_id`. The previous `shared_agent` scope (keyed by
  `agent_id`) is eliminated; rename it everywhere while unlaunched.
- Each tenant bootstraps a default project ("Workspace"); every tenant user
  is a member. Shared automations with no explicit project land in
  Workspace. Workspace appears as a normal project in pickers.
- Delivery default keys: `CommunicationPreferenceKey::personal(tenant, user)`
  and `CommunicationPreferenceKey::project(tenant, project)`. The agent
  component leaves the key. Multiple agents in one project share the project
  default; per-automation overrides remain a future phase.
- Visibility and authorization are membership-based: `Membership(user,
  project)` gates seeing project automations/runs, creating automations, and
  setting/clearing the project delivery default. Any member may do all three
  in v1; per-project admin roles are a later refinement. Until the
  membership store exists, Workspace membership = all tenant users.
- `creator_user_id` stays audit/control identity (list/edit/remove), never
  delivery ownership.

### Delivery resolution rule

```text
Trigger run reaches a delivery point (final reply, approval gate, auth gate)
    |
    v
RunNotificationOrigin = Triggered | TriggeredFromSourceRoute
  (source route, when present, is provenance only — never the target)
    |
    v
TurnScope ownership:
  explicit owner user            -> personal(tenant, owner)
  explicitly ownerless + project -> project(tenant, project)
  anything else                  -> fail closed (no actor fallback)
    |
    v
Slot lookup:
  FinalReply        -> final_reply_target
  ApprovalPrompt    -> approval_prompt_target, else final_reply_target
  AuthPrompt        -> auth_prompt_target, else final_reply_target
  Progress/Status   -> progress_target (no fallback; still deferred)
  unset (after fallback) -> fail closed
    |
    v
Send-time target revalidation (provider authority) -> OutboundPolicyService
-> adapter render -> send. Missing/stale/revoked/mismatched targets fail
closed and send nothing externally.
```

Gate-prompt fallback deliveries are notifications, not interactive gates:
the payload says the automation is waiting and links to the WebUI gate
surface. Interactive gate resume from the provider is a future phase.

## Locked Decisions

Scope and resolution:

- Delivery defaults are scoped by run ownership principal, derived from the
  persisted thread/turn ownership — never from trigger creator, last editor,
  last inbound sender, actor fallback, or a synthetic user id. Missing or
  ambiguous ownership fails closed.
- Scope resolution, preference storage, target inventory, stale-target
  handling, and send-time validation are channel-neutral. Slack is a
  provider implementation, not a backend rule. No core resolver branch may
  reference Slack team/channel/DM concepts (tested).
- Channel-to-project binding for creation-time project inference lives in
  the provider route store (Slack: the channel route record). Core receives
  a trusted `project_id` from the creation surface only.
- Gate access checks are two-layer: the adapter validates provider identity
  to a paired user; core checks project membership (and credential
  ownership for auth gates).
- `ownership_scope = project` on `trigger_create` is host-derived authority:
  the field exists in the capability contract, but the host runtime rejects
  it unless the handler was constructed with shared-creation authority.
  Model tool input and browser bodies are never the source of that
  authority (implemented: `allow_shared_agent_ownership` flag, to be renamed
  with the vocabulary swap).

Targets and preferences:

- Clients submit stable `target_id` values only; backend composition
  resolves and validates. Raw `ReplyTargetBindingRef` values are never
  accepted from clients. Client-facing target failures collapse non-owned /
  unauthorized / missing / stale / capability-mismatched into one safe
  shape; provider reasons stay server-side.
- Communication-preference writes use versioned CAS. The WebUI POST uses a
  server-owned CAS flow (facade reads latest version, writes with that
  expected version, surfaces `CasConflict` as HTTP 409); byte-only backends
  that cannot preserve `Absent`/`Version` expectations fail closed.
  `CasConflict` results stay explicit — no hidden storage retry loops.
- Only the final-reply slot is exposed in the current UI slice. Non-final
  slots and `default_modality` are preserved, not dropped.
- Outbound preferences are a reusable product configuration API, not a WebUI
  rendering API: stable ids, labels, channel metadata, capability flags,
  selection, status, errors — no panel layout or copy state.
- Personal channel defaults require a concrete durable provider target.
  Slack pairing-code redemption is identity-only and never writes defaults;
  after pairing, an explicit opt-in provisions the DM target
  (`conversations.open` -> persist the `D...` id) and saves the
  provider-issued `target_id`. Provisioning is explicit, idempotent across
  processes, and never runs inside preference CAS locks.
- Project channel defaults use admin-managed shared destinations (Slack:
  admin-managed channel routes).
- Outbound ownership stays in `ironclaw_outbound` / `OutboundPolicyService`;
  adapters render only after policy approval and never load preferences or
  invent targets.

Gate prompts and outcomes:

- Approval/auth prompts on triggered runs fall back to the scope's
  final-reply target when their slots are unset; explicit slots win; neither
  set fails closed. Gate prompts are never silently dropped when the scope
  has a final-reply default. Progress never falls back.
- Delivery outcome visibility: the trigger terminal delivery caller records
  a sanitized outcome for every terminal completion — `delivered |
  no_default_configured | target_unavailable | denied | failed | skipped` +
  delivery kind + timestamp — including resolution-stage failures that must
  not create `OutboundDeliveryAttempt` rows. Outcome writes are best-effort
  and never alter delivery. Surfaced on automation rows (recent runs +
  `last_delivery`), with `no_default_configured` linking to the delivery
  panel. No triggered terminal completion ends with neither a send nor an
  outcome record.

CX (decided 2026-06-09):

1. DM to a shared agent creates a personal automation; a bound channel
   creates a project automation. Surface maps to scope; no "which project?"
   prompt.
2. Any project member may create project automations and set/clear the
   project delivery default.
3. Approval gates: resolvable by any project member. Auth gates: credential
   owner only; others see "waiting on <owner>'s <provider> authorization".
4. Creation confirmations always name the delivery destination, because
   results may go to a different channel than where the automation was
   created.
5. Workspace is shown as a real project.

## Current State (audited 2026-06-09)

Landed or staged in this worktree (pre-rename vocabulary):

- Scoped preference model + versioned CAS repository (personal +
  shared-agent keys), resolution engine with fail-closed behavior, gate
  fallback, and triggered-origin ambiguity fail-close. `ironclaw_outbound`
  tests green.
- Channel-neutral target provider/resolver contract; Slack shared-channel
  targets (admin routes, owner-change/deletion authority) and Slack personal
  DM target authority at provider/storage level. The DM provisioner is
  test-gated; no production route provisions DM targets yet (plan #4600).
- Trigger ownership scope (`personal | shared_agent`, to be renamed
  `personal | project`): record/fire fields, both repositories, materializer
  thread recording (ownerless for shared), host-runtime authority gate on
  shared creation, caller-level tests.
- Product facade: personal automations list, outbound preference get/set,
  shared-agent list facade + ownership DTO discriminator (to be renamed),
  operator-gated shared routes in WebUI v2 with handler-level guard +
  403 tests.
- WebUI: personal Automations page with Delivery defaults panel; admin
  automations tab is a non-mutating placeholder pending the project surface.
- Contract doc `docs/reborn/contracts/communication-delivery-resolution.md`
  documents the triggered fallback and fail-closed rules (rules 6-7).
- Code review (multi-agent, 2026-06-09): 16 findings; all fixed or
  dismissed-with-reason except the blocking bug below. `.review/` holds the
  findings archive.

Not built yet:

- Phase 7 trigger terminal delivery caller — `RunNotificationOrigin::
  Triggered` has zero production callers; nothing observes trigger run
  completion and calls outbound resolution/policy/adapter.
- DM provisioning product route + post-pairing prompt + panel action.
- Workspace bootstrap, membership store, membership-based projection.
- Delivery outcome recording/surfacing.
- CLI outbound setup (deferred until CLI can compose the target-provider
  registry without duplicating serve wiring).

## Blocking Bug: Ownership Does Not Reach The Turn Scope

Found 2026-06-09 by the e2e test
`trigger_poller_submits_shared_agent_turn_with_ownerless_scope` (currently
`#[ignore]`d, pointing here). Shared/project trigger fires fail at run time
before reaching the model while the poller records `last_status: Ok`:

- The materializer records the trigger thread ownerless (correct), under the
  system subtree.
- Turn submission goes through the conversation binding; the materializer
  passes `trusted_owner_user_id = None`, and `BindingRecord::resolution()`
  maps a `None` owner to `TurnScope::new(...)` — no explicit-owner marker.
- At run time `ThreadScopeResolver::resolve_for_turn` sees no explicit owner
  and actor-falls-back to the creator's `owners/<user>` subtree. The thread
  is not there -> `unknown thread` -> terminal driver failure, every fire.
  Personal triggers work only because thread and fallback happen to agree.
- The same gap breaks delivery: the run's `TurnScope` never carries the
  explicit-ownerless marker the project preference key derivation needs.

Fix (chunk 2 below): tri-state `TrustedOwnerScope { Unspecified,
Owned(UserId), Ownerless }` on the trusted binding contract;
`BindingRecord::resolution()` maps Owned -> `new_with_owner(Some)`,
Ownerless -> `new_with_owner(None)` (explicit), Unspecified -> `new()`;
materializer passes Owned(creator) for personal and Ownerless for project
fires. The ownerless state is trusted-scope-only input — raw adapters must
not reach it. Projection consequence accepted: ownerless lifecycle events
drop out of creator-owner-filtered streams until membership-based visibility
(chunk 4); project runs surface through the project tab instead. The ignored
e2e test, reworked to seed a project-scoped trigger, is the acceptance test.

## Implementation Plan

Chunks (each lands with caller-level tests; wave 1 = chunks 1-2 as four
parallel disjoint-crate workstreams, then one composition integration
workstream; chunks 3-6 follow):

1. Vocabulary swap `shared_agent` -> `project`, pre-launch, mechanical:
   - `ironclaw_outbound`: `CommunicationPreferenceKey::project(tenant,
     project)` replaces `shared_agent(tenant, agent, project?)`; resolution
     branch keys on explicitly-ownerless + `project_id`.
   - `ironclaw_triggers` + `ironclaw_host_runtime`:
     `TriggerOwnershipScope::Project` requires `project_id` (not
     `agent_id`); wire value `"project"`; authority flag renamed
     `allow_project_ownership`; schema/docs updated.
   - `ironclaw_product_workflow` + `ironclaw_webui_v2` + static frontend:
     facade methods, DTO enum, route paths (`/outbound/project/...`),
     i18n/copy.
   - `ironclaw_reborn_composition`: preference facade scope derivation
     (`project_id` required, `agent_id` no longer part of the key), Slack
     target validation maps shared-channel routes to project scope (agent
     stays route metadata).
2. Ownership propagation fix (see Blocking Bug). `ironclaw_conversations`
   `TrustedOwnerScope`, `ThreadScopeResolver` already honors explicit
   owners; materializer mapping; un-ignore + rework the e2e test.
3. Workspace bootstrap + membership store/port (composition factory +
   product workflow port). Membership checks for project list/create and
   project preference writes replace the operator-flag gate on those routes.
4. Membership-based projection visibility (Workspace-membership-for-all
   makes v1 trivial).
5. CX flows: DM=personal rule in the Slack relay, destination-naming
   confirmation copy, project tab (rename of admin tab) wired to project
   APIs, Workspace in pickers. Includes the `lib/api.js` project helpers the
   admin tab currently lacks.
6. Gate authority policy: approval = project member, auth = credential
   owner. After chunk 3.

Then the remaining feature stack:

- PR F: Phase 7 trigger terminal delivery caller — constructs
  `RunNotificationOrigin::Triggered` / `TriggeredFromSourceRoute` from the
  persisted ownership, resolves scoped defaults, validates through
  `OutboundPolicyService`, renders through the Slack adapter, records
  delivery outcomes (decision above). Requires chunks 1-2.
- PR G: personal Slack DM provisioning route + post-pairing one-click
  default prompt + panel "Connect Slack DM" action (#4600). Two entry
  points, one backend provisioning route.
- PR H: project automations surface — project list facade wired to the
  project tab, membership-gated. Requires chunk 3.
- PR I: delivery outcome surfacing in automation rows (badge + "Set
  delivery default" link).

## Acceptance Criteria (remaining work)

Scoped defaults and resolution:

- Personal runs use personal defaults; project runs use project defaults;
  ambiguous ownership, missing defaults, and stale/revoked targets fail
  closed and send nothing.
- `TriggeredFromSourceRoute` preserves provenance while scoped defaults pick
  the target.
- Approval/auth prompts deliver via explicit slot, else final-reply
  fallback, else fail closed; progress never falls back.
- CAS conflicts surface as 409 through facade and route; disjoint-slot
  merges preserve both writes.

Ownership propagation (chunk 2):

- Project trigger fire produces an explicitly ownerless turn scope; the run
  reads the project thread and reaches the model (the reworked e2e test).
- Raw adapters cannot express the ownerless trusted scope.

Slack (provider-side, channel-neutral core):

- Shared channel targets list/validate per project scope for members;
  personal DM targets list only after durable provisioned authority.
- DM provisioning is explicit, idempotent, durable, never inside CAS locks;
  send-time DM validation treats `missing_scope`, `user_not_found`,
  `user_disabled`, `user_not_visible`, `invalid_user_combination`,
  `channel_not_found` as stale/unavailable (fail closed, no infinite retry).
- Pairing-code redemption never writes defaults. Raw client Slack ids never
  become defaults.
- Sends use `chat.postMessage` with validated ids.

Trigger delivery E2E (PR F):

- Project trigger result reaches the configured project Slack channel;
  personal trigger result reaches the paired DM; gate notification reaches
  the final-reply target when no explicit gate slot is set.
- `OutboundPolicyService` runs before adapter render on every path; policy
  denial prevents egress.
- Every terminal completion records a delivery outcome; resolution-stage
  failures record outcomes without attempt rows; outcome write failure never
  alters delivery.

WebUI:

- Routes delegate through `RebornServicesApi`; malformed bodies fail before
  mutation; project routes enforce membership (operator flag until chunk 3);
  spoofed scope identifiers cannot read or mutate another scope; personal
  route never writes project defaults.
- Delivery panel shows loading/empty/error/selected/saved/clearing/stale
  states; personal and project copy are distinct; asset embedding covered.

Trigger and poller hardening (carried forward, still binding):

- Trusted trigger ingress stays sealed: authority on worker-minted
  `TrustedTriggerSubmitRequest`; raw `TrustedInboundTurnRequest` private to
  `ironclaw_conversations`; product surfaces cannot mint trusted requests;
  architecture tests keep any reusable trusted-ingress facade out of
  production paths.
- Fire-time creator authorization runs before turn submission using the real
  agent/project access source of truth (the same source membership should
  use); denied access is a permanent fire failure; authz unavailability is
  retryable without submitting a turn. External delivery stays disabled
  until this is real.
- The poller's `last_status: Ok` currently reflects submission, not run
  outcome — follow-up: reflect terminal run failure in trigger run status
  (found via the ownership bug; tracked, not yet scheduled).

## Out Of Scope / Follow-Ups

- Per-automation delivery overrides; explicit per-slot target pickers UI;
  Slack progress/projection delivery; non-text modality defaults.
- Interactive gate resume from provider surfaces (deliveries link to WebUI).
- Per-project admin roles (v1 is member-or-not).
- One-time immediate trigger creation; automation list pagination/totals.
- Local-dev store-graph persistence changes beyond the current slice; Slack
  host-beta restart-safe claims wait on durable conversation/idempotency
  state.
- Static seeded Slack channel delete-over-fallback tombstones; subject-scoped
  shared-route listing (revisit if route scans get expensive — known
  per-request pagination scan in the Slack target provider).

## Canonical Refs

- `docs/reborn/contracts/communication-delivery-resolution.md` — preference
  fields, resolution rule order (incl. gate fallback rules 6-7), trigger
  delivery boundary.
- `docs/reborn/contracts/product-adapters.md` — adapter outbound rendering
  boundary.
- `crates/ironclaw_outbound/src/resolution_engine.rs` — triggered resolution
  + fail-closed + fallback.
- `crates/ironclaw_outbound/src/communication_preferences.rs` — scoped
  preference model + CAS contract.
- `crates/ironclaw_reborn_composition/src/outbound_preferences.rs` —
  preference facade (server-owned CAS write path).
- `crates/ironclaw_reborn_composition/src/slack_outbound_targets.rs` — Slack
  target authority (shared channel + personal DM).
- `crates/ironclaw_reborn_composition/src/trigger_poller_trusted_submit.rs`
  — materializer; ownership mapping.
- `crates/ironclaw_reborn/src/thread_scope.rs` — per-caller thread scope
  resolution (the actor-fallback rule).
- `crates/ironclaw_conversations/src/memory.rs` — binding resolution; the
  trusted owner scope contract lands here.
- `crates/ironclaw_product_workflow/src/reborn_services.rs` — product
  facades.
- `crates/ironclaw_webui_v2/src/handlers.rs`,
  `crates/ironclaw_webui_v2_static/static/js/pages/automations/` — WebUI
  surface.
