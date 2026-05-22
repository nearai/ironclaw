# Native Extensions

## Provider Split Policy

Each provider lives in a self-contained subdir under `src/`. Subdirs MUST NOT import from siblings.

Split a provider into its own sibling crate (`ironclaw_native_extensions_<provider>`) when ANY of:
- Provider needs heavy external deps not shared by others (>5 MB additional binary size or >5s compile-time impact)
- Provider count exceeds ~10
- Provider has independent release cadence (for example, a vendored SDK with its own versioning)

Split is mechanical: copy subdir -> new crate -> update workspace + `reborn_dependency_boundaries.rs` -> adjust register fn name.

## Provider Capability Packages

A provider's `register` fn populates `RegistrationOutput` with one
`ExtensionPackage` per capability surface plus its first-party capability
handlers. Capability ids MUST be prefixed with the package's `ExtensionId`
(for example `google-calendar.list_events` under extension `google-calendar`)
or `ExtensionPackage::from_manifest` rejects them. Write capabilities declare
`PermissionMode::Ask` plus `EffectKind::ExternalWrite`; read capabilities use
`PermissionMode::Allow`. Approval gating is descriptor-level — handlers never
implement approval themselves; the host authorization layer blocks an
unapproved write before `dispatch` runs.

Handler outputs MUST project only whitelisted fields onto dedicated output
structs; the raw provider response and the OAuth access token are never echoed
into handler output.

The Google subdir's calendar package lives in `src/google/calendar/`
(`manifest.rs` declares descriptors, `handlers.rs` implements them). The Gmail
package lives in `src/google/gmail/` and follows the same `manifest.rs` /
`handlers.rs` split. Both packages share the credential resolver, network
policy, OAuth provider, and `scopes` defined directly under `src/google/`;
sibling capability packages MUST NOT import from each other.
