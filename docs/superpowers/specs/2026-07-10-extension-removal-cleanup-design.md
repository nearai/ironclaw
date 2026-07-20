# Explicit Extension Removal Cleanup

**Date:** 2026-07-10
**Status:** Approved
**Related issue:** [nearai/ironclaw#5953](https://github.com/nearai/ironclaw/issues/5953)

## Problem

Reborn extension removal currently infers channel cleanup from presentation
metadata. Every extension whose summary includes `ExternalChannel` is treated
as though it owns a Slack-style personal connection. The only production
`ChannelConnectionFacade` is Slack-specific, so a generic channel either:

- cannot be removed when that facade is absent; or
- is sent through a Slack implementation that silently ignores it.

`ExternalChannel` says how an extension communicates. It does not say that the
host owns a per-user connection that must be disconnected.

## Required Behavior

Every installed extension can be removed through the same operation used by
the WebUI and `builtin.extension_remove`.

- An extension with only lifecycle-owned files and installation state has no
  external cleanup requirements and removes normally.
- A generic external-channel extension likewise removes normally unless trusted
  host metadata explicitly attaches a cleanup requirement.
- A channel that owns host-managed resources declares the matching cleanup
  adapter and binding explicitly.
- Slack's user-facing `slack` package explicitly declares its personal Slack
  cleanup requirement.
- `slack_bot` does not inherit personal Slack cleanup merely because it exposes
  an external-channel surface.
- A declared cleanup requirement is mandatory. If its adapter is unavailable
  or fails, removal stops before local deletion and returns a retryable error.

## Ownership Model

Trusted catalog metadata owns cleanup declarations. Untrusted request bodies
and arbitrary manifest strings cannot select an adapter or cleanup scope.

```rust
pub struct ExtensionRemovalCleanupRequirement {
    pub adapter_id: ExtensionRemovalCleanupAdapterId,
    pub binding: ExtensionRemovalCleanupBinding,
}

pub enum ExtensionRemovalCleanupBinding {
    ChannelConnection { channel: ExtensionRemovalChannelId },
}
```

The adapter registry is keyed by the typed adapter id. A cleanup adapter:

- accepts only its binding type;
- receives trusted `ResourceScope` separately from the authenticated actor,
  replaces only the scope's user with that actor, and preserves tenant, agent,
  and project values from the trusted scope rather than reconstructing them
  from actor identity;
- is idempotent;
- returns success only after its owned state is clean; and
- returns sanitized failures.

The initial Slack adapter delegates the already-tested Slack disconnect
operation. It calls disconnect directly and never probes
`caller_channel_connections` to discover whether cleanup applies. Future
channels register their own adapter and attach the corresponding trusted
requirement.

## Removal Flow

`RebornLocalExtensionManagementPort::remove` remains the convergence point:

1. Resolve the installed extension and its trusted cleanup requirements.
2. If requirements are present, require an authenticated actor and build the
   caller scope from trusted execution context.
3. Execute every declared cleanup requirement in deterministic order.
4. Only after all cleanup succeeds, run the existing local extension removal.
5. Preserve the existing credential cleanup behavior after successful local
   removal; credential ownership is outside issue #5953.

No cleanup decision may inspect `LifecycleExtensionSurfaceKind`, connection
status maps, or whether a generic facade happens to recognize a channel id.

## Compatibility and State

This change does not alter `ExtensionInstallation`, database schemas, or
persisted manifest formats. It requires no SQL migration, eager rewrite, or
operator action.

Already-installed host-bundled Slack packages resolve through the current
trusted catalog and therefore receive the explicit Slack requirement at
runtime. Generic existing channel installations resolve to no host-owned
cleanup unless their trusted package definition explicitly supplies one.

## Legacy Channel-Removal Code to Delete

- `RemovableChannelCleanup`;
- `removable_channel_cleanup_for_summary`;
- `disconnect_channel_for_cleanup`;
- `cleanup_channel_before_remove`;
- the management port's removal-specific channel-facade field and builder;
- the optional `IfConnectionFacadeSupportsChannel` behavior;
- credential-based channel probing; and
- tests asserting that any generic external channel requires Slack cleanup.

Keep `ChannelConnectionFacade` for connection status and explicit user-driven
disconnect. Keep existing credential cleanup behavior. Do not modify v1 `src/`.

## Test Contract

TDD must begin with tests that fail on the current implementation and cover:

- generic external-channel removal succeeds without a channel cleanup adapter;
- a generic channel does not invoke a registered Slack cleanup adapter;
- an explicit channel requirement invokes exactly its matching adapter;
- a missing required adapter fails before deleting package files or state;
- adapter failure fails before deleting package files or state;
- Slack cleanup happens before package deletion;
- Slack cleanup does not call connection-status discovery;
- `slack_bot` does not receive personal Slack cleanup;
- an authenticated actor is required only when declared cleanup needs it; and
- WebUI and `builtin.extension_remove` produce the same observable result.

Tests must assert filesystem and installation-store effects, not only response
payloads.

Adapter acceptance coverage must also prove retry convergence. For Slack,
`slack_channel_connection_facade_disconnects_identity_and_personal_dm_target`
invokes the same caller cleanup twice and verifies the repeated call succeeds
with the owned connection state still clean, while
`slack_channel_connection_facade_retries_after_identity_delete_failure`
simulates partial progress, retries, and verifies the remaining identity state
is removed without duplicate destructive effects.
