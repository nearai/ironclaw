# PR-derived browser test recipes

Choose the smallest set that reaches the changed contract.

| PR change | Primary browser evidence | Important second case |
|---|---|---|
| New page or component | Navigate through the intended entrypoint and verify rendered content/actions | Deep-link and refresh |
| Form or settings | Submit realistic valid input and verify the resulting state | Invalid/boundary input; refresh read-back |
| CRUD | Create clearly named temporary data and verify it appears | Edit/read-back; delete only when safe |
| Navigation/routing | Click through and verify URL plus target content | Browser back/forward or direct URL |
| Auth/session | Login and verify protected content | Refresh, logout, or denied route as relevant |
| Permissions | Verify the allowed role can act | Verify a lower role is denied without side effects |
| Backend behavior exposed in UI | Trigger through the nearest production caller and verify rendered result | Error/retry path |
| Upload/download | Upload a small non-sensitive fixture and verify it is usable | Download/read-back and content identity |
| Async job/automation | Start the action and verify status transition | Refresh/poll to terminal state |
| Streaming/live updates | Measure visible DOM/state changes over time | Existing-page follow-up or reconnect case |
| Visual/layout | Inspect the changed viewport and interaction | One adjacent supported breakpoint |
| Error handling | Trigger bounded safe failure and verify actionable UI | Recovery/retry succeeds |

## Evidence standard

A click, HTTP 200, toast, or raw event is not enough when the PR promises a
user-visible or durable outcome. Prefer:

- rendered state after the action;
- read-back after refresh/navigation;
- exact URL and route state;
- role-specific allowed/denied behavior;
- terminal job status;
- downloaded or persisted content identity;
- visible incremental changes for streaming.

## Test-data discipline

- Use names prefixed with `railway-test-` when creating temporary records.
- Do not use secrets, PII, or user-owned content.
- Resolve exact targets before deletion.
- Clean up only records created by the current test.
- State explicitly when cleanup was not authorized or could not be verified.

