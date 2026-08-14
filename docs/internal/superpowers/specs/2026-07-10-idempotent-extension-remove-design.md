# Idempotent Extension Removal and OAuth Activation Design

## Goals

Make removal of a known catalog extension idempotent: if its installed package is already absent, removal must still run the same owner-scoped lifecycle cleanup and return a truthful non-error result.

Make extension OAuth installation complete only after the requested extension is active and its model-visible capabilities are published. OAuth credentials and identity bindings must not be reported as a successful extension connection while activation is absent or failed.

## Confirmed failure

After an older Slack removal deleted the installed package without completing every cleanup step, a retry called `builtin.extension_remove` with the valid input `{"extension_id":"slack"}`. The tool rejected the missing installation before cleanup, surfaced the unrelated `InputEncode` classification, and allowed the assistant to claim that nothing remained to disconnect.

In production, a Slack OAuth callback completed successfully after the Slack package had been removed. The browser then called the Slack activation endpoint, received HTTP 400, swallowed the failure, closed the configuration modal, and broadcast that the channel was connected. The resulting user had an active Slack identity binding and credential but no `slack.*` capabilities. The OAuth flow itself had been created after package removal, so OAuth start also failed to reject the missing requested extension.

## Idempotent removal design

The generic extension lifecycle remover resolves removal metadata from the persisted installed manifest when present and otherwise from the trusted available-extension catalog. The persisted manifest doubles as a cleanup tombstone: package/runtime state and materialized files may be removed first, but the manifest record is deleted only after external cleanup converges. This preserves the exact credential providers and removable channel surface across retries, restarts, and catalog drift without adding a new schema.

- Installed known extension: run shared channel cleanup, remove runtime/installed state and materialized files, run credential cleanup from the retained manifest tombstone, delete the tombstone, then return `removed: true`.
- Already-absent extension with a retained manifest tombstone: retry the same external cleanup from that exact manifest, delete the tombstone only after convergence, and return a truthful already-absent success.
- Already-absent known catalog extension without a tombstone: persist the trusted catalog manifest as a tombstone before external cleanup, then follow the same retry/finalization path so a later catalog deployment cannot change the cleanup obligation.
- Unknown or unmanaged extension: preserve the current rejection and never delete unmanaged files.

The remover remains provider-generic. It will not add Slack OAuth, binding, or path logic. Slack cleanup continues through the existing channel connection facade and product-auth lifecycle cleanup authority used by WebUI removal.

## OAuth activation design

Extension-scoped OAuth setup will use the existing generic `AuthContinuationRef::LifecycleActivation` contract instead of `SetupOnly`. The continuation identifies the requester extension and is persisted with the OAuth flow.

The composition-owned continuation dispatcher will handle lifecycle activation through the canonical extension lifecycle facade after OAuth completion. It will derive the authenticated caller scope from the durable auth event, activate the requested extension, and require an `activated: true` response with the extension's published capability IDs before the continuation succeeds. Turn-gate continuations continue through the existing turn-resume dispatcher; no provider-specific activation branch is added.

OAuth start for an extension will reject a catalog entry that is not currently installed. This prevents a stale browser from beginning a credential flow for a package that no longer exists. The error will be a stable client-visible conflict/invalid-state response telling the UI to refresh and install the extension again.

The WebUI will treat server-confirmed lifecycle activation as the completion authority. It must not close the modal, broadcast `channel-connected`, or claim success when activation fails. The existing best-effort post-OAuth activation call will be removed or reduced to an idempotent status refresh once the durable continuation owns activation.

If lifecycle activation fails after Slack binding/token mutation, the callback compensation path revokes only the exact credential material written by that callback and disconnects the failed connection epoch rather than commit a half-configured connection. Before activation, the flow takes an exclusive, leased `completing` claim; timestamp-fenced settlement prevents concurrent callbacks from letting a losing failure tear down a successful activation. The completed flow stores a redacted fingerprint of its access/refresh handles. Metadata-only account writes do not suppress cleanup, while a later reconnect or refresh with different handles makes stale compensation a no-op. It never selects every account for the provider. A failed flow plus its fingerprint is the durable cleanup journal; the fingerprint is cleared only after compensation succeeds, and flow-status polling resumes stale dispatch or cleanup after restart. A failed reconfiguration must not restore an older identity after its credential has already been replaced. If activation succeeds and only persistence of the continuation acknowledgement fails, the working binding and credential are preserved and the claim is released for an idempotent retry.

## Error behavior

A valid retry for an already-absent catalog extension will no longer produce `InputEncode`. If shared cleanup fails, removal will return the existing operational failure rather than claim success. Unknown extension ids remain invalid input.

Extension OAuth start against an absent installation returns a sanitized, actionable client error before creating a flow. OAuth callback activation failure returns a failed callback instead of a false success, preserves retryability where appropriate, and does not emit a channel-connected notification. The UI surfaces the failure and leaves the setup affordance available.

## Tests

Extend the existing extension lifecycle caller coverage to prove removal behavior:

1. A catalog-known but uninstalled Slack extension invokes the same channel cleanup and credential cleanup seams as an installed removal.
2. The response is successful and explicitly represents an already-absent package.
3. No package files are materialized or deleted during the repair path.
4. The existing unknown/unmanaged-extension test continues to reject removal and preserve files.
5. The production model-visible capability call with `{"extension_id":"slack"}` no longer returns `InputEncode` for this state.

Add caller-level OAuth and WebUI coverage to prove installation behavior:

1. Extension OAuth start persists `LifecycleActivation` for the requester extension and rejects an absent installation before creating a flow.
2. OAuth callback dispatches the lifecycle continuation through the production-composed lifecycle facade and publishes the real Slack capability IDs, including `slack.search_messages`.
3. Lifecycle activation failure makes the callback fail and invokes the existing Slack credential/binding rollback path.
4. Callback replay does not duplicate activation or corrupt lifecycle state.
5. A post-activation continuation-marker failure preserves the active binding and credential, avoids a second provider exchange, and succeeds on callback retry.
6. A lifecycle activation failure after reconfiguration revokes the replaced credential and leaves the Slack owner fully disconnected instead of restoring a stale binding.
7. The configuration modal does not close or broadcast channel-connected when activation/completion fails, and it renders a retryable error.
8. A whole-path regression drives install -> OAuth callback -> activation and asserts against the active capability surface, not merely that an activation request was attempted.
9. Activation compensation leaves unrelated accounts for the same provider configured.
10. A stale failed callback cannot revoke a newer credential generation, in either the in-memory contract or durable filesystem implementation, and a failed lifecycle flow is durably projected as `failed`.
11. Concurrent callbacks invoke lifecycle activation once, stale claim owners cannot settle after lease takeover, and fail-once compensation converges after service restart.

## Scope

No database migration, Slack-specific activation fallback, automatic reinstall, global registry redesign, or broader extension lifecycle refactor. One backwards-compatible optional fingerprint field is added to the existing serialized OAuth flow record so compensation can be generation-safe across retries and restarts. The generic continuation state machine reuses the existing `Completing` status; Slack-specific work remains limited to its existing identity-binding compensation hook.
