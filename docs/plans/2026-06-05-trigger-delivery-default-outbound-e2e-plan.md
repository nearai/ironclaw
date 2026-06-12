# Trigger Delivery Scoped Default Outbound E2E Plan

Date: 2026-06-05

Status: source-of-truth implementation plan, revised 2026-06-08

Context: the discuss-phase workflow was requested for this work, but this
checkout has no `.planning/ROADMAP.md` phase entry to attach to. This
repo-native plan captures the phase context and implementation decisions for
downstream agents.

This plan supersedes the removed 2026-05-29 trigger-loop delivery resolution
plan for trigger delivery and default outbound delivery planning. That older
plan is available only through git history; this file is the repo-local source
of truth.

## Summary

This plan turns trigger result delivery from an internal fast-follow into a
user-visible end-to-end flow with scoped delivery defaults:

1. A triggered event resolves the delivery default for the run's owner scope.
2. Shared tenant agents send to an admin-configured shared delivery channel.
3. Personal agents send to the owning user's personal delivery target, such as
   a paired Slack DM.
4. Trigger completion sends final replies through the existing outbound policy
   and product-adapter path.

Slack is the first external delivery channel because the current Slack adapter
already supports outbound `FinalReply`, `GatePrompt`, and `AuthPrompt` via
Slack `chat.postMessage`. The first E2E remains text final replies only; Slack
progress/projection payloads and non-text modality defaults remain deferred.

## Completed Prerequisites

- Product-workflow outbound preference facade exists:
  `get_outbound_preferences`, `set_outbound_preferences`, and
  `list_outbound_delivery_targets`.
- Client-safe target DTOs exist for target IDs, labels, channel metadata, and
  capability flags.
- Composition has a product-surface-neutral outbound preferences facade backed
  by `RebornLocalRuntimeServices::outbound_preferences`.
- Phase 2 introduces scoped preference storage through `DeliveryDefaultScope`
  so defaults can be personal or shared-agent scoped.

## Implementation Snapshot

- Base new implementation PRs on `origin/main`, not `origin/reborn-integration`.
- The old `trigger-delivery-scoped-defaults` commit and phase worktrees are
  reference snapshots only. Do not blindly rebase or cherry-pick them.
- The current branch payload should be semantically ported onto `main`:
  migrate the current internal product-workflow preference calls onto scoped
  versioned repository operations instead of preserving a compatibility shim for
  unlaunched callers.
- Old phase worktree copies of the 2026-05-29 plan should be treated as
  historical comparison context only; update this document as the source of
  truth.
- Slack host-beta can expose configured/persisted Slack channel routes through
  a generic `OutboundDeliveryTargetProvider` staged on `SlackHostBetaMounts`.
- Slack final-reply delivery reads the shared
  `local_runtime.outbound_preferences` repository instead of a private
  in-memory preference repository.
- **Resolved 2026-06-11 (PR #4782):** the carry-forward finding from the
  2026-05-29 plan (Slack host-beta constructing its own outbound state store)
  is closed. `RebornLocalRuntimeServices` now owns a single outbound store
  exposing preferences, state, gate-routes, and triggered-delivery handles;
  `slack_host_beta` consumes those handles rather than constructing a private
  `FilesystemOutboundStateStore`. The `/outbound` grant was removed from
  `slack_host_state_mount_view`, and a `clippy::disallowed-methods` lint bans
  `FilesystemOutboundStateStore::new` outside the factory.
- The staged Slack provider is not yet wired into the WebUI/API bundle and
  should not become user-selectable until the scoped authority model below is
  implemented.

## Known Gaps Before Phase 2

- Shared-agent default delivery scope is implemented in Phase 2.
- There is no durable target-authority resolver for validating a saved Slack
  channel or DM target at trigger-fire time.
- WebUI v2 still has no outbound preference/target HTTP routes.
- Automations has no Delivery panel.
- The base Automations create/list/remove surface and trigger-type catalog are
  owned by the PR 18.9/18.10 trigger-management plan. This delivery plan
  assumes that source-agnostic Automations shell exists; it must not define a
  separate trigger-type discovery contract or hardcode source placeholders.
- Trigger terminal completion is not yet wired to external outbound delivery.

## Carry-Forward Deferred Work

These items came from the earlier default-outbound and trigger-delivery review
loops. They are not optional polish; each one must either be completed before
the affected phase ships or explicitly deferred again in that PR body with a
tracked follow-up.

### Repository And CAS Hardening

- Harden communication-preference read-modify-write semantics before depending
  on concurrent preference updates for production delivery behavior.
- Byte-only filesystem roots must not downgrade `Absent` or `Version` compare
  and swap expectations to unconditional `Any` writes. Safe outcomes are:
  - preserve `Absent` write-once semantics
  - fail closed for unsupported versioned CAS
  - use a real scoped backend lock/transaction where available
- Do not treat a process-local mutex as the production CAS guarantee. A
  byte-only `LocalFilesystem` fallback may use a path lock plus process-local
  version overlay so local/dev preference updates do not silently overwrite
  stale writes, but restart-safe or multi-process scoped defaults require a
  real versioned backend lock/transaction.
- Phase 2 chooses the fail-closed outcome for communication preferences when a
  filesystem backend cannot preserve versioned CAS. Any byte-only local/dev
  overlay is a follow-up and must not silently fall back to unconditional
  writes.
- Phase 2 intentionally preserves explicit `CasConflict` results for valid
  versioned preference write races. Do not reintroduce hidden storage-level
  retry loops that blindly rewrite a complete preference record after a
  conflict; storage does not know whether the caller intended to merge or
  replace each slot.
- Follow-up: extend `WriteCommunicationPreferenceRequest` with an explicit
  expected key or expected scope before tightening mismatch classification.
  Then `expected_key != record.key()` can fail as `InvalidRequest`, while true
  stale-version or missing-row races continue to surface as `CasConflict`.
- Add caller-level tests through the preference repository or product facade,
  not only helper-level tests.

### Product API Follow-Ups

- Add conflict handling for outbound preference updates in the product API/UI
  phase. Recommended first behavior: surface a stable conflict response that
  tells the client to reload the latest preference before retrying. If the UI
  needs lower-friction saves later, add a bounded product-facade retry that
  reloads the latest record and reapplies only the requested field, with tests
  for same-field conflicts and disjoint-field preservation.
- Expand public modality support in a dedicated API phase before surfacing
  non-text defaults. The outbound repository can store more modalities than
  the current product response DTO intentionally exposes.
- Phase 3 adds a product-safe stale-target status DTO surface through
  `RebornOutboundDeliveryTargetStatus`. The low-churn response shape keeps the
  current selected-target summary field and adds the sibling
  `final_reply_target_status` value: `none_configured`, `available`, or
  `unavailable`.
- Keep route/UI state out of backend DTOs. Backend status may describe target
  availability and validation errors; it must not encode Automations panel
  layout, button labels, saved/loading state, or WebUI copy.

### Persistence And Local-Dev Store Graph

- Local-dev builds with the `postgres` feature do not change store topology in
  this E2E. Broad filesystem persistence for outbound preferences or the wider
  non-libSQL local-dev store graph is a follow-up after scoped delivery lands.
- Do not switch only one preference store casually while run, thread,
  checkpoint, event, conversation-binding, idempotency, or delivery-attempt
  state remains in-memory. A persisted default target with non-persisted
  authority state can create false confidence after restart.
- Slack host-beta currently still uses in-memory conversation bindings,
  idempotency, and delivery-attempt state. Shared default delivery must not
  claim restart-safe Slack E2E until the target authority state needed for
  validation is durable.

### Trigger And Poller Follow-Ups

- Trigger terminal delivery must not use WebUI projections as a substitute for
  product-adapter outbound delivery.
- Preserve the trigger-poller security fast-follows from the PR 19 plan:
  trusted trigger ingress must stay sealed, and fire-time creator authorization
  must use the real agent/project access source of truth before a shipped
  runtime enables arbitrary user-created trigger firing.
- Sealed trusted trigger ingress means:
  - trusted trigger authority stays on worker-minted
    `TrustedTriggerSubmitRequest`;
  - raw `TrustedInboundTurnRequest` construction stays private inside
    `ironclaw_conversations`;
  - composition receives only the narrow trigger-fire submission operation it
    needs;
  - product adapters, product workflow, first-party capabilities, and generic
    WebUI/API handlers must not mint or submit host-trusted trigger requests;
  - architecture or negative tests must keep any reusable
    `ironclaw_trusted_ingress`-style facade out of production dependency paths.
- Fire-time creator authorization means:
  - the authorization request carries `tenant_id`, `creator_user_id`,
    `agent_id`, `project_id`, `trigger_id`, and `fire_slot`;
  - authorization runs before trusted inbound turn submission and before prompt
    thread recording;
  - denied or revoked access is a permanent trigger-fire failure;
  - backend authorization unavailability is retryable and must not submit a
    turn;
  - external delivery remains disabled until this uses the real agent/project
    access source of truth, not a tenant-only placeholder.
- One-time immediate trigger creation remains out of scope for this E2E unless
  explicitly pulled into a later phase.

## Locked Decisions

- Trigger delivery defaults are scoped by run ownership:
  - personal agent/run -> personal delivery default
  - shared tenant agent/run -> shared-agent delivery default
- Scope resolution, preference storage, target inventory, stale-target handling,
  and send-time authority validation must be channel-neutral. Slack is the
  first provider implementation, not a special backend rule.
- Keep per-automation delivery overrides out of the first E2E.
- Store only the final-reply target in the first UI slice. Preserve the
  non-final slots inside `CommunicationPreferenceTargets` and preserve
  `default_modality` unless a later phase explicitly expands those contracts.
- Do not let any client write arbitrary `ReplyTargetBindingRef` values. Clients
  submit stable `target_id` values; backend composition resolves and validates
  those targets.
- Treat outbound preferences as a reusable product configuration API, not a
  WebUI rendering API. Backend responses may expose stable ids, labels, channel
  metadata, capability flags, current selection, status, and errors; they must
  not encode Automations panel layout, button/copy state, saved/loading UI
  state, or WebUI-only presentation decisions.
- Do not set a channel default during identity-only pairing flows. For Slack,
  pairing-code redemption binds identity only and does not have a concrete
  reply target.
- After successful pairing, the UI may offer an explicit follow-up action to
  use the user's Slack DM as the personal default for triggered automation
  delivery. Accepting that prompt provisions or validates a concrete DM target,
  then saves its provider-issued `target_id`; pairing success alone never
  mutates defaults.
- Personal channel defaults require a concrete personal target backed by
  durable provider authority. For Slack this is a DM-capable target, typically
  after a real inbound Slack DM or an explicit DM target provisioning flow.
  Pairing identity alone is not sufficient authority for a saved personal
  delivery default.
- Shared-agent channel defaults use admin-managed shared destinations. For
  Slack this is an admin-managed channel route.
- Missing, stale, deleted, revoked, or ownership-mismatched targets fail closed
  and send nothing externally.
- Trigger delivery derives `DeliveryDefaultScope` from the persisted run/agent
  ownership record, not from the trigger creator, last editor, last inbound
  sender, or a synthetic user id. Missing or ambiguous ownership fails closed.
- Communication-preference writes must use versioned compare-and-swap. Byte-only
  filesystem fallback must fail closed when it cannot preserve `Absent` or
  `Version` expectations; unconditional `Any` writes are not acceptable for
  preferences.
- Preserve outbound ownership: target choice and validation stay under
  `ironclaw_outbound` / `OutboundPolicyService`; product adapters render only
  after policy approval.
- Implement in a separate worktree off `origin/main`; leave the dirty main
  checkout alone.

## Core Model

Introduce a delivery default scope before wiring any provider deeper:

```text
DeliveryDefaultScope
  Personal {
    tenant_id,
    user_id
  }

  SharedAgent {
    tenant_id,
    agent_id,
    project_id?
  }
```

Triggered delivery with no live source route resolves defaults in this order:

```text
Trigger completes
    |
    v
Does the run have a live source route?
    |
    +-- yes --> TriggeredFromSourceRoute
    |           validate observed source route
    |             |
    |             +-- valid --> send to that source route
    |             |
    |             +-- missing/stale/invalid --> fail closed
    |                                        do not fall back to scoped defaults
    |
    +-- no ---> Determine run ownership scope
                    |
                    +-- Personal     -> personal default, e.g. Slack DM
                    |
                    +-- SharedAgent  -> shared default, e.g. Slack #alerts
                    |
                    v
              resolve saved target authority
                    |
                    +-- valid --> OutboundPolicyService -> adapter render/send
                    |
                    +-- missing/stale/invalid --> fail closed
```

Definitions:

- Live source route: a run was directly caused by an inbound product event and
  still carries that event's concrete reply target. For Slack this means an
  observed Slack envelope or accepted inbound message produced the
  `reply_target_binding_ref`; future providers should expose equivalent
  observed source authority without changing outbound resolution rules.
- Default outbound route: where trigger results go when no inbound conversation
  caused the run.

## Provider-Neutral Contract

Every outbound channel provider must plug into the same three concepts:

1. Target inventory
   - lists client-safe targets for a delivery scope
   - returns stable `target_id`, display metadata, channel/provider id, and
     capability flags
   - does not expose raw provider credentials or raw `ReplyTargetBindingRef`
     values to clients

2. Target authority resolution
   - resolves a selected `target_id` before writing a default
   - revalidates a saved default before sending
   - returns stale/unavailable when the provider-side route, pairing,
     permission, or destination no longer exists
   - validates already-durable concrete targets; identity-only bindings and
     generic provider names are not writable defaults

3. Product-adapter delivery
   - receives only validated target metadata
   - renders provider-specific payloads after `OutboundPolicyService` approval
   - never loads preferences or invents default targets itself

Slack-specific details such as team id, channel id, DM target, and
`chat.postMessage` belong inside the Slack provider/adapter implementation.
They must not leak into the core default-resolution rules.

## Slack Mapping

Slack has two defaultable target families:

1. Shared agent channel target
   - Source: admin-managed Slack channel route.
   - Scope: `SharedAgent { tenant_id, agent_id, project_id? }`.
   - Example: shared tenant agent sends trigger results to `#alerts`.
   - Required authority: route still exists, belongs to the tenant/agent scope,
     and the Slack installation/team/channel remain postable.

2. Personal DM target
   - Source: durable Slack identity binding plus a DM-capable target.
   - Scope: `Personal { tenant_id, user_id }`.
   - Example: personal agent sends trigger results to the owner's Slack DM.
   - Required authority: user remains paired and the DM target can be resolved
     without trusting raw client input.
   - Provisioning flow: call Slack `conversations.open` with the paired Slack
     user id, store the returned DM conversation id as durable provider
     authority, and send later with `chat.postMessage` using that DM
     conversation id.
   - Required Slack scopes: `im:write` to open or resume the 1:1 DM and
     `chat:write` to post. `im:read` is needed only if validation uses
     `conversations.info`; `users:read` is needed only if the provider must
     discover Slack user ids itself.

Not defaultable in the first E2E:

- pairing-code redemption by itself
- arbitrary Slack channel ID supplied by the client
- a generic "Slack" channel with no concrete conversation
- implicit "last Slack conversation" defaults
- Slack progress/projection delivery
- non-text modality defaults

## Canonical Refs

- `docs/reborn/contracts/communication-delivery-resolution.md`
  - preference fields, rule order, and trigger delivery boundary.
- `docs/reborn/contracts/product-adapters.md`
  - adapter outbound rendering boundary and Slack-like external channel
    capability model.
- `crates/ironclaw_outbound/src/communication_preferences.rs`
  - current tenant/user-scoped preference baseline; Phase 2 should add or
    replace this with the scoped default model and versioned repository
    contract described below.
- `crates/ironclaw_outbound/src/resolution_engine.rs`
  - triggered notification preference lookup and fail-closed behavior.
- `crates/ironclaw_product_workflow/src/outbound_delivery.rs`
  - `prepare_and_render_product_outbound` validation-before-render path.
- `crates/ironclaw_reborn_composition/src/slack_delivery.rs`
  - Slack final-reply observer and observed reply-target authority.
- `crates/ironclaw_reborn_composition/src/slack_personal_binding_pairing_serve.rs`
  - pairing-code redeem route; identity-only today.
- `crates/ironclaw_webui_v2/src/handlers.rs`
  - WebUI handlers must delegate through `RebornServicesApi`.
- `crates/ironclaw_webui_v2_static/static/js/pages/automations/automations-page.js`
  - Automations page composition point.

## Phase Breakdown

Break the work into small PR-sized phases. Each phase should land with focused
caller-level tests and should not require later phases to make its local
contracts correct.

### Phase 1 — Existing Product Facade Baseline

Status: completed before this revision.

Goal: create the client-safe product workflow contract for reading/writing a
final delivery target and listing eligible targets.

Exit criteria now carried forward:

- Future WebUI handlers, CLI commands, Slack prompts, and product surfaces can
  call a stable facade without knowing outbound storage.
- Client input uses `target_id`, not raw `ReplyTargetBindingRef`.
- The facade remains product-surface neutral.

### Phase 2 — Scoped Default Model And Repository Contract

Goal: add scoped delivery defaults and make outbound resolution choose between
personal DM defaults and shared-agent channel defaults.

Current implementation approach:

- Start from a clean `origin/main` worktree.
- Port the scoped-default model from the old `trigger-delivery-scoped-defaults`
  branch as a semantic change, not a direct conflict-marker resolution.
- Make the versioned scoped repository contract the only supported mutation
  path for this phase.
- Treat `put_communication_preference` as an insert-only seed/create helper.
  Existing preference mutation must read the scoped version or ETag and write
  through the versioned repository contract.
- Do not preserve `update_communication_preference` as a compatibility adapter.
  The old preference callback API has not launched, so current internal callers
  should migrate directly to read scoped preference, apply the update with the
  observed version or ETag, and write with CAS.
- Do not add v1 filesystem path dual-read migration in this phase because the
  outbound preference surface has not launched with durable production rows.
  The record decoder still accepts legacy `{ tenant_id, user_id }` payloads
  when such a row is loaded through the current storage path; live migration of
  old path hashes is unnecessary unless a future rollout identifies launched
  persisted rows.
- Any temporary helper used during the phase must be private, must require the
  observed scoped version or ETag, and must be removed before Phase 2 exits.
- Keep the PR scoped to outbound model, repository/storage behavior,
  default-resolution behavior, and caller-level tests. WebUI routes, Slack
  target authority, and trigger terminal E2E remain later phases.

Deliverables:

- `DeliveryDefaultScope` or equivalent model:
  - `Personal { tenant_id, user_id }`
  - `SharedAgent { tenant_id, agent_id, project_id? }`
- Scope-aware preference identity, such as
  `CommunicationPreferenceKey { scope: DeliveryDefaultScope }`, so shared-agent
  records never use synthetic user ids or collide with personal defaults.
- Repository contract/storage for shared-agent defaults.
- CAS-safe read-modify-write behavior for every scoped default store path.
- Scoped preference reads return a version or ETag, and set/clear operations
  require the caller's expected version. Stale same-slot writes return a
  conflict instead of silently overwriting a newer default.
- Compatibility note for existing user-scoped `CommunicationPreferenceRecord`
  payloads and whether a path-level migration is required.
- Resolution tests proving:
  - personal runs use personal defaults
  - shared tenant-agent runs use shared-agent defaults
  - missing defaults fail closed
  - stale or mismatched scopes fail closed
  - concurrent updates do not drop existing slots or overwrite a racing writer
  - stale same-slot updates conflict, while intentional disjoint-slot merges
    preserve both updates

Exit criteria:

- Trigger delivery no longer assumes every default is tenant/user scoped.
- Shared-agent defaults are not faked through a synthetic user id.
- Outbound resolution consults shared-agent defaults for ownerless shared-agent
  scopes and personal defaults for explicitly owned scopes.
- Personal preference behavior remains backward compatible.
- CAS fallback behavior fails closed when versioned CAS is unsupported;
  unconditional `Any` writes are not an acceptable fallback.

### Phase 3 — Channel-Neutral Target Authority Resolver

Goal: validate saved target IDs at write time and trigger-fire time without
adding Slack-specific backend bridge logic.

Deliverables:

- Generic target inventory/provider contract for listing client-safe targets.
  Target inventory must be bounded by cursor/limit or an equivalent scoped
  search contract; clients must not force providers to materialize every target
  in a tenant on each refresh.
- Generic target authority resolver:
  - `target_id` -> validated reply target candidate
  - saved/default target -> revalidated authority at send time
- Direct target lookup by `(delivery_scope, target_id)` for preference writes
  and send-time revalidation; implementations must not repeatedly enumerate the
  full provider inventory just to resolve one saved target.
- Stale-target outcome that callers can surface safely without making missing
  inventory look valid.
- Client-facing errors for non-owned, non-authorized, missing, stale, or
  capability-mismatched targets must use one safe external failure shape.
  Provider-specific reasons stay server-side for logs/metrics.
- Tests for valid target, invalid target, deleted target, ownership change,
  tenant isolation, and user isolation.

Exit criteria:

- Clients never submit arbitrary raw `ReplyTargetBindingRef` values that are
  accepted without backend validation.
- The same resolver path is used for preference writes and trigger delivery
  revalidation.
- The contract can support Slack now and future outbound channels later.
- No core resolver branch matches on Slack-specific team/channel/DM concepts.

### Phase 4 — Slack Target Implementations

Goal: implement Slack as the first channel using the channel-neutral target
provider and authority resolver.

Current implementation status:

- PR C1 is merged: shared-agent Slack channel targets are backed by
  admin-managed Slack channel routes.
- PR C1 treats persisted admin routes as authoritative over static seeded
  Slack channel fallback when the same channel has a stored owner.
- PR C2 implements provider-side personal Slack DM target authority:
  `conversations.open` provisions the concrete `D...` conversation id, the
  Slack host-state store persists it under the Slack personal-binding mount,
  and the outbound target provider lists a personal DM target only after that
  durable authority exists.
- Static seeded Slack channel deletion needs explicit tombstone/disabled-route
  state before delete can override startup fallback. PR C1 does not invent that
  persistence contract; keep it as a follow-up if static seeded routes must be
  revocable from the admin UI.
- PR C2 moves Slack outbound target authority into
  `slack_outbound_targets.rs`, keeping shared-channel and personal-DM Slack
  details out of core outbound preference logic.
- PR C1 pages through the existing route-store API for shared-channel target
  inventory so stored routes past the first page still override static fallback.
  Follow up with a subject-scoped route-store query if route inventory scans
  become too expensive for tenants with many Slack channel routes.
- PR C2 keeps the Slack reply-target binding formatter inside the Slack
  outbound-target module. Follow up with a shared bounded binding-ref helper
  only if another provider needs the same formatter shape.
- Slack pairing-code redemption remains identity-only. It must not synthesize a
  personal default, write preferences, or treat a paired Slack user id as a
  deliverable DM target.

Deliverables:

- Shared-agent Slack channel target backed by admin-managed Slack channel
  routes. Implemented in PR C1.
- Personal Slack DM target backed by durable Slack identity/DM target authority.
  Implemented in PR C2 at provider/storage level; user-facing provisioning and
  default-selection routes remain Phase 5.
- Slack route deletion/owner-change tests. Covered in PR C1 for admin-managed
  shared-channel targets, plus persisted-owner override for static seeded
  channels. Static seeded delete-over-fallback needs tombstone state.
- Subject-scoped shared-route listing and shared binding-ref helper extraction
  remain follow-ups, not Phase 4 blockers.
- Pairing-code redemption tests proving it remains identity-only.
- First-inbound/DM target tests proving only a concrete target can become a
  personal default. PR C2 covers explicit DM provisioning plus "no provisioned
  authority means no personal target"; first-inbound prompt routing remains
  Phase 5.

Exit criteria:

- Shared tenant agents can target an admin-configured Slack channel. PR C1
  covers listing, preference selection, deletion revocation, and owner-change
  authority movement.
- Personal agents can target the owner's Slack DM after DM authority exists.
  PR C2 implements provider-side target inventory after authority exists;
  selecting it as a default through user-facing routes remains Phase 5.
- No target is synthesized from arbitrary client input.
- Slack final reply delivery still uses `chat.postMessage`.

### Phase 5 — WebUI v2 Outbound Preference Routes

Goal: expose scoped defaults and target inventory through WebUI v2 routes while
preserving the existing route/facade boundary.

Deliverables:

- Route descriptors, handlers, and router mounts for:
  - `GET /api/webchat/v2/outbound/preferences`
  - `PUT /api/webchat/v2/outbound/preferences`
  - `GET /api/webchat/v2/outbound/targets`
- Request shape that includes or derives delivery scope safely.
- Preference GET responses include the scoped compare token (`version` or
  `etag`), and PUT/clear requests must supply the caller's observed token unless
  the endpoint explicitly uses a server-owned CAS flow.
- Admin/operator authorization for shared-agent default updates.
- Browser-authenticated state-changing routes must use the existing WebUI v2
  CSRF/origin protection; bearer-token-only variants must state that explicitly.
- Handler tests proving every route delegates through `RebornServicesApi`.
- Descriptor tests for route IDs, methods, paths, and product workflow effect
  ownership.

Exit criteria:

- HTTP clients can read options and update final targets through product
  workflow only.
- Personal users cannot mutate shared-agent defaults unless authorized.
- Malformed requests fail before preference mutation.

### Phase 6 — Automations Delivery Panel

Goal: make the default delivery target visible and editable from Automations.

Deliverables:

- Shared WebUI API helpers for outbound preferences and targets.
- `useDeliveryDefaults` React Query hook.
- Standalone Delivery panel above the automations list.
- Personal-agent state: default DM/personal target or none.
- Shared-agent/admin state: default shared channel or none.
- Empty, loading, error, selected, saved, clearing, and stale-target states.
- Static API tests and asset embedding coverage.

Exit criteria:

- Users can see the default that a background trigger will actually use.
- Authorized users can change or clear the applicable default.
- Trigger source/type selector behavior continues to come from the base
  Automations trigger-type catalog, not this delivery panel.
- The automations table remains structurally unchanged.

### Phase 7 — Trigger Completion Delivery E2E

Goal: connect trigger terminal completion to external final-reply delivery.

Deliverables:

- Trigger terminal delivery caller that constructs `RunNotificationOrigin`.
- `Triggered` final-reply path through scoped default resolution.
- `TriggeredFromSourceRoute` source-route precedence coverage.
- Authoritative `DeliveryDefaultScope` lookup from the persisted run/agent
  ownership record, with fail-closed behavior when ownership is missing,
  ambiguous, or inconsistent with the trigger context.
- Validation through `OutboundPolicyService` before adapter render.
- Slack E2E or integration coverage proving:
  - shared-agent trigger result reaches configured Slack channel
  - personal-agent trigger result reaches paired Slack DM
  - missing/stale targets fail closed

Exit criteria:

- Missing defaults fail closed and send nothing.
- Invalid/revoked targets fail validation and send nothing.
- Valid Slack defaults render through the Slack product adapter, not WebUI
  projections.

## Suggested PR Stack

1. PR A: scoped default model and repository contract.
2. PR B: channel-neutral target authority resolver.
3. PR C: Slack shared-channel and personal-DM target implementations.
4. PR D: WebUI v2 routes.
5. PR E: Automations Delivery panel.
6. PR F: trigger terminal delivery E2E.

If Slack authority exposes unexpected storage gaps, split PR C into:

- PR C1: Slack shared-channel target authority. Active/current slice.
- PR C2: Slack personal DM target authority. Required follow-up because current
  pairing persists identity only and does not persist a concrete Slack DM
  conversation target.

## Implementation Plan

### 1. Scoped Default Model

Add a default delivery scope contract before adding more Slack UI wiring.

The repository/API should support:

- read scoped default
- update scoped final target
- clear scoped final target
- preserve non-final slots for the same scope
- reject cross-scope writes
- return a scoped version or ETag on reads
- require that expected version or ETag on set/clear writes
- return a conflict for stale same-slot writes
- expose that version or ETag through product-safe DTOs whenever a later
  product/API route lets clients update or clear the default

The old user-scoped `CommunicationPreferenceRecord` can remain the personal
scope implementation initially, but shared-agent defaults need their own real
storage shape.

### 2. Target Inventory And Authority

Add a composition-owned provider/resolver that:

- lists client-safe eligible targets for a requested delivery scope
- resolves a client-selected `target_id` before writing preferences
- revalidates a saved target before trigger delivery sends
- returns a stale/unavailable result when the target disappeared
- supports bounded inventory pagination and direct target lookup so read,
  write, and send-time validation paths do not require full provider scans

Target inventory must be backed by known conversation/reply-target authority,
not raw client input.

This layer must be written in provider-neutral terms such as target id, delivery
scope, capabilities, validation status, and verified outbound metadata. Provider
implementations may use Slack channel routes, DMs, or future channel-specific
state internally, but core outbound code must not encode those channel details.

### 3. Slack Authority

Implement Slack target authority in two paths:

- Shared channel route:
  - route exists
  - route belongs to the tenant/agent/project scope
  - Slack installation/team/channel metadata matches
  - route is still postable
- Personal DM:
  - user is paired
  - DM target authority already exists as a durable concrete target before it
    is listed or saved as a default
  - explicit post-pairing opt-in can provision the DM target by calling Slack
    `conversations.open(users = slack_user_id)` and storing the returned
    `D...` conversation id as provider authority
  - any DM target provisioning flow is explicit, idempotent across processes,
    keyed by tenant/user/provider identity, and backed by a durable unique
    constraint or equivalent insert-if-absent operation
  - target belongs to the same tenant/user

Do not write defaults at pairing-code redemption time. Do not run provider
network calls while holding preference CAS locks; resolve or provision durable
target authority first, then write the preference with the scoped compare
token. If that preference write conflicts during DM provisioning, re-read the
current preference. If the saved target already matches the provisioned target
id, treat the operation as success; otherwise retry with the new compare token
or surface the conflict through the normal preference-write path.

Slack DM send-time validation should treat `missing_scope`, `user_not_found`,
`user_disabled`, `user_not_visible`, Slack Connect `invalid_user_combination`,
and `channel_not_found` as unavailable/stale target outcomes. These outcomes
fail closed and do not retry indefinitely.

### 4. WebUI v2 Routes

Add route descriptors, handlers, and router mounts:

- `GET /api/webchat/v2/outbound/preferences`
- `PUT /api/webchat/v2/outbound/preferences`
- `GET /api/webchat/v2/outbound/targets`

Handlers must depend only on `RebornServicesApi`, matching existing WebUI v2
boundaries. Invalid target IDs and malformed bodies should fail before any
preference write. State-changing browser routes must enforce the existing WebUI
v2 CSRF/origin guard before facade mutation. Target validation failures exposed
to clients should collapse non-owned, unauthorized, missing, stale, and
capability-mismatched targets into a single product-safe unavailable/rejected
shape.

### 5. Automations Delivery Panel

Add a standalone Delivery panel under Automations:

- load scoped preferences and targets with React Query
- show the current default target for the active agent/run ownership scope
- allow authorized selection or clearing
- after Slack pairing succeeds, offer a one-click choice to make the paired
  Slack DM the personal default for triggered automation delivery
- show the paired Slack DM as a selectable personal target once durable DM
  authority exists
- show admin-managed Slack channel routes as selectable shared-agent targets
  for authorized operators
- show empty state when no eligible delivery targets exist
- show stale target state when the saved default can no longer be resolved
- keep the existing automations list/table unchanged
- do not read, write, or imply per-automation delivery overrides in this phase

Expected WebUI starting points:

- `crates/ironclaw_webui_v2_static/static/js/lib/api.js`
- `crates/ironclaw_webui_v2_static/static/js/lib/api.test.mjs`
- `crates/ironclaw_webui_v2_static/static/js/pages/automations/automations-page.js`
- new `pages/automations/hooks/useDeliveryDefaults.js`
- new `pages/automations/components/delivery-defaults-panel.js`
- `crates/ironclaw_webui_v2_static/static/js/i18n/en.js`
- `crates/ironclaw_webui_v2_static/src/assets.rs`

### 6. Trigger Completion Delivery

Wire trigger terminal completion to the product outbound path:

- construct `RunNotificationOrigin::Triggered` for trigger-origin final replies
- construct `TriggeredFromSourceRoute` when a trigger run has a live source
  route and verify source-route precedence
- resolve scoped defaults with `OutboundResolutionEngine`
- validate with `OutboundPolicyService`
- render only through ready Reborn product-adapter outbound paths

Do not use WebUI projections as a delivery substitute. Do not let adapter code
load preferences or construct validated targets.

## Tests And Acceptance Criteria

### Scoped Defaults

- personal scope reads/writes/clears final target
- shared-agent scope reads/writes/clears final target
- legacy user-scoped preference rows read back as personal scoped defaults
- personal users cannot mutate shared-agent defaults without authorization
- missing default fails closed
- clearing target preserves non-final slots
- tenant/user/agent/project scoping does not leak
- scope lookup uses persisted run/agent ownership rather than trigger creator,
  last editor, last inbound sender, or synthetic user id
- CAS conflict/retry coverage proves scoped preference updates preserve
  concurrent writes and reject unsupported CAS safely
- stale same-slot writes return conflict through the product facade and HTTP
  route; disjoint-slot merges preserve both updates

### Target Authority

- valid target writes default
- invalid target rejects mutation
- stale target surfaces unavailable state
- deleted route fails closed
- owner change fails closed
- direct target lookup resolves one `(scope, target_id)` without full inventory
  enumeration
- client responses do not distinguish non-owned, unauthorized, missing, stale,
  or capability-mismatched target reasons
- same resolver path supports write-time and send-time validation
- product facade/preference-write tests drive a fake provider/resolver and prove
  invalid, stale, cross-scope, and capability-mismatched targets are rejected
- trigger completion tests drive send-time revalidation and prove failure happens
  before adapter render

### WebUI v2

- route descriptors include the three outbound routes
- handlers dispatch only through `RebornServicesApi`
- malformed body returns 400 before facade mutation
- state-changing browser routes reject missing or invalid CSRF/origin evidence
  before facade mutation
- facade errors map to HTTP status consistently
- GET preferences returns a version or ETag and PUT/clear requires the expected
  version or ETag when the route exposes client-managed updates
- shared-agent mutations require admin/operator authorization
- GET preferences, PUT preferences, and GET targets derive or authorize scope
  from the authenticated caller before facade calls
- spoofed `tenant_id`, `user_id`, `agent_id`, or `project_id` inputs cannot
  enumerate, read, or mutate another personal or shared-agent scope

### Static WebUI

- API tests cover GET/PUT paths and JSON body
- Delivery panel renders loading, empty, error, selected, saved, clearing, and
  stale-target states
- personal and shared-agent copy/states are distinct
- asset embedding test covers the new Automations Delivery panel

### Slack

- shared Slack channel routes list as shared-agent targets only for authorized
  scopes
- personal Slack DM target lists only after user pairing/DM authority exists
- successful Slack pairing can offer, but does not automatically apply, a
  follow-up action to make Slack DM the personal default for triggered
  automation delivery
- accepting the post-pairing default prompt provisions or validates the DM
  target and saves only the provider-issued `target_id`
- personal Slack DM target provisioning is explicit, idempotent, durable, and
  does not run inside preference CAS locks
- concurrent personal Slack DM target provisioning for the same tenant/user is
  idempotent across processes and does not create duplicate provider authority
- Slack DM provisioning calls `conversations.open` for the paired Slack user
  and persists the returned `D...` conversation id before default selection
- Slack DM sends use `chat.postMessage` with the validated `D...` conversation
  id, not a raw client-submitted Slack user id
- raw client-supplied Slack team/channel/DM ids cannot become saved defaults
- generic Slack selections and implicit last-conversation defaults cannot become
  saved defaults
- pairing-code redemption never writes default targets
- Slack final reply delivery still sends `chat.postMessage`
- progress/projection payloads remain deferred

### Trigger Delivery

- personal trigger final reply loads personal default
- shared-agent trigger final reply loads shared-agent default
- trigger final reply fails closed when run/agent ownership is missing,
  ambiguous, or inconsistent before target resolution
- missing default target fails closed and sends nothing
- revoked/invalid target fails validation and sends nothing
- valid Slack shared-channel target reaches adapter render and records Slack
  egress
- valid Slack personal DM target reaches adapter render and records Slack egress
- `TriggeredFromSourceRoute` preserves live source route precedence
- `OutboundPolicyService` is invoked before adapter render for personal,
  shared-agent, and source-route delivery
- outbound policy denial prevents Slack render and external egress

## Out-Of-Scope Follow-Ups

- Per-automation delivery overrides: future Automations delivery override phase.
- UI for progress, approval, and auth prompt targets: future modality expansion
  phase after final-reply E2E ships.
- Slack progress/projection delivery: future Slack delivery modality phase.
- Non-text modality defaults: future product API modality expansion phase.
- One-time immediate trigger creation: future trigger-create UX/API phase;
  current `trigger_create` remains
  recurring-cron focused.
- Accurate automation totals beyond the current capped list contract: future
  Automations list pagination/totals phase.

## Suggested Worktree

Use a clean worktree:

```bash
git fetch origin main
git worktree add -b trigger-delivery-scoped-defaults \
  ../trigger-delivery-scoped-defaults \
  origin/main
```

Run implementation and verification from that worktree. Do not reuse the dirty
main checkout for code changes.

If an older worktree is mid-rebase or conflict-marked, leave it untouched until
the clean `main` port is complete. Remove the whole obsolete worktree only after
confirming no useful comparison artifacts remain; do not delete individual files
inside it to make source-of-truth decisions.
