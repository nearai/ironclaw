# Channel delivery battle-test defect remediation

## Goal

Repair every defect reproduced while exercising PR #7157 locally, preserve the
channel-neutral delivery architecture, and prove the repairs with focused
regressions plus a restarted full-stack user-journey matrix.

## Root causes and changes

1. **Standalone outbound delivery is absent from the capability policy.**
   Keep the already-written policy grant and visibility regression for
   `builtin.outbound_deliver`.
2. **An already-connected channel can be absent from the outbound target
   catalog.** OAuth post-bind provisioning only runs when a new identity is
   bound. Add a generic post-admission backfill: after a durable direct-chat
   admission, resolve the installation-scoped identity and upsert the proven
   actor/conversation pair into the generic DM-target store. Do not mutate the
   catalog for shared conversations or failed/retryable admissions.
3. **Permanent pre-submit routine failures never notify configured channels.**
   Emit a typed settlement only after the trigger repository durably records a
   permanent failure. Extend the generic trigger-delivery hook and product
   notifier with a no-run system-event notification that uses the fire's stable
   route identity, owner-scoped target resolution, outbound policy, coordinator,
   durable attempt evidence, and redacted failure copy.
4. **Completed one-shot failures disappear from the Failures filter/count.**
   Fetch completed automations for the page, keep them out of the default All
   and Scheduled totals, but include completed failed rows in the Failures tab
   and failure count.
5. **A clean confirmation ending in a quoted literal with `:` is sent three
   times.** Narrow the completion-nudge trailing-off heuristic so a Markdown
   block quote containing the promised literal counts as meaningful content;
   preserve the nudge for a bare unfinished sentence ending in `:`.
6. **A duplicate-avoidance request can resume a paused routine.** Clarify the
   routine skill and resume capability contract: listing to ensure uniqueness
   is read-only; pause/resume is allowed only when the user explicitly requests
   that lifecycle change.
7. **A simple reminder activates commitment capture and writes memory.** Remove
   the ambiguous `remind me` criterion from commitment triage while retaining
   explicit commitment/obligation and stronger passive-signal criteria. Pin the
   routing behavior with the exact reminder phrasing and an explicit commitment
   control case.

## Test-first sequence

For each item, add the smallest caller-level regression and run it once against
the current production code to observe the expected failure before changing the
implementation. Then make the production change and rerun the focused test.

## Verification

- Focused Rust tests for composition policy, extension-host ingress/catalog,
  trigger settlement, background notification delivery, loop termination,
  host-runtime manifests, and skill selection.
- WebUI presenter/hook tests, typecheck, conventions lint, and production build.
- Full delivery-contract and frontend suites plus focused caller-level tests
  for the other changed seams, `cargo fmt --check`, all-target clippy with
  warnings denied for affected crates, and the complete architecture suite.
- After focused verification, commit intentionally and push
  `HEAD:channel-delivery-tool`, then rebuild the shipping `ironclaw` binary,
  restart the launchd-managed local stack, and verify local and public health.
- Fresh user-style scenarios from Web, Slack, and Telegram covering one-shot
  and recurring routines, web-only behavior, Slack-only and Telegram-only
  delivery, multiple destinations, same-slot independent routines, duplicate
  prevention, pause/resume/remove, completed failures, terminal pre-submit
  failure notification, restart recovery, idempotency, unavailable targets,
  notification-channel fan-out, and memory non-mutation for ordinary reminders.
- Clean up QA routines/state through product APIs, inspect logs for panics and
  serious errors, audit the final diff, and verify the remote SHA/checks.
