# Generic OAuth Denial Lifecycle Design

## Goal

Make auth denial release or resume a blocked run according to who made the
decision, without embedding lifecycle policy in Telegram, Slack, WebUI, or any
other product adapter. Also guide Slack personal OAuth to the host's configured
workspace so a user's unrelated active Slack workspace is not selected by
default.

## Observed failures

The QA incident exposed two separate lifecycle gaps:

1. Slack personal OAuth authorization URLs omitted Slack's optional `team`
   parameter. Slack therefore selected the user's active `near-devhub`
   workspace, where the QA app is not distributed, and returned
   `invalid_team_for_non_distributed_app`.
2. An explicit `auth deny` canceled the OAuth flow but resumed the same model
   run with `GateResumeDisposition::Denied`. The model then selected a sibling
   Slack capability, which created another auth gate and left the conversation
   blocked under a new gate reference.

Provider-owned cancellation is a third, distinct path. When a user presses
Cancel in the OAuth popup and the provider returns `error=access_denied`, the
flow is marked `Failed(ProviderDenied)`, but the canceled-auth continuation is
not dispatched, so the parked gate remains blocked.

## Behavior contract

| User action | Auth-flow result | Blocked-run result | Rationale |
| --- | --- | --- | --- |
| Explicit product action `auth deny` | Cancel active flow | Terminally cancel the exact blocked run | The user told IronClaw to abandon the current run. Resuming the model can recreate the gate. |
| OAuth provider popup returns denial/cancel | Mark flow `Failed(ProviderDenied)` | Resume the exact auth gate with `GateResumeDisposition::Denied` | The user declined this credential grant, but did not ask to abandon the surrounding request. The model may explain the denial or continue without the capability. |
| OAuth callback completes | Mark flow completed and persist credentials | Resume the exact gate normally | Existing behavior remains unchanged. |
| Provider never calls IronClaw back | Flow remains awaiting user | Remain blocked until explicit denial or existing lifecycle cleanup | Popup closure and provider-owned error pages are not observable from a plain channel link. This change does not infer cancellation from browser closure. |

The conversation thread and transcript are never deleted. Explicit denial
cancels only the active run, preserving audit history and admitting the next
message through the existing thread.

## Ownership and data flow

### Explicit denial

`DefaultAuthInteractionService` in `ironclaw_product_workflow` owns the policy:

1. Re-read the scoped run and verify it is parked on the exact auth gate.
2. Conditionally reserve an active auth-flow record as `Canceling`. This
   compare-and-swap transition chooses one winner against callback completion.
3. Re-verify the exact gate and call `TurnCoordinator::cancel_run` with
   `SanitizedCancelReason::UserRequested`
   and a typed `BlockedAuthGate` compare-and-cancel precondition carrying the
   exact gate reference.
4. Finalize the reserved flow as `Canceled`. Roll it back to `AwaitingUser` if
   run lookup or cancellation fails, including a stale-gate failure.
5. Return `ResolveAuthInteractionResponse::Canceled`.

All product surfaces already normalize an auth-denial action into
`AuthInteractionDecision::Deny`, so this one change applies to Telegram, Slack,
WebUI, and future adapters. Adapters remain responsible only for parsing,
routing, acknowledgment, and rendering.

If the flow record is absent but the run is still parked on the exact gate, the
service cancels the run. If OAuth completed before the denial reservation,
completion wins and the service resumes the gate through the completed path.
If denial reserves first, callback completion cannot cross the `Canceling`
state. The turn store evaluates the run status and gate reference atomically
with cancellation, so a preflight read cannot race a successful resume into
cancellation.

### Provider-popup denial

`RebornProductAuthServices` owns provider callback lifecycle independent of the
originating surface:

1. Preserve the durable `Failed(ProviderDenied)` flow state and provider-denied
   HTTP response.
2. Build an `AuthContinuationEvent` from the failed, scoped flow.
3. Dispatch it through
   `RebornAuthContinuationDispatcher::dispatch_canceled_auth_continuation`.
4. For `TurnGateResume`, the existing product-workflow dispatcher resumes the
   exact blocked gate with `GateResumeDisposition::Denied` and the
   `BlockedAuthGate` precondition.
5. Mark the continuation dispatched so duplicate callbacks and reconciliation
   are idempotent.

Provider denial and lifecycle cleanup share one dispatch-and-acknowledgement
gateway. It retries one transient backend failure inline. If dispatch still
cannot complete, the callback surfaces backend unavailability rather than
claiming the gate was released; the durable unacknowledged failed flow remains
the retry journal for a duplicate callback or explicit reconciliation.

Setup-only and lifecycle continuations retain their existing provider-owned
terminal cleanup behavior. The canceled continuation has no turn side effect
when the flow did not originate from a turn gate.

### Slack workspace targeting

`SlackSetupService` will expose a non-secret OAuth authorization context with
the configured client ID and team ID. Both Slack personal OAuth URL builders
will include exactly one `team=<configured team id>` extra parameter. Token
exchange continues to use the existing secret-bearing credential accessor.

This is a UX hint, not an authorization boundary. The existing callback binding
validation remains authoritative for the returned Slack team and app IDs.
Enabling Slack app distribution or accepting arbitrary workspaces is outside
this change.

## Failure and race handling

- A conditional `Canceling` reservation precedes run cancellation. The run
  transition carries an atomic `BlockedAuthGate` precondition; if the run has
  resumed or moved to a different gate, the stale denial cannot cancel it and
  the reservation is rolled back. The flow is terminally canceled only after
  the run cancellation succeeds.
- OAuth completion and explicit denial have one durable winner. A completed
  flow follows the completion/resume path; a callback that encounters a live
  denial reservation fails closed without persisting credentials.
- Cancellation settlement is timestamp-fenced. A replay finalizes a stranded
  reservation when the exact run is already canceled, while an expired lease
  may be reclaimed without allowing the stale worker to settle it.
- Provider-denied continuation dispatch uses the existing gate precondition and
  actor/scope validation. A stale gate is treated as already converged.
- Duplicate provider-denied callbacks do not resume twice after
  `continuation_emitted_at` is recorded.
- A transient canceled-continuation dispatch is retried once inline. Exhausted
  retries surface backend unavailability and leave the durable continuation
  marker unset for a later callback or reconciliation attempt.
- If the post-write flow reload fails, the callback surfaces that backend error.
  The durable failed flow remains available for later reconciliation.
- Slack terminal binding cleanup continues to run on provider denial even after
  the turn-gate continuation is dispatched.

## Test strategy

1. Product-workflow contract tests prove `AuthInteractionDecision::Deny`
   cancels the flow and run, never calls `resume_turn`, handles missing flow
   records, remains idempotent, and cannot cancel a run that left the gate
   between the product read and store transition.
2. Turn-store contract tests prove conditional cancellation requires both the
   expected blocked status and exact gate reference, and that the optional
   request field remains wire-compatible with older unconditional requests.
3. Product-auth API tests prove `ProviderDenied` dispatches the canceled
   continuation, records the failed flow, returns the provider-denied callback
   error after successful dispatch, and leaves reload or exhausted-dispatch
   failures durably retryable without reporting false completion.
4. Product-workflow continuation tests continue to prove canceled provider
   continuations resume only the exact `BlockedAuth` gate with a denied
   disposition.
5. Slack route and gate-provider tests parse generated authorization URLs and
   assert the configured team appears exactly once alongside `user_scope` and
   PKCE parameters.
6. The Telegram journey resolves the exact run from the durable transcript,
   waits for its authoritative `Cancelled` state, asserts the model call count
   remains unchanged, and then proves a subsequent message is accepted on the
   same thread for an explicit in-chat `auth deny`.
7. A second Telegram journey installs Slack from a paired DM, proves the
   delivered authorization URL targets the configured workspace, drives a
   provider denial through the real public callback route, verifies the durable
   `Failed(ProviderDenied)` continuation marker and exact run transition from
   `BlockedAuth` to `Completed`, then proves both the resumed reply and a new
   message are delivered in the same Telegram thread. These journeys are
   adapter proof of the generic workflow contract, not Telegram-owned lifecycle
   rules.

## Non-goals

- Deleting conversation threads or transcripts.
- Automatically treating popup closure without a provider redirect as
  cancellation. Provider-reported cancellation remains in scope.
- Making the Slack app distributable or supporting arbitrary Slack workspaces.
- Reintroducing the reverted OAuth expiry/supersession changes from PR #6130.
- Adding channel-specific auth lifecycle policy.
