# Channel Delivery Tool — Explicit Delivery, Two Lanes, No Heuristics

- **Date:** 2026-07-27
- **Status:** Approved design (brainstorm complete; implementation plan next)
- **Problem owner:** Ben Kurrek (product decisions with Firat, 2026-07-27)
- **Supersedes:** the `pushy-today` branch approach
  (`2026-07-24-delivery-destination-routing-design.md` — harden seal-one-destination
  routing + mechanical self-send guard). That branch is a salvage reference only.
- **Code references:** verified against `main` (`aa8b748c3`) on 2026-07-27;
  re-verify at implementation time. The extension-host seam moved out of
  composition on 2026-07-24 (#6669, #6616) — do not trust older path citations.

## 1. Problem and product model

Getting content to the user on a connected channel is implicit today: the model
seals a destination (`route_current`), or a trigger carries a stored
`delivery_target_id`, or fire-time falls back through a preference-slot chain —
and the host pushes the finalized reply after the turn ends. The model can
never observe the outcome, "send me X on slack" lexically attracts the
act-as-user `slack.send_message`, multi-destination is inexpressible, and
conditional delivery ("only ping me if X") is impossible because the push is
unconditional.

The new model is **two lanes**:

- **Lane 1 — conversation lifecycle (not delivery).** Every run's final reply
  lands in the conversation it belongs to, automatically, always: a
  channel-origin run replies in its channel conversation, a WebUI run renders
  in its thread, an automation fire's reply lands in that fire's own thread.
  Never sealed, never redirected, never suppressed. This is the natural
  lifecycle of agent turns.
- **Lane 2 — explicit delivery.** Reaching any *other* surface is an explicit,
  model-visible act: **`builtin.outbound_deliver`**, called mid-run under the
  channel's **bot identity**, any number of times per run, one catalog target
  per call, synchronously through the `DeliveryCoordinator`, returning
  provider-issued evidence the model can honestly reference in its reply.
  Automations deliver results the same way, from their fire prompts.

## 2. Decisions (locked)

1. **An explicit delivery tool, not implicit rerouting.** The affordance gap —
   "no way for the model to pick a tool to send from channel" — is fixed with a
   first-class tool. Vendor send tools are not rerouted or self-send-denied.
2. **Bot-send is the DEFAULT for "send it to me", not a ban on self-sends.**
   Guidance and the tool's existence make bot delivery the unambiguous default;
   the vendor tools' approval gates remain the human backstop. No hard runtime
   deny of vendor self-sends (room for a future user-changeable setting).
3. **Final reply always returns to the origin conversation automatically.**
   No suppression or dedup logic against lane 2. The only mechanical guard is
   the tool soft-denying a target that IS the run's own origin conversation
   (an identity check on sealed binding refs, not intent parsing).
4. **Automations ride the same tool.** `delivery_target_id`, create-time route
   inheritance, the fire-time precedence chain, and the unconditional result
   push are deleted. Fire prompts state delivery steps explicitly.
5. **Synchronous, evidence-bearing calls.** One target per call;
   multi-destination is multiple calls. No enqueue/post-turn settling: in-run
   evidence is the point. No "queued" result state exists in v1.
6. **Notifications for background runs survive as one explicit setting:** a
   user-configured **set** of notification channels (0..N catalog targets,
   empty ⇒ web-app only) receiving gate/auth prompts and failure notices.
   All other roles of the communication-preference machinery are deleted.
7. **Gating:** `PermissionMode::Allow` + `origin_gate_matrix.loop_run =
   GatedUnlessGranted` — identical to `route_current` today; the reviewed
   17-id ungated seed is untouched. Under strict approval profiles the first
   delivery gates once; durable "always allow" then covers interactive runs
   and scheduled fires alike. Revisit (seed addition) only if first-fire
   gating proves hostile in practice.
8. **v1 addressing = what the catalog has:** personal DM targets + operator-
   registered shared channels. Self-serve group discovery is a named follow-up
   (§15), and the tool contract does not change when it lands.

## 3. Behavior matrix (normative)

| Scenario | Lane 2 (tool, bot identity) | Lane 1 (final reply) |
|---|---|---|
| WebUI: "summarize my inbox" | — | WebUI thread |
| WebUI: "send me the summary on Slack" | deliver → Slack bot DM, evidence back | short honest ack in thread |
| WebUI: "send to Telegram AND the team Slack channel" | two calls, per-call evidence | ack summarizing both |
| WebUI: "email me the report" | impossible — no channel surface, no target | honest refusal + available channels |
| Slack DM: "what's on my calendar" | — | auto-echo in same Slack conversation |
| Slack DM: "send this to my Telegram too" | deliver → Telegram bot DM | ack echoes in Slack |
| Slack DM: "send it to me here" | soft-denied (origin conversation); model just replies | content IS the reply |
| "post the standup to #eng-updates" (registered shared channel) | deliver → bot posts there | ack at origin |
| "send to my family group" (unregistered) | honest refusal + available targets (v1) | refusal at origin |
| "Slack-message Firat the notes" | NOT this tool — vendor `slack.send_message` as the user, approval-gated | ack at origin |
| Routine: "every morning send me news on Telegram" | fire prompt names the step; each fire delivers via tool | digest in the fire's thread |
| Routine created from Slack, no channel named | create-time model writes "deliver to my Slack DM" into the prompt (visible, replaces hidden inheritance) | fire thread |
| Routine: "ping me on Slack only if CI breaks" | green: no call, nothing external; red: deliver | report in fire thread either way |
| Fire blocks on approval gate | host notification → notification channels; approve from any; second approve "already resolved" | run parked; panel hold badge |
| Fire blocks on expired credential | auth prompt → DM-capable notification targets only | parked → resumes after re-auth |
| Fire fails | failure notice → notification channels | failure in run history |
| Nothing configured | notifications land in web app only (badges, gate UI) | — |

## 4. Current state (verified anchors)

**The coordinator (stays, gains one intent).**
`crates/ironclaw_product/src/delivery_coordinator.rs` — nine semantic
`DeliveryIntent`s split policy-class (`FinalReply`, `GatePrompt`, `AuthPrompt`,
`TriggeredDelivery` → `deliver()`, outbound-policy validated) vs notice-class
(source-routed `deliver_notice()`). Sole delivery-state writer; attempt
persisted `Prepared→Sending` before egress (OUT-3); crash recovery marks
`Sending→Unknown`, never blind-resends (OUT-6); partial-multipart is terminal
(OUT-7); `Delivered { vendor_message_refs, conversation, .. }` carries
provider evidence. No no-op-sink constructor (OUT-4).

**Run-delivery drivers.** `crates/ironclaw_product/src/run_delivery.rs` —
`RunDeliveryObserver` (channel-origin echo; lane 1, stays) and
`TriggeredRunDeliveryDriver` (`run_delivery/triggered.rs`): state→intent map at
`triggered_notification_for_state` — `Completed` → `TriggeredDelivery` result
push + footer (**dies**); `BlockedApproval` → `GatePrompt`; `BlockedAuth` with
URL → `AuthPrompt` (DM-only, enforced at send); `BlockedAuth` manual-token →
cancels run + notice (all three notification arms **stay**, retargeted §7).

**The heuristic pile (dies).**
- `crates/ironclaw_extension_host/src/channel_triggered_delivery.rs`:
  `resolve_per_trigger_target` (re-resolves stored id via creator-scoped
  registry), `route_extension` (per-trigger → stored preference → *sole active
  extension guess* → "ambiguous" error), `stored_preference_target` (personal
  slot chain `final_reply → approval → auth → progress`).
- `crates/ironclaw_outbound/src/resolution_engine.rs` per-kind slot fallbacks;
  missing → `PreferenceTargetMissing` → blocked fires silently undeliverable.
- Per-trigger target: `TriggerRecord.delivery_target` + `TriggerFire.delivery_target`
  (`crates/ironclaw_triggers/src/lib.rs`), `delivery_target_id` on
  `builtin.trigger_create` (`crates/ironclaw_host_runtime/src/first_party_tools/trigger_management.rs`),
  create-time sealed-route inheritance
  (`crates/ironclaw_reborn_composition/src/factory.rs`,
  `LocalRuntimeTriggerCreatorPairingHook::resolve_current_run_delivery_target`
  reading `run_state.reply_target_binding_ref`).
- Per-run seal: `builtin.outbound_delivery_target_route_current`
  (`crates/ironclaw_host_runtime/src/first_party_tools/outbound_delivery.rs`) —
  note its production router is **never wired** on main: only
  `UnavailableRunFinalReplyRouter` is registered and
  `register_outbound_delivery_first_party_handler` has zero callers. The
  supporting records (`RunFinalReplyTargetRecord`, `RunFinalReplyHandoffRecord`,
  `crates/ironclaw_outbound/src/run_final_reply_target.rs`,
  `run_final_reply_handoff.rs`, store methods) are production-unconsumed.
- User default: `CommunicationPreferenceRecord` four target slots
  (`crates/ironclaw_outbound/src/communication_preferences.rs`); only
  `final_reply_target` is ever written
  (`crates/ironclaw_product/src/reborn_services/outbound_preferences.rs`;
  picking web-app *clears* it). Written by the automations-page panel and
  `builtin.outbound_delivery_target_set`.
- Prompt heuristics: delivery-target lines + ScheduledTrigger no-target warning
  (`crates/ironclaw_turns/src/run_profile/runtime_context.rs`), trigger-create
  description + prompt-field warnings, `routine-advisor` bundled skill, Slack
  `send_message.md` "host delivers after the turn" wording — all pinned by
  verbatim substring tests (`trigger_management/tests.rs`,
  `tool_surface_contract.rs`, `bundled_skills.rs`, `runtime_context.rs` tests).

**The catalog (stays; becomes the tool's vocabulary).**
`crates/ironclaw_outbound/src/delivery_targets.rs` —
`OutboundDeliveryTargetEntry { summary { target_id, channel, display_name,
description }, capabilities, destination, owner }`, owner-scoped registry.
`crates/ironclaw_extension_host/src/channel_outbound_targets.rs` composes two
families generically: personal DMs (`{ext}:personal-dm:{space}:{user}`, from
`FilesystemChannelDmTargetStore`) and operator-registered shared channels
(`{ext}:shared-channel:{space}:{conversation}`, from `*_subject_routes`
admin config). `builtin:web_app` is a host-owned pseudo-target
(`crates/ironclaw_reborn_composition/src/factory.rs`) — **dies** (§11).
`ChannelAdapter::list_targets` is DM-provisioning-only today (Slack `im:`
lookup; Telegram unsupported) — untouched by v1.

**Gating machinery.** `crates/ironclaw_host_api/src/capability.rs` —
`OriginGateMatrix { loop_run, product, automation }`; `ScheduledTrigger` origin
maps to `ScheduledLoopRun` → the `loop_run` column; 17-id ungated seed pinned by
`crates/ironclaw_architecture/tests/reborn_origin_gate_matrix_ratchet.rs`;
`route_current` today is `PermissionMode::Allow` + `GatedUnlessGranted`
(`crates/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`, which also
freezes the exact 31-id builtin list).

**Structural pins to update, not violate.** `ironclaw_extension_host` owns
generic channel delivery (`crates/ironclaw_architecture/tests/telegram_extension_gates.rs`);
`ProductSurface` methods frozen to `invoke/query/stream_events`
(`reborn_service_method_freeze_ratchet.rs`) — everything here rides capability
descriptors; production struct dead-code ratchet (#6673) applies to new types.
`tests/integration/delivery_user_journeys.rs` is a retired stub on main, and
`tests/reborn_trace_first_party_tool_coverage.rs` carries a stale claim that
live journeys cover `route_current` — this project rebuilds that suite.

## 5. The tool contract — `builtin.outbound_deliver`

- **Input** (`deny_unknown_fields`):
  - `target_id` — exact opaque `OutboundDeliveryTargetId` from
    `builtin__outbound_delivery_targets_list`.
  - `content` — markdown text, schema-bounded (`maxLength` 32768). Rendering,
    splitting, and vendor formatting stay behind `ChannelAdapter::deliver`.
- **Success output:** `{ delivered: true, target_id, channel, display_name,
  provider_message_refs: [...] }` — taken from the coordinator's `Delivered`
  outcome. Never claims what the vendor did not confirm
  (`tool-evidence.md`: outbound send returns provider identifiers; there is no
  queued state in v1). A `display_preview` summary ("Delivered to {display_name}")
  rides the existing preview channel.
- **Model-visible failures** (run continues; model adapts or reports honestly):
  - unknown/foreign/revoked `target_id` → invalid-input issue on `target_id`
    (same shape as today's `invalid_target_input`);
  - target == this run's origin conversation → **Denied** with recovery text
    "your reply already lands in this conversation — just reply";
  - policy `Rejected` → Denied; adapter `Unauthorized` / terminal transport
    failure → recoverable Failed with the sanitized `DeliveryFailureKind`.
  Only genuine host faults (store down) end the run.
- **Multiplicity/idempotency:** each invocation is one durable delivery attempt
  (attempt `projection_ref` derived from the invocation id), single-flight,
  bounded coordinator retries inside the call, OUT-7 unchanged. No
  caller-supplied idempotency key in v1. Per-run call cap: 16 deliveries
  (counted per scope like the post-edit-check seen-set); calls past the cap are
  Denied with the cap named.
- **Permission/origin:** effects `[DispatchCapability, ExternalWrite]`,
  `PermissionMode::Allow`, matrix `loop_run: GatedUnlessGranted`,
  `product/automation: Forbidden` (decision 7).
- **Naming:** the builtin id list stays at 31 — `outbound_deliver` replaces
  `outbound_delivery_target_route_current`; `notification_channels_set`
  replaces `outbound_delivery_target_set`; `outbound_delivery_targets_list`
  is unchanged.

## 6. Delivery path architecture

The tool follows the exact seam pattern of the tool it replaces:

1. **Handler** in `crates/ironclaw_host_runtime/src/first_party_tools/`
   (one file per capability), holding an `Arc<dyn ModelChannelDelivery>` port.
2. **Port** defined in `crates/ironclaw_outbound` (domain contract owner, like
   `RouteCurrentRunFinalReply` today): request `{ scope, run_id,
   authenticated_actor_user_id, target_id, content }` → typed outcome carrying
   the evidence or a redacted failure class.
3. **Implementation** in `crates/ironclaw_extension_host` (the
   architecture-pinned home of generic channel delivery): resolves the target
   through the caller-scoped registry, performs the same-origin check by
   comparing the entry's sealed `reply_target_binding_ref` against the run's
   own `reply_target_binding_ref` from trusted run state (the same lookup the
   retired inheritance hook used), builds the candidate, and drives
   `DeliveryCoordinator::deliver` with a new **policy-class intent
   `DeliveryIntent::ModelDelivery`** (`as_str`: `model-delivery`).
4. **Composition** injects the implementation via a register fn (replacing the
   never-called `register_outbound_delivery_first_party_handler`).

The coordinator is byte-for-byte the same machine: sole writer, attempt before
egress, generation-pinned adapter resolution, bounded retries, evidence out.
"No direct product send path" remains true because the tool IS the coordinator
path — there is no second pipeline (`architecture.md` §4).

Catalog semantics: resolution for the tool uses the same owner-scoped registry;
the `final_replies` capability bit is read as "content deliveries" (both DM and
shared entries set it today; the field keeps its name in v1 with an updated
doc-comment — renaming it is not worth the wire churn). `builtin:web_app` is removed — lane 1 already
owns the web app, and an empty notification set expresses "app only".

## 7. Notification channels

- **Record:** `CommunicationPreferenceRecord`'s four slots are replaced by
  `notification_targets: Vec<OutboundDeliveryTargetId>` (bounded, cap 8,
  deduplicated, each validated through the owner-scoped registry at write
  time). Read-time migration: a legacy record with `final_reply_target = t`
  reads as `notification_targets = [t]` (preserves today's de-facto
  notification behavior); legacy fields stay deserializable and ignored;
  next write persists the new shape via the existing CAS path.
- **Fan-out:** the background-run watcher (`TriggeredRunDeliveryDriver`,
  renamed in role to background-run notifier) delivers `GatePrompt` /
  `AuthPrompt` / `FailureNotice` as one coordinator attempt per configured
  target. Auth prompts (OAuth URLs) go only to personal-DM targets; non-DM
  targets get a redacted "a routine needs re-authorization — open the app"
  notice; if no DM-capable target exists the prompt lives in the web app and
  the run stays parked (no cancel). Manual-token auth keeps today's
  cancel+notice behavior. Empty set ⇒ no external attempts; the automations
  panel hold badge and gate UI are the surface (matches today's no-default
  reality).
- **Cross-channel resolution:** `DeliveredGateRouteStore` already records every
  conversation a prompt landed in, so a bare "approve" reply resolves from any
  notification channel; gate resolution is one-shot (reserve-or-wait), a
  second approve reads "already resolved".
- **Surfaces:** the automations-page delivery-defaults panel becomes a
  "Notification channels" multi-select over the same catalog (wire DTO becomes
  list-valued); `builtin.notification_channels_set` (list of target ids, full-replace
  CAS write, `PermissionMode::Ask`, approval-gated, `Exclusive` concurrency)
  replaces `target_set` for in-chat configuration.
- Interactive runs are untouched: channel-origin gate/auth prompts stay
  source-routed to the origin conversation; WebUI runs use the in-app gate UI.

## 8. Automations simplification

- `builtin.trigger_create` loses `delivery_target_id` (schema + input struct +
  output field). The create-time model writes delivery steps into the routine
  prompt explicitly — it knows which conversation it is being asked from, so
  "send me the news every morning" asked from Slack yields a prompt naming the
  Slack DM. Visible text replaces hidden route inheritance.
- Deleted: the create-hook target resolution/validation/inheritance
  (`resolve_delivery_target`, `resolve_implicit_delivery_target`,
  `validate_delivery_target`, `LocalRuntimeTriggerCreatorPairingHook`'s
  delivery half), `TriggerFire.delivery_target` plumbing, and
  `channel_triggered_delivery.rs`'s whole routing block (`resolve_per_trigger_target`,
  `route_extension`, `stored_preference_target`); the hook's surviving job is
  wiring the notifier with the notification set.
- The `Completed` arm of `triggered_notification_for_state` stops delivering
  (no more result push, no footer); `TriggeredDelivery` remains only as a
  retired intent name to remove from `DeliveryIntent` (§11).
- Fires with no tool call deliver nothing externally — by design; the fire
  thread and run history are the record.
- **Existing-trigger migration (amended 2026-08-04, Task 12 ruling — see
  `.superpowers/sdd/2026-07-27-channel-delivery-tool/task-12-report.md`):** a
  one-time startup migration rewrites each trigger that has a stored
  `delivery_target`. The rule below **replaces** this section's original
  "resolvable → append with display name + clear; unresolvable → clear only"
  text, which Task 12's implementation falsified against a real boot (journey
  10): a boot-time non-resolution is common, not exceptional, and clearing the
  column on it would have silently and irreversibly dropped routing for every
  routine whose target had not resolved yet.

  **Landed rule: always write the delivery step, never clear-only.** For every
  record with a stored `delivery_target`: append
  "Deliver the result to {display_name} using builtin__outbound_deliver
  (target id: {id})." when the target resolves, or the same sentence with
  "the destination it was routed to" in place of the name when it does not
  (never a fabricated label — the id is preserved either way) — then clear the
  column. The one exception is **prompt-cap overflow**: if appending the step
  would push the prompt past the trigger prompt byte cap, the record is left
  **untouched** (route and prompt both intact) with a loud warn naming the
  trigger, rather than clearing the column with no replacement step.

  **Rationale:** a registry lookup returning "no such target" (`Ok(None)`) is
  **ambiguous** at boot time — it is the identical answer for a genuinely
  retired target, an extension whose activation failed (activation failures
  are tolerated-and-continue, so boot proceeds with that extension
  contributing no targets), a target mid-reconfiguration, and one not yet
  provisioned. Clearing the column is irreversible; keeping the id costs
  nothing. The asymmetry decides it. This is explicitly **not** a claim that
  extension activation is asynchronous relative to composition — boot
  activation is synchronous and awaited; the non-resolution Task 12 observed
  was a test-fixture property (a provider re-registered after boot 1 but not
  on later boots), not evidence of an ordering race. The always-write rule
  holds regardless of *why* a lookup returns nothing, which is exactly why it
  does not depend on fixing boot ordering.

  The DB column itself stays (read-tolerated, never written) — no destructive
  schema change in v1.

## 9. Guidance consolidation and vendor-tool posture

One authoritative **"Delivery"** block, a markdown asset in
`crates/ironclaw_turns/prompts/` loaded via `include_str!` (repo rule), rendered
whenever delivery tools are visible. It teaches: lane 1 (your reply already
lands where the conversation lives — never re-send it), lane 2 (the tool, when
the user wants content on another surface; the result is your evidence — report
it honestly), routines (write delivery steps into the prompt; no call means no
delivery), and the vendor boundary (act-as-user messaging tools reach other
people and places as the user; "send it to me" defaults to the delivery tool).

The scattered sites shrink to pointers, same change:

- `runtime_context.rs`: delivery-target lines → a one-line notification-channels
  line; the ScheduledTrigger no-target warning dies; the ScheduledTrigger origin
  line becomes "the final reply lands in this run's thread; deliver externally
  only via `builtin__outbound_deliver` as the prompt instructs"; the Inbound
  line keeps "replies post back automatically".
- `TRIGGER_CREATE_DESCRIPTION` + trigger `prompt` field description: rewritten
  around explicit prompt-authored delivery.
- `routine-advisor` bundled skill: rewritten (product skill — product behavior
  change, flagged per the two-skill-systems rule).
- Slack `send_message.md`: keeps act-as-user framing; the "host delivers after
  the turn completes" paragraph is replaced with a pointer to the delivery
  tool. No `recipient_argument` manifest field, no self-send deny (decision 2).
- Every verbatim substring test pinning old wording is updated in the same
  commits (`trigger_management/tests.rs`, `tool_surface_contract.rs`,
  `bundled_skills.rs`, `runtime_context.rs` tests, QA fixtures).

## 10. Law and docs

- **`docs/internal/reborn/extension-runtime/overview.md` §5.4 rewrite:** the outbound
  section gains model-initiated delivery as a first-class policy-class intent
  through the same coordinator. The sole-writer rule, attempt persistence, and
  crash semantics are restated to cover it; the boundary note becomes: the
  delivery tool is how the *model* delivers as the assistant (bot identity);
  vendor send tools remain the model acting as the user; final replies remain
  lane 1 and never ride either tool. §5.2's `slack.send_message` note and the
  emitter language ("emitters never know what channel" — now scoped to
  host-emitted intents) are updated coherently. `checklist.md`'s OUT items gain
  the model-delivery evidence requirement (provider refs in the tool result;
  queued-not-delivered must be typed — impossible in v1 by construction).
- **Port `tools.md` and `tool-evidence.md`** from `pushy-today` into
  `.claude/rules/` on main (fixing the stale "Everything Goes Through Tools"
  cross-reference in `pushy-today`'s `architecture.md` §4 rather than porting
  it verbatim).
- **Stale-doc rider** (from the salvage spec): root `CLAUDE.md` channel-section
  drift (`[channel.config]` → top-level `[admin_configuration]`, Slack tool
  count, `ChannelAdapter` path) and the same fixes in affected `.claude/skills/`
  files. `openwiki/` is regenerated, not hand-edited.

## 11. Deletions inventory

| Deleted | Where |
|---|---|
| `builtin.outbound_delivery_target_route_current` (tool, schemas, handler, `UnavailableRunFinalReplyRouter`, register fn) | `host_runtime/first_party_tools/` |
| `RouteCurrentRunFinalReply` port + `RunFinalReplyTargetRecord` + `RunFinalReplyHandoffRecord` + their store methods | `ironclaw_outbound` |
| `builtin.outbound_delivery_target_set` (replaced by `notification_channels_set`) | capability surface + local_dev handler |
| `builtin:web_app` pseudo-target + host-owned provider registration | composition factory |
| `TriggerRecord.delivery_target` write path, `TriggerFire.delivery_target`, `delivery_target_id` input/output, create-hook delivery half | triggers, host_runtime, composition |
| `resolve_per_trigger_target` / `route_extension` / `stored_preference_target` | extension_host `channel_triggered_delivery.rs` |
| `DeliveryIntent::TriggeredDelivery` + result push + footer | product `delivery_coordinator.rs`, `run_delivery/triggered.rs` |
| Four preference slots + per-kind slot fallbacks | outbound `communication_preferences.rs`, `resolution_engine.rs` |
| Delivery-target prompt lines + ScheduledTrigger warning + "delivered automatically" wording | turns `runtime_context.rs`, tool descriptions, routine-advisor, slack prompt doc |

`RunFinalReplyDestination` survives only as the catalog entry's sealed
destination shape, and every consumer of its `WebApp` variant dies with this
design (per-trigger resolution, the preference WebApp→None mapping, the
web_app provider) — so the `WebApp` variant is deleted too, and if that leaves
a single-variant enum the plan collapses the catalog destination to the sealed
binding ref directly.
Retired vocabulary (`outbound_delivery_target_route_current`,
`delivery_target_id`, `TriggeredDelivery`) is added to
`reborn_retired_taxonomy.rs` so the heuristics cannot creep back. The frozen
31-id list, origin-gate ratchet rows, coverage-claim list, and struct ratchet
are updated in the same PRs that change them.

## 12. Error handling

- Coordinator outcomes map to model-visible results per §5; every sanitized
  failure retains its server-side cause in logs/audit (`error-handling.md`).
- A notification fan-out failure on one target does not block the others; each
  attempt records independently; terminal failures are visible in the attempt
  ledger and (for fires) the run-history status.
- Preference-record CAS conflicts on `notification_channels_set` surface as the
  existing conflict error (retryable by the model).
- Migration failures (§8) leave the trigger untouched and log loudly; the fire
  path treats a still-present stored target as absent (write path is gone).

## 13. Testing (integration-first)

Rebuild `tests/integration/delivery_user_journeys.rs` from its retired stub
(the `pushy-today` version is the pattern donor for harness/journey structure;
its receipt/self-send-guard content is superseded). Scripted-trace journeys,
asserting at the wire-recorder and attempt-ledger seams (never
`wait_for_status(Completed)` alone):

1. WebUI → "send me X on Slack": tool call → bot credential on the wire,
   `Delivered` attempt, provider refs present in the tool result AND the
   attempt row; ack (not content) as the final reply.
2. Slack-origin → deliver to Telegram: content once in Telegram, ack echoed in
   Slack (lane 1), both attempts correctly attributed.
3. Same-origin soft-deny: deliver targeting the origin conversation → Denied
   with recovery text → model replies normally, zero duplicate sends.
4. Multi-call partial failure: two targets, one adapter failure → per-call
   honest results; failed call's attempt row `Failed{kind}`; model reports
   accurately (trace-pinned).
5. Routine fire delivers via tool with no `delivery_target_id` anywhere;
   conditional fire (no call) produces zero external attempts.
6. Blocked fire fan-out: two notification channels; gate prompt lands on both;
   approve from one resumes; second approve → already-resolved; auth prompt
   goes only to the DM-capable target.
7. Empty notification set: blocked fire produces no external attempts; hold
   badge visible via `builtin.trigger_list` / automations list.
8. Refusal: no-channel-surface ask ("email me…") → no tool call, no trigger,
   explanatory reply (scripted trace).
9. `notification_channels_set`: set/read-back through the service seam;
   legacy single-slot record migration (read-time and write-back).
10. Trigger migration: stored `delivery_target` → an explicit delivery step is
    always written into the prompt (resolvable targets by display name,
    unresolvable ones by raw id — never clear-only, per the §8 amendment) and
    the column cleared; the one exception is prompt-cap overflow, which leaves
    the record untouched with a loud warn.

Crate tier: coordinator `ModelDelivery` intent-class tests (extend
`outbound_delivery_contract.rs`), catalog resolution/capability filtering,
preference-record round-trip, schema/description contract updates, per-run cap.
Architecture tier: id list, origin-gate ratchet, retired-taxonomy additions,
dependency edges for the new port/impl. Recorded QA fixtures: tool-choice
traces for "send me X on Y" (interactive + fire). E2E: journey-coverage rows
per `test_journey_coverage.py` for the changed surfaces; Playwright coverage
for the notification-channels panel. Full gate: fmt, clippy both lanes,
`cargo test -p ironclaw_architecture`, integration tier, e2e manifest.

## 14. Rollout note

Single train on `main` (no feature flag: deployment shape ≠ `#[cfg]`, and the
old and new worlds cannot coexist coherently). The PR series lands
tool + notifier first, deletions + migration second, guidance/docs third —
each PR green on the full gate. (Slicing is the implementation plan's job.)

## 15. Non-goals (each with its revisit trigger)

| Excluded | Revisit when |
|---|---|
| Self-serve group discovery/registration (adapter conversation search, confirm-to-register flow) | first real demand to deliver to an unregistered group |
| Attachments in deliveries (envelope parts already support it) | a consumer needs bytes outbound |
| Standardized act-as-user messaging framework (`channel.messaging_capabilities`) | separate project, explicitly out (mission) |
| Hard deny / user setting for vendor self-sends | product decides to expose the identity choice |
| Progress streaming to channels | product asks for it |
| Caller-supplied idempotency keys on the tool | a real double-send incident the per-invocation attempt does not cover |
| Email-as-a-channel (would dissolve the Gmail refusal case) | separate product decision |
| Multi-replica delivery coordination | multi-replica serving ADR |
