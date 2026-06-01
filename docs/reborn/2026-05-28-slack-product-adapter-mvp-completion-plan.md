# Reborn Slack ProductAdapter MVP completion plan

**Status:** Design / execution plan — pending implementation
**Date:** 2026-05-28
**Branch scope:** `reborn-integration` — Slack ProductAdapter MVP, Reborn product-surface seams only
**Tracks:** [#3857 — Lane 10: add Slack ProductAdapter MVP with preconfigured credentials](https://github.com/nearai/ironclaw/issues/3857)
**Current slice:** [#4035 — feat(slack): add Reborn ProductAdapter core](https://github.com/nearai/ironclaw/pull/4035)
**Related Reborn lanes:** [#3280](https://github.com/nearai/ironclaw/issues/3280), [#4164](https://github.com/nearai/ironclaw/pull/4164), [#3281](https://github.com/nearai/ironclaw/issues/3281), [#3266](https://github.com/nearai/ironclaw/issues/3266), [#3094](https://github.com/nearai/ironclaw/issues/3094), [#3289](https://github.com/nearai/ironclaw/issues/3289), [#3279](https://github.com/nearai/ironclaw/issues/3279)

## 1. Purpose

[#3857](https://github.com/nearai/ironclaw/issues/3857) asks for a default-off Slack ProductAdapter MVP using preconfigured Slack app credentials. The MVP must verify Slack requests, support DMs and app mentions, route work through Reborn services, acknowledge Slack events immediately, deliver final replies asynchronously, and handle approval/auth-required states safely.

This plan explains why the whole issue cannot be honestly closed in one immediate PR without violating Reborn layering, while still identifying what can be completed now. It turns the remaining work into a short sequence of reviewable slices and makes active blockers visible through linked issues/PRs.

The key boundary is:

> Slack owns Slack protocol parsing/rendering and Slack conversation semantics; ProductWorkflow, outbound fanout, approval/auth interaction services, and acceptance harnesses own the cross-product behavior.

## 2. Current state

[#4035](https://github.com/nearai/ironclaw/pull/4035) is the current Slack adapter-core draft. The status snapshot below is as of **2026-05-28**; use the latest GitHub state when implementing follow-up slices.

- Open and still draft.
- Review-blocked with `CHANGES_REQUESTED`.
- Merge-blocked with `DIRTY` merge state against `reborn-integration`.
- Scoped to adapter-core / protocol-boundary work only.

It currently adds a native `ironclaw_slack_v2_adapter` crate that is intended to own:

- Slack Events API inbound normalization:
  - DM `message` events -> `ProductInboundPayload::UserMessage`.
  - `app_mention` events -> `ProductInboundPayload::UserMessage`.
  - bot/self/subtyped/unsupported events -> authenticated `NoOp`.
- Slack request-signature auth metadata declaration.
- Host-mediated Slack Web API egress declaration using a `slack_bot_token` handle.
- `FinalReplyView` -> `chat.postMessage` render contract.
- Adapter-local parse/render/boundary coverage.

[#4035](https://github.com/nearai/ironclaw/pull/4035) does **not** complete [#3857](https://github.com/nearai/ironclaw/issues/3857). It intentionally does not own host webhook registration, Slack signature verification, immediate ACK orchestration, ProductWorkflow submission, final-reply fanout, approval/auth interaction behavior, or fake Slack E2E acceptance.

## 3. Blockers

The issue's exit criteria span multiple Reborn lanes that are still open. Implementing all of them inside Slack now would require Slack-specific shortcuts such as direct runtime calls, Slack-local pending approval stores, or custom outbound fanout. Those shortcuts would conflict with the review direction on [#4035](https://github.com/nearai/ironclaw/pull/4035): no v1 reuse and no bypass of Reborn product surfaces.

| #3857 requirement | Current blocker | Why it blocks full completion now | Safe next step |
|---|---|---|---|
| Merge the Slack adapter-core slice | [#4035](https://github.com/nearai/ironclaw/pull/4035) open draft, `CHANGES_REQUESTED`, `DIRTY` as of 2026-05-28 | The first slice still needs review cleanup and conflict resolution before downstream host wiring can depend on it. | Finish #4035 as adapter-only core; do not expand it into host/runtime wiring. |
| Route Slack inbound work through ProductWorkflow | [#3280](https://github.com/nearai/ironclaw/issues/3280), [#4164](https://github.com/nearai/ironclaw/pull/4164), [#3885](https://github.com/nearai/ironclaw/pull/3885) | User-message routing exists, but ProductWorkflow routing/outcome shape is still being planned/reviewed for full product-surface behavior. Slack should not route around it. | Wire only to stable ProductWorkflow APIs; keep command/read/continuation assumptions out of Slack. |
| Deliver final AgentLoop replies asynchronously back to Slack | [#3281](https://github.com/nearai/ironclaw/issues/3281), [#3266](https://github.com/nearai/ironclaw/issues/3266) | Slack can render `chat.postMessage`, but durable projection fanout and outbound/subscription policy are owned by EventStreamManager/outbound lanes. | Keep Slack render contract in #4035; add fanout only after shared outbound policy stabilizes. |
| Support proactive Slack delivery / Slack as product capability | [#3281](https://github.com/nearai/ironclaw/issues/3281), [#3266](https://github.com/nearai/ironclaw/issues/3266) | Proactive send should be a generic outbound delivery capability, not a Slack-specific LLM tool or special runtime bypass. | Model Slack as a delivery backend behind shared outbound policy. |
| Conversational approval handling (`approve` / `deny`) | [#3094](https://github.com/nearai/ironclaw/issues/3094), [#3280](https://github.com/nearai/ironclaw/issues/3280) | Approval service foundations exist, but Slack must not maintain its own pending approval authority or directly resume turns. | Route Slack replies to approval interaction services once the product contract is stable. |
| Auth-required Slack UX | [#3289](https://github.com/nearai/ironclaw/issues/3289), [#3094](https://github.com/nearai/ironclaw/issues/3094) | OAuth/manual token/recovery foundations have landed, but product auth setup/recovery behavior remains open. Slack should not invent a parallel auth flow. | Post safe blocked/setup guidance only through the shared auth interaction contract. |
| Fake Slack E2E proving signed event -> Reborn -> async reply / approval | [#3279](https://github.com/nearai/ironclaw/issues/3279), plus [#3281](https://github.com/nearai/ironclaw/issues/3281) / [#3266](https://github.com/nearai/ironclaw/issues/3266) | The acceptance harness and outbound fanout surfaces are not ready to prove the full flow end-to-end. | Keep adapter-local tests in #4035; add full acceptance once harness/outbound land. |

## 4. Merged foundations that are not active blockers

Some dependencies have already landed and should be treated as foundations, not reasons to block the plan:

| Area | Link | Status | Use in Slack plan |
|---|---:|---|---|
| Approval service foundation | [#4029](https://github.com/nearai/ironclaw/pull/4029) | Merged | Use as the approval interaction target; do not reimplement in Slack. |
| Product auth OAuth routes | [#4031](https://github.com/nearai/ironclaw/pull/4031) | Merged | Foundation for auth setup; [#3289](https://github.com/nearai/ironclaw/issues/3289) still owns completion. |
| Manual token secure submit | [#4068](https://github.com/nearai/ironclaw/pull/4068) | Merged | Auth foundation; not Slack-specific. |
| Credential recovery projections | [#4069](https://github.com/nearai/ironclaw/pull/4069) | Merged | Auth/projection foundation; not Slack-specific. |
| ProductWorkflow completion plan | [#4164](https://github.com/nearai/ironclaw/pull/4164) | Open docs PR | Coordinates [#3280](https://github.com/nearai/ironclaw/issues/3280) follow-up routing work. |

## 5. Scope

### In scope for #3857

- Default-off Slack ProductAdapter MVP with preconfigured credentials.
- Slack request verification using `SLACK_SIGNING_SECRET` at the host ingress boundary.
- Slack bot-token egress through a host-owned credential handle (`SLACK_BOT_TOKEN` / `slack_bot_token`), never raw token exposure in the adapter.
- DM and app mention event normalization into ProductAdapter DTOs.
- Immediate Slack Events API ACK before long-running Reborn work.
- ProductWorkflow/RebornServices submission for supported inbound events.
- Slack `chat.postMessage` delivery for final replies through shared outbound policy.
- Conversational approval/auth handling only through Reborn interaction services.
- Fake Slack acceptance once the shared product-flow harness can prove the full route.

### Out of scope for #3857 MVP

- Slack OAuth install flow. The issue explicitly uses preconfigured credentials for the MVP.
- Full Slack v1 parity: pairing, broadcast lifecycle, file ingestion, search, rich home views, admin flows, or legacy Slack channel reuse.
- v1 `Channel` integration, old Slack config/startup wiring, or direct use of runtime internals.
- Slack-local approval stores, direct `TurnCoordinator::resume_turn`, or any custom run authority in the Slack adapter.
- A Slack-specific LLM tool for proactive messages. Proactive delivery should use shared outbound/product capability abstractions.

## 6. Proposed implementation sequence

The goal is to keep the Slack work reviewable without pretending the blocked parts are unblocked.

- **PR0 (current):** [#4035](https://github.com/nearai/ironclaw/pull/4035) — adapter-core/protocol boundary.
- **Default follow-up implementation PRs after #4035:** 3.
- **Fallback maximum:** 4 only if acceptance/outbound closure needs to split from approval/auth work.

### PR0 — Finish adapter-core slice (#4035)

**Scope**

- Resolve current merge conflicts with `reborn-integration`.
- Address remaining review items without expanding into host/runtime wiring.
- Keep the crate adapter-only: Slack payload parsing, auth/egress metadata declaration, final-reply render contract, architecture boundary rules, adapter-local tests.
- Keep the PR draft until the owner explicitly decides to mark ready.

**Current blockers**

- [#4035](https://github.com/nearai/ironclaw/pull/4035) is draft, `CHANGES_REQUESTED`, and `DIRTY`.
- This is locally fixable; it is not a cross-lane architecture blocker.

**Should not add**

- v1 Slack channel wiring.
- feature flag/startup integration in old runtime paths.
- ProductWorkflow calls from inside the adapter crate.
- Slack-local approval/auth state.

### PR1 — Host ingress, request verification, immediate ACK, ProductWorkflow submit

**Scope**

- Add the Reborn host ingress/registration path for Slack behind `REBORN_SLACK_ENABLED` or equivalent.
- Require preconfigured Slack signing secret and bot token handles.
- Verify Slack request signatures and timestamp/replay before parsing.
- ACK Slack events immediately.
- Normalize supported events through the adapter and submit to stable ProductWorkflow/RebornServices APIs.
- Ignore/ACK unsupported events safely.

**Blocked by / sequencing dependency**

- [#4035](https://github.com/nearai/ironclaw/pull/4035) must land or stabilize first so host wiring depends on the final adapter contract.
- [#3280](https://github.com/nearai/ironclaw/issues/3280) / [#4164](https://github.com/nearai/ironclaw/pull/4164) define ProductWorkflow completion sequencing. PR1 should use only stable user-message ProductWorkflow APIs and avoid assuming unresolved command/read/continuation behavior from [#3885](https://github.com/nearai/ironclaw/pull/3885).

**Pseudo-code shape**

```rust
async fn slack_events_handler(request: HttpRequest) -> HttpResponse {
    verify_slack_signature(request.headers(), request.raw_body(), signing_secret)?;
    let ack = HttpResponse::ok();

    spawn_reborn_task(async move {
        let parsed = slack_adapter.parse_inbound(verified_context, request.body).await?;
        product_workflow.accept_inbound(parsed).await?;
    });

    ack
}
```

Function names in this sketch are illustrative. The actual implementation should bind to stable ProductWorkflow/RebornServices entrypoints and use the repo's owned Reborn task/runtime abstraction, not an ad-hoc unmanaged background process.

### PR2 — Shared outbound/final-reply delivery to Slack

**Scope**

- Connect Reborn final reply/projection output to Slack `chat.postMessage` through shared outbound delivery policy.
- Use the adapter's render contract from [#4035](https://github.com/nearai/ironclaw/pull/4035).
- Preserve Slack thread reply semantics when `thread_ts` is present.
- Classify Slack egress failures into retryable/unauthorized/permanent delivery outcomes.
- Support proactive Slack delivery only as a backend of generic outbound capability, not as a Slack-specific tool bypass.

**Blocked by**

- [#3281](https://github.com/nearai/ironclaw/issues/3281) — durable projection fanout / EventStreamManager.
- [#3266](https://github.com/nearai/ironclaw/issues/3266) — outbound egress and subscription policy.

**Pseudo-code shape**

```rust
async fn deliver_slack_projection(event: ProjectionEvent) -> DeliveryOutcome {
    let target = outbound_policy.resolve_target(event.reply_target_ref)?;
    let view = final_reply_view_from_projection(event)?;
    let request = slack_adapter.render_final_reply(&target, &view, slack_bot_token_handle)?;
    protocol_http_egress.send(request).await.map(classify_slack_delivery)
}
```

### PR3 — Conversational approval/auth handling and Slack MVP closure

**Scope**

- Render approval prompts safely in Slack when Reborn emits an approval interaction.
- Interpret scoped Slack replies such as `approve` / `deny` only when they bind to a pending approval for the same user/thread/run scope.
- Route decisions through approval interaction services; do not resume runs directly from Slack.
- Render auth-required blocked/setup guidance through shared auth interaction semantics.
- Add fake Slack acceptance covering signed DM/app mention, invalid signature, ProductWorkflow submit, async final reply, and approval approve/deny paths when harness support exists.

**Blocked by**

- [#3094](https://github.com/nearai/ironclaw/issues/3094) — approval/auth interaction service completion.
- [#3289](https://github.com/nearai/ironclaw/issues/3289) — auth setup/product flows.
- [#3279](https://github.com/nearai/ironclaw/issues/3279) — product-flow acceptance harness.
- Depending on landing order, PR3 may also depend on [#3281](https://github.com/nearai/ironclaw/issues/3281) / [#3266](https://github.com/nearai/ironclaw/issues/3266) from PR2.

**Pseudo-code shape**

```rust
async fn handle_slack_reply_as_interaction(message: SlackMessage) -> ProductInboundPayload {
    let intent = parse_interaction_intent(&message.text)?;
    match intent {
        InteractionIntent::Approve | InteractionIntent::Deny => {
            ProductInboundPayload::ApprovalResolution(resolve_scoped_approval(message, intent)?)
        }
        InteractionIntent::AuthDenied | InteractionIntent::AuthProvided(_) => {
            ProductInboundPayload::AuthResolution(resolve_scoped_auth(message, intent)?)
        }
        InteractionIntent::NormalMessage => ProductInboundPayload::UserMessage(...),
    }
}
```

The scope lookup must fail closed on stale/missing/cross-user/cross-thread approvals.

### Optional PR4 — Acceptance-only split

Split PR3 only if:

- [#3279](https://github.com/nearai/ironclaw/issues/3279) lands after the approval/auth implementation is otherwise ready.
- The acceptance harness requires non-trivial fixture work that would make PR3 too large.
- Reviewers ask to isolate final fake Slack E2E proof from feature wiring.

PR4 should not introduce new Slack behavior. It should only prove the full #3857 flow and close remaining docs/status gaps.

## 7. Product capability / proactive Slack messages

The issue discussion notes that Slack should not be treated only as a channel; it also needs to participate in product capabilities such as proactive messages.

This plan handles that by treating Slack as a **delivery backend** behind shared outbound/product capability policy. The agent should not need a Slack-specific send-message tool to notify a user if the generic product outbound layer already knows the Slack binding and reply target.

Implications:

- Slack adapter may render and deliver Slack messages through declared egress.
- Reborn outbound policy decides whether proactive delivery is allowed.
- ProductWorkflow/EventStreamManager owns subscription/projection routing.
- Slack does not bypass policy with raw Slack API calls from the LLM/tool layer.

Related blockers remain [#3281](https://github.com/nearai/ironclaw/issues/3281) and [#3266](https://github.com/nearai/ironclaw/issues/3266).

## 8. Safety and layering invariants

Every implementation slice must preserve:

- No v1 Slack channel reuse or v1 startup/config paths.
- No raw Slack signing secret or bot token exposure to adapter parsing/rendering logic.
- Invalid Slack signatures reject before body parsing.
- Timestamp/replay protection for Slack requests.
- Unsupported Slack events ACK safely without creating Reborn work.
- Bot/self-message loops are ignored.
- Slack adapter does not call `TurnCoordinator`, `HostRuntime`, raw stores, raw network clients, or secret stores directly.
- Approval/auth decisions route through Reborn interaction services and fail closed on stale/missing/cross-scope interactions.
- Outbound delivery uses declared host/credential handles and shared policy.

## 9. Test guidance

This plan is docs-only. Implementation PRs should add the narrowest tests through the caller boundary for the behavior they own.

| PR | Required coverage |
|---|---|
| PR0 / [#4035](https://github.com/nearai/ironclaw/pull/4035) | Adapter parse/render/no-op/auth metadata/egress metadata/redaction; public adapter boundary error mapping; architecture boundary rule. |
| PR1 | Signed Slack request success; invalid signature reject before parse; timestamp/replay guard; immediate ACK; ProductWorkflow submit call; unsupported events ACK/no-op. |
| PR2 | Final reply projection -> Slack egress; Slack thread preservation; retryable/unauthorized/permanent delivery classification; no raw token exposure. |
| PR3 | Approval prompt render; scoped approve/deny replies; stale/missing/cross-scope fail closed; auth-required blocked/setup message; no direct resume. |
| Optional PR4 | Full fake Slack E2E once harness/outbound support exists. |

## 10. Reviewer checklist

- [ ] The plan makes clear that [#4035](https://github.com/nearai/ironclaw/pull/4035) is adapter-core only and does not close [#3857](https://github.com/nearai/ironclaw/issues/3857).
- [ ] Active blockers are linked and separated from merged foundations.
- [ ] The proposed follow-up PR count is reviewable: 3 by default after #4035, 4 only as an acceptance/outbound fallback.
- [ ] Slack host wiring does not use v1 channel paths.
- [ ] Async final replies and proactive delivery wait for shared outbound/EventStreamManager policy instead of becoming Slack-specific shortcuts.
- [ ] Approval/auth handling waits for shared interaction services and does not create Slack-local run authority.
- [ ] Acceptance expectations are tied to [#3279](https://github.com/nearai/ironclaw/issues/3279) rather than hidden in adapter unit tests.
