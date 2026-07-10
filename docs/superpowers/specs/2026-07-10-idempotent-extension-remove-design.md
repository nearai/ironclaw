# Idempotent Extension Removal Design

## Goal

Make removal of a known catalog extension idempotent: if its installed package is already absent, removal must still run the same owner-scoped lifecycle cleanup and return a truthful non-error result.

## Confirmed failure

After an older Slack removal deleted the installed package without completing every cleanup step, a retry called `builtin.extension_remove` with the valid input `{"extension_id":"slack"}`. The tool rejected the missing installation before cleanup, surfaced the unrelated `InputEncode` classification, and allowed the assistant to claim that nothing remained to disconnect.

## Design

The generic extension lifecycle remover will resolve removal metadata from the installed extension summary when present and otherwise from the trusted available-extension catalog. Both summaries already describe the extension's credential providers and removable channel surface.

- Installed known extension: run shared channel/auth cleanup, remove installed state and materialized files, then return `removed: true`.
- Already-absent known catalog extension: run the same shared channel/auth cleanup, skip package/file deletion, then return a successful response explaining that the package was already absent and cleanup completed.
- Unknown or unmanaged extension: preserve the current rejection and never delete unmanaged files.

The remover remains provider-generic. It will not add Slack OAuth, binding, or path logic. Slack cleanup continues through the existing channel connection facade and product-auth lifecycle cleanup authority used by WebUI removal.

## Error behavior

A valid retry for an already-absent catalog extension will no longer produce `InputEncode`. If shared cleanup fails, removal will return the existing operational failure rather than claim success. Unknown extension ids remain invalid input.

## Tests

Extend the existing extension lifecycle caller coverage to prove:

1. A catalog-known but uninstalled Slack extension invokes the same channel cleanup and credential cleanup seams as an installed removal.
2. The response is successful and explicitly represents an already-absent package.
3. No package files are materialized or deleted during the repair path.
4. The existing unknown/unmanaged-extension test continues to reject removal and preserve files.
5. The production model-visible capability call with `{"extension_id":"slack"}` no longer returns `InputEncode` for this state.

## Scope

No schema changes, migrations, Slack-specific fallback, automatic reinstall, or broader extension lifecycle refactor.
