# Reborn ProductWorkflow routing completion plan

**Status:** Design / execution plan — pending implementation
**Date:** 2026-05-28
**Branch scope:** `reborn-integration` — ProductWorkflow routing and Reborn product-surface seams only
**Tracks:** [#3280 — Add ProductWorkflow and InboundTurnService facade](https://github.com/nearai/ironclaw/issues/3280)
**Parent:** [#3031 — Reborn product surface migration](https://github.com/nearai/ironclaw/issues/3031)
**Related foundations:** [#3269](https://github.com/nearai/ironclaw/issues/3269), [#3193](https://github.com/nearai/ironclaw/issues/3193), [#3089](https://github.com/nearai/ironclaw/issues/3089), [#3013](https://github.com/nearai/ironclaw/issues/3013), [#3198](https://github.com/nearai/ironclaw/issues/3198), [#3264](https://github.com/nearai/ironclaw/issues/3264)
**Active related lanes:** [#3286](https://github.com/nearai/ironclaw/issues/3286), [#3094](https://github.com/nearai/ironclaw/issues/3094), [#3289](https://github.com/nearai/ironclaw/issues/3289), [#3278](https://github.com/nearai/ironclaw/issues/3278), [#3281](https://github.com/nearai/ironclaw/issues/3281), [#3279](https://github.com/nearai/ironclaw/issues/3279), [#3266](https://github.com/nearai/ironclaw/issues/3266)

## 1. Purpose

[#3280](https://github.com/nearai/ironclaw/issues/3280) adds the product-facing Reborn workflow facade between `ProductAdapter`s and host-layer Reborn services. The target shape is:

```text
ProductAdapter
  -> ProductWorkflow
      -> InboundTurnService          # user-message subset
      -> ProductCommandRouter        # command seam; full matrix owned by #3286
      -> Approval/Auth interaction services
      -> MissionService
      -> EventStreamManager          # read/subscription dispatch only; #3281 owns streams
      -> product-safe acknowledgement / outcome
```

This document is an execution plan for finishing [#3280](https://github.com/nearai/ironclaw/issues/3280) without turning it into a catch-all implementation bucket for every downstream Reborn product-surface service. It sequences the remaining `ProductWorkflow` routing work around active blockers, keeps follow-up implementation PRs reviewable, and records the broader review feedback from [#3885](https://github.com/nearai/ironclaw/pull/3885) that should be discussed outside that narrow PR.

## 2. Current state

The current ProductWorkflow slice already establishes the main facade shape:

- `crates/ironclaw_product_adapters` owns adapter-facing DTOs, payloads, acknowledgements, protocol auth evidence, egress DTOs, and adapter capabilities.
- `crates/ironclaw_product_workflow` owns host-side ProductWorkflow orchestration, binding resolution, the mutating-action idempotency ledger, user-message dispatch, command seams, and product-safe error mapping.
- User messages route through `InboundTurnService`, which resolves the canonical binding/scope, stages accepted message content, and submits through `TurnCoordinator`.
- Mutating actions pass through an idempotency ledger and settle durable outcomes (`Accepted`, `DeferredBusy`, `Rejected`, `Duplicate`, `NoOp`, and command outcomes). Current `reborn-integration` exposes `CommandResult`; [#3885](https://github.com/nearai/ironclaw/pull/3885) proposes the narrower `CommandRouted` acknowledgement shape.
- Approval and auth resolution payloads already have ProductWorkflow dispatch paths on `reborn-integration`; remaining work should be treated as contract hardening/alignment with [#3094](https://github.com/nearai/ironclaw/issues/3094) / [#3289](https://github.com/nearai/ironclaw/issues/3289), not as first-time routing.
- `resolve_projection_subscription` exists as a read-path entrypoint for subscription requests; [#3885](https://github.com/nearai/ironclaw/pull/3885) tightens the mutating `accept_inbound` guard for `SubscriptionRequest`.
- [#3885](https://github.com/nearai/ironclaw/pull/3885) is still open and review-blocked at the time of this plan, so follow-up implementation should not assume its final ack/error naming until review settles.

Current explicit stub rejections for not-yet-wired action kinds are intentional. They prevent adapters from treating unsupported paths as successful, and they avoid speculative wiring to service boundaries that are still owned by other issues.

## 3. Scope

### In scope for #3280

- ProductWorkflow dispatch from `ProductInboundPayload` variants to stable host-service seams.
- Product-safe acknowledgement and rejection taxonomy at the workflow boundary.
- Mutating-action idempotency behavior for product actions that can create messages, runs, gate/auth decisions, mission actions, delivery attempts, or other side effects.
- Read-path separation for projection/subscription requests so high-volume read activity does not create mutating ledger rows.
- Caller-level contract tests through `ProductWorkflow`, not only private helper tests.
- Internal workflow crate cleanup that directly supports routing clarity, such as trigger-to-route helper placement after the downstream shape is stable.

### Out of scope for #3280

- Legacy v1 crates, `src/agent/`, `src/bridge/`, old channel runtime paths, or engine-v2 ownership shape.
- Slack/Telegram adapter-specific parsing/rendering behavior. [#3857](https://github.com/nearai/ironclaw/issues/3857) and [#4035](https://github.com/nearai/ironclaw/pull/4035) are adapter lanes, not ProductWorkflow completion work.
- Implementing the internals of `ApprovalInteractionService`, `AuthInteractionService`, `MissionService`, `EventStreamManager`, outbound egress policy, or the acceptance harness.
- Broad DTO serde refactors in `ironclaw_product_adapters` unless opened as a separate cleanup PR.
- A generic internal-message bypass or hidden ProductWorkflow route that skips normal policy, binding, idempotency, and product-safe outcome mapping.

The boundary rule for the remaining work is:

> #3280 finishes ProductWorkflow by routing to stable service boundaries as they land; it does not own the downstream services themselves.

## 4. Related issue / PR status map

This table distinguishes merged foundations from active blockers. A merged PR is not listed as an active blocker; it is listed only when it affects sequencing or confirms that part of the surface has landed.

| Area | Link | Current status | Why it matters to #3280 |
|---|---:|---|---|
| Parent epic | [#3031](https://github.com/nearai/ironclaw/issues/3031) | Open issue | Product-surface migration umbrella. |
| ProductWorkflow facade | [#3280](https://github.com/nearai/ironclaw/issues/3280) | Open issue | This plan's scope. |
| Current narrow slice | [#3885](https://github.com/nearai/ironclaw/pull/3885) | Open PR, changes requested | Must land before follow-up implementation assumes `CommandRouted` / read-path guard shape. |
| ProductAdapter foundation | [#3269](https://github.com/nearai/ironclaw/issues/3269) | Closed issue | Foundation for adapter DTO/capability split. |
| Binding/session contracts | [#3193](https://github.com/nearai/ironclaw/issues/3193) | Closed issue | Canonical binding/scope source for ProductWorkflow. |
| SessionThreadService | [#3089](https://github.com/nearai/ironclaw/issues/3089) | Closed issue | Accepted-message staging target. |
| TurnCoordinator kernel | [#3013](https://github.com/nearai/ironclaw/issues/3013) | Closed issue | Turn submission target. |
| TurnCoordinator API shape | [#3198](https://github.com/nearai/ironclaw/issues/3198) | Closed issue | ProductWorkflow must use public API, not trusted runner internals. |
| Turn admission policy | [#3264](https://github.com/nearai/ironclaw/issues/3264) | Closed issue | Admission errors map to product-safe outcomes. |
| Command behavior | [#3286](https://github.com/nearai/ironclaw/issues/3286) | Open issue | Full command compatibility matrix remains outside #3280. |
| Command foundation | [#3990](https://github.com/nearai/ironclaw/pull/3990) | Merged PR | Foundation landed; not an active blocker. |
| Lifecycle UX contracts | [#4012](https://github.com/nearai/ironclaw/pull/4012) | Merged PR | Reduces command/lifecycle uncertainty; issue [#3286](https://github.com/nearai/ironclaw/issues/3286) still tracks remaining behavior. |
| Approval/auth umbrella | [#3094](https://github.com/nearai/ironclaw/issues/3094) | Closed issue | Interaction-service foundation landed; no longer an active blocker, but follow-up hardening should still align with the settled contracts. |
| Approval service | [#3889](https://github.com/nearai/ironclaw/issues/3889) | Closed issue | Approval service lane landed; not an active blocker. |
| Approval service PR | [#4029](https://github.com/nearai/ironclaw/pull/4029) | Merged PR | Provides approval interaction service target. |
| Auth product flows | [#3289](https://github.com/nearai/ironclaw/issues/3289) | Open issue | Auth setup/recovery still owns service behavior that ProductWorkflow must call, not duplicate. |
| OAuth routes | [#4031](https://github.com/nearai/ironclaw/pull/4031) | Merged PR | Auth route foundation landed; [#3289](https://github.com/nearai/ironclaw/issues/3289) remains open. |
| Manual token submit | [#4068](https://github.com/nearai/ironclaw/pull/4068) | Merged PR | Auth foundation landed; not an active blocker by itself. |
| Credential recovery projections | [#4069](https://github.com/nearai/ironclaw/pull/4069) | Merged PR | Auth/projection foundation landed; [#3289](https://github.com/nearai/ironclaw/issues/3289) remains open. |
| MissionService | [#3278](https://github.com/nearai/ironclaw/issues/3278) | Open issue | ProductWorkflow cannot route mission actions to an unstable target. |
| EventStreamManager | [#3281](https://github.com/nearai/ironclaw/issues/3281) | Open issue | Projection/read/subscription and fanout ownership still lives here. |
| SSE replay fallback | [#4065](https://github.com/nearai/ironclaw/pull/4065) | Merged PR | Related event-stream foundation; not an active blocker by itself. |
| Acceptance harness | [#3279](https://github.com/nearai/ironclaw/issues/3279) | Open issue | Final end-to-end proof should land through the harness. |
| Outbound egress/subscription policy | [#3266](https://github.com/nearai/ironclaw/issues/3266) | Open issue | Final projection/egress behavior cannot be asserted before policy stabilizes. |
| Slack adapter lane | [#3857](https://github.com/nearai/ironclaw/issues/3857) | Open issue | Adapter consumer of ProductWorkflow; not part of #3280 implementation. |
| Slack adapter PR | [#4035](https://github.com/nearai/ironclaw/pull/4035) | Open PR, ready for review, changes requested | Shows why ProductWorkflow boundaries must stay adapter-agnostic; no longer draft and merge conflict has been resolved. |

## 5. Blockers

| Remaining #3280 area | Active blocker | Why blocked | What unblocks it |
|---|---|---|---|
| Baseline routing slice | [#3885](https://github.com/nearai/ironclaw/pull/3885) open with changes requested | Follow-up routing code should not stack on a review-disputed `CommandRouted` / read-path guard shape. | #3885 accepted or reviewer explicitly agrees to stack. |
| Command route hardening / matrix completion | [#3286](https://github.com/nearai/ironclaw/issues/3286) open | ProductWorkflow already has a baseline command route, and [#3990](https://github.com/nearai/ironclaw/pull/3990) / [#4012](https://github.com/nearai/ironclaw/pull/4012) merged foundations. The remaining blocker is the full command compatibility matrix, lifecycle behavior, and final command outcome naming across #3286 / [#3885](https://github.com/nearai/ironclaw/pull/3885). | #3286 closes or provides stable command-router contract points and outcome semantics ProductWorkflow can align with. |
| Approval/auth continuation hardening | [#3289](https://github.com/nearai/ironclaw/issues/3289) open; [#3094](https://github.com/nearai/ironclaw/issues/3094) closed | Basic ProductWorkflow approval/auth dispatch already exists. Approval service landed via [#4029](https://github.com/nearai/ironclaw/pull/4029), and auth foundations landed via [#4031](https://github.com/nearai/ironclaw/pull/4031), [#4068](https://github.com/nearai/ironclaw/pull/4068), [#4069](https://github.com/nearai/ironclaw/pull/4069). The remaining blocker is whether final auth setup/product-flow behavior requires ProductWorkflow outcome, idempotency, or rejection-shape changes. | #3289 service boundaries are stable enough to confirm no further #3280 ProductWorkflow delta is needed, or to make a narrow hardening PR. |
| Mission routing | [#3278](https://github.com/nearai/ironclaw/issues/3278) open | #3280 must not invent MissionService behavior or infer mission intent from ordinary chat. | MissionService integration exposes explicit mission-action target semantics. |
| Event/read/subscription routing | [#3281](https://github.com/nearai/ironclaw/issues/3281) and [#3266](https://github.com/nearai/ironclaw/issues/3266) open | ProductWorkflow may dispatch read/subscription requests, but EventStreamManager owns streams and #3266 owns outbound/subscription policy. | EventStreamManager and outbound/subscription policy surfaces stabilize. |
| Final acceptance proof | [#3279](https://github.com/nearai/ironclaw/issues/3279) open | Caller-level behavior needs TurnCoordinator product-flow acceptance coverage. | Harness lands or exposes reusable fixtures for ProductWorkflow end-to-end contracts. |

## 6. Routing ownership model

`ProductWorkflow` owns dispatch and product-safe outcomes, not downstream business logic.

| Payload | ProductWorkflow responsibility | Target owner | Current plan |
|---|---|---|---|
| `UserMessage` | Resolve binding, run inbound policy, stage accepted message, submit/defer turn, settle durable outcome. | `InboundTurnService` / `TurnCoordinator` | Existing path; keep coverage. |
| `Command` | Normalize command action, enforce admission, call command router, settle command routed/result acknowledgement or rejection. | [#3286](https://github.com/nearai/ironclaw/issues/3286) command lane | Finish after #3885 and stable #3286 contract. |
| `ApprovalResolution` | Deduplicate, check binding/auth context, route to approval interaction service, settle product-safe ack/rejection. | [#3094](https://github.com/nearai/ironclaw/issues/3094), [#4029](https://github.com/nearai/ironclaw/pull/4029) | Basic route exists; revisit only if #3094 changes outcome/idempotency/rejection semantics. |
| `AuthResolution` | Deduplicate, route auth continuation to auth interaction/setup service, avoid direct `resume_turn`. | [#3094](https://github.com/nearai/ironclaw/issues/3094), [#3289](https://github.com/nearai/ironclaw/issues/3289) | Basic route exists; align with final #3289 auth setup/recovery semantics if they require ProductWorkflow changes. |
| `SubscriptionRequest` | Reject from mutating `accept_inbound`; expose read-path resolver only. | [#3281](https://github.com/nearai/ironclaw/issues/3281), [#3266](https://github.com/nearai/ironclaw/issues/3266) | #3885 establishes guard; later routes through stream manager policy. |
| `LinkedThreadAction` | Require explicit typed intent; no hidden mission/event inference. | [#3278](https://github.com/nearai/ironclaw/issues/3278), [#3281](https://github.com/nearai/ironclaw/issues/3281) | Route after MissionService/EventStreamManager targets stabilize. |
| `NoOp` | Durable terminal no-op acknowledgement. | ProductWorkflow | Existing path. |

## 7. PR sequencing target

This plan deliberately keeps the implementation series short.

- **This docs PR:** Add the execution plan only; no code changes.
- **PR0 (already open):** [#3885](https://github.com/nearai/ironclaw/pull/3885) — read-path guard and command routed ack.
- **Follow-up implementation PR target:** 3 PRs, counting PR2 as a hardening/alignment slot rather than guaranteed new routing work.
- **Collapse rule:** if [#3094](https://github.com/nearai/ironclaw/issues/3094) / [#3289](https://github.com/nearai/ironclaw/issues/3289) do not require ProductWorkflow deltas, PR2 should collapse into PR3 instead of creating review churn.
- **Fallback maximum:** 4 PRs only if PR3 becomes too large or blocker landing order forces a split.

### PR1 — Command route hardening and command-matrix alignment

**Default scope**

- Land on top of [#3885](https://github.com/nearai/ironclaw/pull/3885) once accepted or explicitly approved for stacking.
- Treat the existing ProductWorkflow command route as the baseline; do not reimplement the current context/admission/execute seam.
- Align command outcome naming, acknowledgement shape, and product-safe replay behavior with [#3885](https://github.com/nearai/ironclaw/pull/3885) review results and the [#3286](https://github.com/nearai/ironclaw/issues/3286) command matrix.
- Keep command admission/rejection mapping product-safe.
- If still useful after review, extract trigger-to-route mapping inside `ironclaw_product_workflow`; do not move workflow route concepts into `ironclaw_product_adapters`.
- Keep explicit unsupported-action rejections for payloads whose downstream services are not stable.

**Blocked by**

- [#3885](https://github.com/nearai/ironclaw/pull/3885) review state.
- [#3286](https://github.com/nearai/ironclaw/issues/3286) remaining command behavior decisions. [#3990](https://github.com/nearai/ironclaw/pull/3990) and [#4012](https://github.com/nearai/ironclaw/pull/4012) are merged foundations, not active PR blockers.

**Pseudo-code shape**

```rust
match envelope.payload() {
    ProductInboundPayload::Command(command_payload) => {
        // Baseline route exists. PR1 only adjusts this if #3885 / #3286
        // require a different product-safe outcome or command-matrix mapping.
        existing_command_route(envelope, action_id, fingerprint, command_payload).await
    }
    _ => existing_non_command_path(...),
}
```

### PR2 — Continuation hardening: approval + auth alignment

**Default scope**

- Treat current ProductWorkflow approval/auth dispatch as the baseline, not as missing work.
- Audit whether final [#3094](https://github.com/nearai/ironclaw/issues/3094) / [#3289](https://github.com/nearai/ironclaw/issues/3289) behavior requires ProductWorkflow changes to acknowledgement shape, idempotency key derivation, stale/expired interaction handling, or product-safe rejection mapping.
- Preserve ProductWorkflow-owned idempotency, product-safe rejection taxonomy, and actor/binding authority checks.
- Do not call `TurnCoordinator.resume_turn` directly from adapters or from ad-hoc ProductWorkflow code.
- Do not implement approval/auth service internals here.
- If the audit finds no ProductWorkflow delta, collapse this slot into PR3 and document that approval/auth continuation routing is already covered.

**Blocked by**

- [#3094](https://github.com/nearai/ironclaw/issues/3094) is closed; use its landed approval/auth interaction-service contracts as the foundation.
- [#3289](https://github.com/nearai/ironclaw/issues/3289) still open for auth setup/product flows.
- Landed foundations: [#3889](https://github.com/nearai/ironclaw/issues/3889) closed, [#4029](https://github.com/nearai/ironclaw/pull/4029) merged, [#4031](https://github.com/nearai/ironclaw/pull/4031) merged, [#4068](https://github.com/nearai/ironclaw/pull/4068) merged, [#4069](https://github.com/nearai/ironclaw/pull/4069) merged. These should be used as inputs, not reimplemented.

**Pseudo-code shape**

```rust
match envelope.payload() {
    ProductInboundPayload::ApprovalResolution(resolution) => {
        // Baseline route exists. PR2 only adjusts this if the final
        // interaction-service contract requires a different product-safe
        // outcome or stale/expired handling shape.
        existing_approval_resolution_path(envelope, action_id, fingerprint, resolution).await
    }
    ProductInboundPayload::AuthResolution(resolution) => {
        // Baseline route exists. PR2 only adjusts this if #3289 changes the
        // adapter-facing continuation semantics.
        existing_auth_resolution_path(envelope, action_id, fingerprint, resolution).await
    }
    _ => existing_path(...),
}
```

### PR3 — Mission/event/read routing and final #3280 closure

**Default scope**

- Define or depend on a typed linked-action intent contract before routing `LinkedThreadAction`; the current payload is opaque and must not be interpreted through string conventions.
- Route `LinkedThreadAction` only when the payload carries explicit mission/event/thread action intent.
- Route projection read/subscription requests through the stable EventStreamManager and outbound/subscription policy seams.
- Finish ProductWorkflow-facing dispatch shape once downstream targets are stable.
- Add final caller-level ProductWorkflow contract tests for all payload variants that #3280 owns.
- Decide whether a handler registry is now justified. If downstream targets are still simple, keep explicit typed routing functions.

**Blocked by**

- [#3278](https://github.com/nearai/ironclaw/issues/3278) MissionService integration and any typed mission-action intent contract.
- [#3281](https://github.com/nearai/ironclaw/issues/3281) EventStreamManager and any typed event/thread-action intent contract.
- [#3266](https://github.com/nearai/ironclaw/issues/3266) outbound egress/subscription policy.
- [#3279](https://github.com/nearai/ironclaw/issues/3279) acceptance harness for final proof. [#4065](https://github.com/nearai/ironclaw/pull/4065) is merged event-stream-related foundation, not an active PR blocker.
- A typed linked-action intent contract, tentatively named `LinkedThreadActionIntent` or `LinkedThreadActionKind`, must exist before PR3 routes opaque linked-action payloads.

**Pseudo-code shape**

```rust
match envelope.payload() {
    ProductInboundPayload::LinkedThreadAction(action) => {
        // Requires a typed linked-action contract first; do not infer
        // mission/event intent from opaque action_id/data string conventions.
        let intent = LinkedThreadActionIntent::try_from(action)?;
        match intent {
            LinkedThreadActionIntent::Mission(mission_action) => {
                mission_service.submit_product_action(context, mission_action).await
            }
            LinkedThreadActionIntent::Event(event_action) => {
                event_stream_manager.route_product_action(context, event_action).await
            }
        }
    }
    ProductInboundPayload::SubscriptionRequest(_) => {
        reject_mutating_path("subscription_request must use resolve_projection_subscription")
    }
    _ => existing_path(...),
}
```

### Optional PR4 — Acceptance / outbound stabilization split

PR4 is not part of the default plan. Split PR3 only if:

- PR3 becomes too large for a focused review.
- [#3279](https://github.com/nearai/ironclaw/issues/3279) or [#3266](https://github.com/nearai/ironclaw/issues/3266) lands after the mission/event routing pieces.
- Reviewers ask to separate final acceptance/outbound verification from dispatch implementation.

PR4 should not introduce new routing concepts. It should only add final acceptance/outbound verification glue and documentation updates needed to close [#3280](https://github.com/nearai/ironclaw/issues/3280).

## 8. Final dispatch shape

The final ProductWorkflow dispatcher should remain explicit until all target services are stable. Premature handler registries over stub rejections hide missing downstream ownership and make it easier to route to the wrong abstraction.

Target shape:

```rust
async fn dispatch_payload(
    envelope: &ProductInboundEnvelope,
    action_id: ProductActionId,
    fingerprint: ActionFingerprintKey,
    ports: DispatchPorts<'_>,
) -> Result<DispatchedAction, ProductWorkflowError> {
    match envelope.payload() {
        ProductInboundPayload::UserMessage(_) => {
            ports.inbound_turn.accept(envelope, ports.before_inbound_policy).await
        }
        ProductInboundPayload::Command(command) => {
            ports.command_router.dispatch(envelope, action_id, fingerprint, command).await
        }
        ProductInboundPayload::ApprovalResolution(resolution) => {
            ports.approval_interactions.resolve(envelope, action_id, fingerprint, resolution).await
        }
        ProductInboundPayload::AuthResolution(resolution) => {
            ports.auth_interactions.resolve(envelope, action_id, fingerprint, resolution).await
        }
        ProductInboundPayload::SubscriptionRequest(_) => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                action: "subscription_request",
            })
        }
        ProductInboundPayload::LinkedThreadAction(action) => {
            ports.linked_thread_actions.dispatch(envelope, action_id, fingerprint, action).await
        }
        ProductInboundPayload::NoOp => Ok(DispatchedAction {
            ack: ProductInboundAck::NoOp,
            dispatch_kind: ActionDispatchKind::NoOp,
        }),
    }
}
```

If PR3 shows repeated setup and uniform side-effect semantics across stable service targets, then introduce a small typed handler registry inside `ironclaw_product_workflow`. Do not move workflow-specific route kinds into `ironclaw_product_adapters`.

## 9. Anti-monolith boundary and deferred ports question

`ProductWorkflow` should remain a thin dispatcher plus idempotency/outcome boundary. It should not become the long-term home for every concrete downstream service implementation.

Current reviewed baseline keeps approval/auth interaction service implementations in `ironclaw_product_workflow`; this plan does not ask [#3280](https://github.com/nearai/ironclaw/issues/3280) to refactor that in-line. The follow-up decision is whether to split product-facing service traits into a small ports-only crate, with bridge implementations wired by `ironclaw_reborn_composition`, so generic domain crates such as `ironclaw_auth`, `ironclaw_approvals`, `ironclaw_conversations`, and `ironclaw_turns` stay product-agnostic.

Target principle:

```text
ironclaw_product_adapters        # DTO floor: payloads, acks, rejections
        ^
ironclaw_product_workflow_ports  # traits only; speaks product DTOs
        ^
ironclaw_product_workflow        # dispatch_payload, DispatchPorts, idempotency, outcome/error mapping
        ^
ironclaw_reborn_composition      # wires concrete bridge implementations
```

Do not stack new concrete mission/event/command service implementations into `ironclaw_product_workflow` by default. Add only the routing/port surface needed for a reviewed ProductWorkflow slice, and track any larger ports-crate extraction as a separate issue.

Auth naming note: `ironclaw_auth::AuthInteractionService` is the lower auth-domain contract; `ironclaw_product_workflow`'s auth interaction service is the product-surface adapter around that lower contract. If this name collision continues to confuse follow-up PRs, prefer a future rename on the product-surface side rather than coupling product DTOs into `ironclaw_auth`.

## 10. Review feedback deferred from #3885

[#3885](https://github.com/nearai/ironclaw/pull/3885) received broader structural feedback that should not expand that PR. This plan tracks where each item belongs.

| Feedback | Plan |
|---|---|
| Duplicate trigger-to-route mapping | Consider in PR1, but keep helper ownership in `ironclaw_product_workflow`. `ProductConversationRouteKind` is workflow/binding semantics and should not be pushed into adapter DTOs. |
| `Wire` struct + manual `Deserialize` boilerplate | Treat as DTO hygiene / serde cleanup outside the #3280 critical path. It does not block ProductWorkflow completion. |
| `dispatch_payload` handler registry | Revisit in PR3 after approval/auth/mission/event targets are stable. Do not build a registry around stub rejections. |
| Explicit stub rejections | Keep until corresponding target service is stable enough to call. Stubs are safer than pretending unsupported product actions succeeded. |

## 11. Product-safe outcome rules

Every implementation PR should preserve these invariants:

- Adapters never map raw downstream errors themselves.
- Canonical tenant, actor, owner/resource scope, thread, source binding, and reply target come from binding/session services, not adapter-provided internal IDs.
- `ProductWorkflow` never exposes raw prompts, tool input, secrets, host paths, backend diagnostics, foreign existence, raw provider errors, or raw runtime output.
- Retryable dependency failures before side effects release the idempotency lease.
- Durable terminal outcomes settle the idempotency ledger.
- Read/subscription requests do not create mutating ProductInboundAction rows.
- Gate/auth continuation payloads route through interaction services; normal gate UX must not resume turns directly from adapter-local logic.

## 12. Test and verification guidance

This design doc does not authorize skipping tests in implementation PRs. Each implementation PR should include the narrowest tests that prove the route through the caller, not only private helper tests.

Recommended checks by PR:

| PR | Required coverage |
|---|---|
| PR1 | ProductWorkflow command-dispatch contract tests; idempotency settle/release behavior; product-safe command rejection mapping; architecture boundary check if dependencies change. |
| PR2 | If ProductWorkflow changes are needed: caller-level approval/auth continuation tests through `ProductWorkflow`; stale/unauthorized/invalid gate/auth cases; retryable vs terminal error behavior. If no changes are needed, record that existing coverage already exercises the baseline routes. |
| PR3 | ProductWorkflow tests for linked-thread action, read/subscription routing, no hidden mission inference, no mutating ledger rows for read paths. |
| Optional PR4 | Acceptance harness / outbound verification only, tied to [#3279](https://github.com/nearai/ironclaw/issues/3279) and [#3266](https://github.com/nearai/ironclaw/issues/3266). |

Because these routes gate side effects through `ProductWorkflow` and wrapper services, implementation PRs should include caller-level integration coverage (for example, `cargo test --features integration` where applicable) rather than helper-only unit tests. Architecture-sensitive changes should run the Reborn boundary tests referenced by the owning crates, especially when adding dependencies or public service ports.

## 13. Open questions for reviewers

1. Should PR1 include a workflow-local trigger-to-route helper, or should that wait until PR3 when all route targets exist?
2. Is `UnsupportedActionKind` sufficient for intentionally unwired payload variants, or should #3280 add a more specific `InvalidMutatingAction` / `WrongEntrypoint` workflow error?
3. Once [#3094](https://github.com/nearai/ironclaw/issues/3094) and [#3289](https://github.com/nearai/ironclaw/issues/3289) settle, is any ProductWorkflow approval/auth hardening still needed, or can the existing baseline route be treated as complete for #3280?
4. Should DTO serde cleanup become a standalone issue, or remain opportunistic cleanup when a DTO is already touched?
5. Where should the typed linked-action intent discriminator live: adapter DTOs, workflow-local contracts, or the owning MissionService/EventStreamManager surface?
6. Should the final dispatcher remain explicit, or should PR3 introduce a typed handler registry after all downstream service targets are stable?

## 14. Reviewer checklist

- [ ] The plan stays within [#3280](https://github.com/nearai/ironclaw/issues/3280) and does not claim ownership of downstream service internals.
- [ ] Active blockers are accurately distinguished from merged/closed foundations.
- [ ] Follow-up implementation is constrained to 3 PRs by default, with a 4th only as a fallback split.
- [ ] `ProductWorkflow` remains the product action orchestrator and `InboundTurnService` remains the narrower user-message path.
- [ ] Adapter-specific work such as [#3857](https://github.com/nearai/ironclaw/issues/3857) / [#4035](https://github.com/nearai/ironclaw/pull/4035) is kept out of this plan except as a consumer of the boundary.
- [ ] The plan avoids v1 crates, raw runtime internals, and direct adapter-local gate/auth resume behavior.
