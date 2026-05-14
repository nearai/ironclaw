# Successor PR: production hook gate-ref factory

> Successor work from PR #3573. Until this lands, hook `PauseApproval` /
> `PauseAuth` decisions surface as `Denied` in production because the
> default factory is `FailClosedHookGateRefFactory`.

## Scope

Implement a `HookGateRefFactory` that mints refs the host's existing
approval/auth router can resolve. The current `UuidHookGateRefFactory`
mints v4 UUIDs — locally unique but router-unregistered, so the loop
parks on a ref the gateway has never heard of. That's why the middleware
default is fail-closed (henrypark133 Critical #3).

## What this factory must do

1. **Reserve a gate.** On `mint_approval_ref(reason)`, register a new
   pending approval with the host's approval gateway. The returned
   `LoopGateRef` is the gateway's chosen identifier — not synthesized
   in the factory.
2. **Bind to `LoopRunContext`.** The factory carries the current
   `LoopRunContext` so the reservation is keyed against the right
   run / thread / tenant. Resolution events route back to the correct
   loop.
3. **Carry one-shot + TTL semantics.** Reservations expire after a
   configurable window (operator policy). The gateway enforces
   one-shot consumption — once a user approval lands, the ref is
   marked consumed and a replay attempt is rejected.
4. **Symmetric `mint_auth_ref`** behavior for auth flows.

## Likely surface

```rust
// in ironclaw_reborn (or a new crate if approval lives elsewhere)
pub struct RouterBackedHookGateRefFactory {
    run_context: LoopRunContext,
    approval_gateway: Arc<dyn ApprovalGateway>,
    auth_gateway: Arc<dyn AuthGateway>,
    reservation_ttl: Duration,
}

#[async_trait]
impl HookGateRefFactory for RouterBackedHookGateRefFactory {
    async fn mint_approval_ref(&self, reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        // 1. Call approval_gateway.reserve(...) with run_context + reason + ttl
        // 2. Map gateway result to LoopGateRef
        // 3. Fail closed if gateway is unavailable
    }
    // ...
}
```

Wired into `RebornLoopDriverHostFactory::with_hook_gate_ref_factory(...)`
per-build (so `LoopRunContext` is the current run's).

## Threat model considerations

- **Forgery resistance.** Already pinned at the factory side (122-bit v4
  UUIDs or gateway-issued opaque tokens). The router-backed impl should
  use the gateway's existing reservation-id format if it's
  unguessable, otherwise wrap a v4 UUID inside.
- **Replay resistance.** Comes from gateway's one-shot consumption,
  not from the factory. Test: consume a ref twice → second consumption
  rejected with `gate_already_consumed`.
- **TTL bypass.** Operator policy. Test: mint, wait > TTL, attempt
  resolution → rejected with `gate_expired`.
- **Cross-run consumption.** Resolution of run A's gate ref from run B
  must be rejected. Test: mint under run A, attempt consumption under
  run B's context → rejected.

## Required tests (caller level)

All through `RebornLoopDriverHostFactory` with the production factory
installed:

1. **Happy path**: `PauseApproval` hook → outcome is `ApprovalRequired
   { gate_ref }`; user resolves via gateway; loop receives `Resolved`
   event; loop unblocks.
2. **TTL expiry**: mint, wait past TTL, attempt resolution → gateway
   rejects; loop receives `Expired` event.
3. **One-shot**: consume twice → second attempt rejected.
4. **Cross-run**: gate-ref from run A is unconsumable from run B.
5. **Gateway unavailable**: mint fails → middleware surfaces
   `Denied { reason_kind: hook_gate_ref_unavailable }`.

## What this PR does NOT do

- Change the middleware default (stays `FailClosedHookGateRefFactory`).
- Provide a UI for the operator to configure TTL — that's the host's
  config concern.
- Touch the existing `UuidHookGateRefFactory` (kept for tests + dev).

## Risk

- Cross-crate dependency on the approval/auth gateway. May require a
  trait extraction if the gateway lives downstream of `ironclaw_hooks`.
- Coordination with the channel-to-user path (#3564 area).

## Effort

Medium. Most of the complexity is in the cross-crate seam to the
approval gateway, not in the factory itself.
