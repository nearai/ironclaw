# Slack gate feedback + delivered-route resolution (Reborn, slack-v2-host-beta)

Status: proposed
Date: 2026-06-10
Related: `docs/plans/2026-06-10-auth-gate-resume-redispatch.md` (separate workstream, `ironclaw_agent_loop` only — does not overlap)

## Problem

Verified against the working tree (see memory note `slack-push-approval-diagnosis`):

1. **Rejected resolutions are silent.** `dispatch_scoped_approval_resolution`
   rejections (`MissingGate`, `AmbiguousGate`, `BindingRequired`) become
   `ProductInboundAck::Rejected`; `SlackFinalReplyDeliveryObserver::observe_workflow_ack`
   skips them (`should_deliver_after_ack` → false,
   `slack_delivery.rs:146-151`) and delivery errors die as `warn!`
   (`slack_delivery.rs:732-747`). The user sees nothing.
2. **Bare `approve` in the prompt's thread never resolves.** All three
   resolution dispatchers (`dispatch_approval_resolution`,
   `dispatch_scoped_approval_resolution`, `dispatch_auth_resolution` —
   `workflow.rs:538-660`) call `lookup_interaction_binding` first. The
   approval prompt is posted as a new top-level channel message, so its
   thread has no conversation binding; the direct-base fallback is
   Direct-route-only (`workflow.rs:368-371`). Even when a binding exists,
   bare approve uses `list_pending` on the reply scope while the gate
   lives on the run's scope.
3. **`DeliveredGateRouteRecord` is only written on the triggered-run
   delivery path** (`slack_delivery.rs:1202-1237`). The live observer's
   gate-prompt notifications set `gate_ref_for_routing: None`
   (`slack_delivery.rs:249,265,292`), so live-channel prompts have no
   route record and even explicit `approve gate:<ref>` fails outside the
   run's own bound conversation.

## Design principles

- **Adapter-agnostic core.** The fix must work for any future product
  adapter (Telegram, Discord, …), not just Slack. Generic pieces land in
  `ironclaw_product_adapters`, `ironclaw_outbound`, and
  `ironclaw_product_workflow`. Slack-specific pieces are confined to two
  touchpoints every adapter will have anyway: (a) render rejected-ack
  feedback, (b) record the conversation ref a gate prompt was delivered
  into.
- **Routing is not authorization.** Route records redirect *where* a
  resolution is attempted; the existing `ApprovalInteractionService` /
  `AuthInteractionService` + `ApprovalResolutionPort` remain the only
  authority on *whether* the actor may resolve. Same stance as the
  existing `DeliveredGateRoutingApprovalService` (user-mismatch forwards
  unchanged).
- **Channel-edge error rule.** Feedback text comes from a canonical
  enum-derived hint (`.claude/rules/error-handling.md`); the
  `RedactedString` reason is never echoed.

## Phase A — rejection + delivery-error feedback

### A1. Canonical user-facing hints (generic)

`crates/ironclaw_product_adapters/src/inbound.rs`:

- `impl ProductRejectionKind { pub fn user_facing_hint(&self) -> &'static str }`
  - `BindingRequired` → "I couldn't match this reply to an active conversation. Reply in the approval thread, or use `approve gate:<ref>`."
  - `AccessDenied` → "You don't have access to resolve this request."
  - `UnknownInstallation` → "This workspace isn't set up with IronClaw yet."
  - `InvalidRequest` → "I couldn't read that request. Use `approve` / `deny`, optionally with `gate:<ref>`."
  - `PolicyDenied` → "That request was declined by policy."
- Exact copy reviewed at impl time; per-kind, no interpolation of
  internal state. Enum method, not stringly (`.claude/rules/types.md`).

### A2. Slack rejected-ack feedback

`crates/ironclaw_reborn_composition/src/slack_delivery.rs`, in
`observe_workflow_ack` (before the `deliver_final_reply` call or inside
it ahead of `should_deliver_after_ack`):

- If `ack` is `Rejected(rejection)` **and** the envelope payload is a
  resolution attempt (`ApprovalResolution`, `ScopedApprovalResolution`,
  `AuthResolution`) → `post_slack_message(egress, envelope.external_conversation_ref(), rejection.kind.user_facing_hint())`.
- Scope deliberately limited to resolution payloads: the user explicitly
  typed approve/deny, so a reply is expected and bounded. Rejected
  *user messages* (policy denials) are a follow-up — different noise
  profile in shared channels.
- `Duplicate { prior }` acks: unwrap and treat as the prior ack for the
  feedback decision (a re-sent approve that already succeeded should not
  produce an error message).

### A3. Delivery-error feedback

In `observe_workflow_ack`'s error branch (`slack_delivery.rs:740-746`),
keep the `warn!` and add a best-effort post:

- `RunWaitTimedOut` → "Still working on this — I'll post the result here when it finishes. Check the WebUI for live status."
  (Honest: with the current observer the result will NOT auto-post after
  timeout. Until Phase C re-arm lands, copy should be "This is taking
  longer than expected — check the WebUI for the result.")
- Any other `SlackFinalReplyDeliveryError` → "Something went wrong delivering the result here. Check the WebUI."
- Feedback post failures: `debug!` only — never recurse.

### A4. Tests

Per `.claude/rules/testing.md`, drive the caller:
`slack_serve/handler_tests.rs`-style tests through
`observe_workflow_ack` with a fake egress, asserting (a) rejected
scoped-approval ack posts the `BindingRequired`/`MissingGate` hint into
the envelope's conversation, (b) rejected non-resolution payload posts
nothing, (c) duplicate-wrapping-accepted posts nothing, (d) timeout
error posts the notice. Unit test the kind→hint mapping for exhaustive
`match` (new variant = compile error anyway).

## Phase B — delivered-route resolution for prompt threads

### B1. Conversation-keyed route records (generic)

`crates/ironclaw_outbound/src/delivered_gate_routes.rs`:

- Add dependency `ironclaw_conversations` (its `ExternalConversationRef`,
  `ids.rs:71`, is the canonical low-level type; do NOT mirror the
  product-adapter copy with a stringly struct).
- `ExternalConversationRef` currently derives `Serialize` only. Add a
  validated `Deserialize` (custom impl or raw-mirror `TryFrom` routed
  through `new()`'s `validate_external_id` checks, per
  `.claude/rules/types.md`) so the record round-trips through the
  filesystem store. Check first whether Serialize-only was a deliberate
  boundary decision; if so, persist a dedicated
  `DeliveredConversationKey` newtype in `ironclaw_outbound` constructed
  from the ref instead.
- `DeliveredGateRouteRecord` gains
  `delivered_conversation_ref: Option<ExternalConversationRef>` —
  `#[serde(default)]`, so existing filesystem records rehydrate
  (wire-stable rule).
- `DeliveredGateRouteStore` gains
  `load_delivered_gate_route_by_conversation(&TenantId, &ExternalConversationRef) -> Result<Option<DeliveredGateRouteRecord>, String>`.
- Implement on `InMemoryDeliveredGateRouteStore` (secondary map) and the
  filesystem store (`filesystem_store.rs` — secondary index file keyed
  by a hash of the conversation ref, same scope dir). Extend the store
  round-trip tests to cover the new lookup, TTL expiry, and
  default-on-missing-field rehydration.

### B2. Record routes on the live delivery path (Slack)

`crates/ironclaw_reborn_composition/src/slack_delivery.rs`:

- `notification_for_actionable_state`: set
  `gate_ref_for_routing: Some(gate_ref)` in the `BlockedApproval` branch
  and `Some(auth gate ref)` in the `BlockedAuth` branch (today all
  `None`).
- After `deliver_run_notification` returns `posted_messages`
  (`slack_delivery.rs:195-204`), when the event kind is
  `ApprovalNeeded`/`AuthRequired` and a gate ref is present, write a
  `DeliveredGateRouteRecord` with:
  - identity key `(tenant, binding.actor_user_id, gate_ref)` — same as today,
  - `scope` = the run's `TurnScope`, `run_id`,
  - `delivered_conversation_ref` = the prompt's own thread:
    `(space = envelope team, conversation = posted.channel, topic = posted.ts)`.
  - Also record the envelope's originating conversation ref (base, and
    its thread if the inbound message was already threaded) as
    additional route rows, so replies in the *original* thread resolve
    too. (Multiple rows, same gate ref — conversation-keyed lookup stays
    unambiguous because each row has one conversation key.)
- Extract the record+sweep block shared with the triggered path
  (`slack_delivery.rs:1202-1237`) into one helper — otherwise this is
  the duplicate-dispatch smell (`.claude/rules/architecture.md` #4).
- Best-effort semantics preserved: route-write failure never fails
  delivery; with Phase A, the degraded path is now *visible* (user gets
  the `gate:<ref>` hint on MissingGate).

### B3. Workflow fallback (generic — this is what future adapters inherit)

`crates/ironclaw_product_workflow/src/workflow.rs` (crate already
depends on `ironclaw_outbound` and `ironclaw_conversations`):

- `DispatchPorts` gains `delivered_gate_routes: &dyn DeliveredGateRouteStore`
  (required port; composition wires the real store, tests wire in-memory —
  no `Option<Arc<…>>`, per architecture rule #2).
- New helper `resolve_via_delivered_route(envelope, ports, decision/payload)`:
  1. Resolve the **actor + tenant** through the existing binding service
     against the topic-stripped base conversation ref FIRST (the channel
     itself is bound — the run originated there; for DMs the Direct
     fallback already works). The route record never supplies the actor,
     and tenant is not knowable before this step (`BindingRequired`
     means the threaded ref had no binding). If no base binding exists →
     fall through to the original rejection.
  2. `load_delivered_gate_route_by_conversation(tenant, envelope.external_conversation_ref())`
     (try exact ref first, then topic-stripped base ref).
  3. TTL check (existing `is_expired`).
  4. Call `approval_interaction_service.resolve` (or auth equivalent)
     with `scope = record.scope`, `gate_ref = record.gate_ref`
     (for the scoped/bare path) and `run_id_hint = Some(record.run_id)`.
- Wire into the three dispatchers:
  - `dispatch_scoped_approval_resolution`: on `BindingRequired` from
    `lookup_interaction_binding` **or** `MissingGate` from the
    single-pending match → try the fallback; if it misses, return the
    original rejection.
  - `dispatch_approval_resolution` / `dispatch_auth_resolution`
    (explicit ref): on `BindingRequired` only → fallback supplies scope;
    the explicit gate ref must equal the record's gate ref (mismatch →
    original rejection).
- `DeliveredGateRoutingApprovalService` (composition wrapper) stays as
  is — it still covers the DM-with-explicit-ref case where a binding
  *does* resolve but to the wrong scope. `list_pending` remains
  unrewritten (its module-doc invariant holds).

### B4. Prompt copy

Update the approval/auth prompt text rendered by
`slack_approval_gate_prompt_view` (and auth equivalent): "Reply
`approve` or `deny` in this thread" becomes true after B2/B3; keep
"or `approve gate:<ref>` from anywhere" as the fallback instruction.

### B5. Tests

- Workflow-level (caller-driven, `test-support` fakes): bare `approve`
  envelope whose conversation ref matches a recorded route → resolved
  with the record's scope/run-id; expired record → `MissingGate`;
  unpaired actor → `BindingRequired`; explicit-ref mismatch → original
  rejection; no record → unchanged behavior (regression on today's DM
  paths).
- Composition-level: live observer posts approval prompt → route record
  exists keyed by the posted message's thread; reply in that thread
  resolves end-to-end (extend `slack_serve/handler_tests.rs`).
- Store contract tests for the new lookup on both impls.

## Phase C — follow-ups (out of scope here, tracked separately)

1. Observer re-arm after `RunWaitTimedOut` (or event-driven wakeup) so
   late results still post.
2. Post live-run gate prompts into the originating message's thread
   (`thread_ts` on the prompt post) — pure UX, reduces channel noise;
   B already makes correctness independent of where the prompt lands.
3. Slack DM pairing writes `CommunicationPreferenceRecord` (triggered
   push default).
4. Decide top-level channel `approve` semantics (currently noop by
   design at `payload.rs:147-150`).
5. Per-gate conversation routing for concurrent gates in one DM. The
   conversation index is keyed by conversation fingerprint alone, so
   when two runs deliver gate prompts into the same DM conversation the
   second route overwrites the first — a bare "approve" then resolves
   only the most recently delivered gate. Known limitation in B:
   acceptable because the bare-reply fallback is a convenience path
   (explicit gate refs and WebUI still resolve either gate), but
   resolving it needs the index to fan out per gate and the fallback to
   surface an AmbiguousGate-style disambiguation prompt.

## Soundness evaluation

| Concern | Assessment |
|---|---|
| Authorization bypass | None added. Route records redirect scope only; actor always comes from the binding/pairing services and the inner interaction services authorize. Mirrors the existing wrapper's user-mismatch-forwards-unchanged stance. |
| Replay/staleness | 48h TTL + opportunistic sweep already exist; fallback re-checks `is_expired`. Idempotency keys (`approval_resolution_idempotency_key`) unchanged. |
| Ambiguity | Conversation-keyed lookup is one-record-per-thread; no `AmbiguousGate` regression. Multiple gates in one run posting to the same thread overwrite — last prompt wins, acceptable because only one gate blocks a run at a time. |
| Regression on working flows | Fallback fires only after today's paths fail (`BindingRequired`/`MissingGate`), so DM explicit-ref and WebUI flows are untouched. `list_pending` invariant preserved. |
| Noise | Phase A replies only to explicit resolution attempts and delivery failures — bounded, expected by the user. |
| Channel-edge errors | All copy from `user_facing_hint()`; `RedactedString` never rendered. |
| Both-backend persistence | `DeliveredGateRouteStore` impls (in-memory + filesystem) both extended; serde `default` keeps old records loadable. Reborn does not use the v1 PG/libSQL pair for this store. |
| Failure degradation | Route write is best-effort; on miss the user now gets an actionable hint instead of silence. |
| Future adapters | Generic pieces: hint method (product_adapters), conversation-keyed store (outbound), workflow fallback (product_workflow). A new adapter only (a) renders `Rejected` ack feedback in its observer, (b) records the delivered conversation ref when it posts a gate prompt. No Slack types in any generic seam. |

Residual risks / open questions:

- **Shared-channel approval authority.** In admin-routed shared
  channels, anyone paired in the workspace could reply `approve` in the
  prompt thread; authorization rests on the inner approval service's
  actor check. Confirm desired policy (gate owner only vs. any
  channel-routed actor) before B lands — it's a policy decision, not a
  routing one.
- **Timeout copy honesty (A3).** Until C1 lands, do not promise "I'll
  post the result here." Use the WebUI-pointing copy.
- **Filesystem secondary index.** Hash-keyed index file must handle
  record overwrite (same identity key, new conversation) without
  leaking stale conversation→gate mappings; sweep covers TTL but
  overwrite must remove the old index row.

## Execution order

A1 → A2/A3/A4 (ship first; makes everything else debuggable in prod) →
B1 → B2 → B3 → B4/B5. A and B are independently shippable PRs.
