# New and Stop Product Commands

**Date:** 2026-07-31
**Status:** Approved

## Goal

Add `/new`, `/stop`, and `/interrupt` to the shared product-command system.
In the WebUI, commands remain available only while a task is active; channel
availability is governed separately by command admission and each channel
manifest's command allowlist. WebUI `/new`
creates and opens a new task, while continuous channel surfaces rotate their
external conversation binding to a fresh canonical thread without deleting the
old thread, transcript, or accepted-message audit records.

## Command vocabulary

- `new` is a user-audience command with no arguments.
- `stop` and `interrupt` are separately declared user-audience tokens that
  parse to the same typed stop command. They are explicit inventory entries,
  not implicit aliases.
- Slack and Telegram manifests explicitly allow all three tokens alongside
  `model` and `status`.
- Unknown slash text continues to submit as ordinary WebUI message text and to
  follow the existing channel rejection/help path.

## WebUI behavior

The server inventory remains the only source for command names. The landing
composer receives an empty inventory, so it neither opens a command menu nor
executes slash text as a product command.

From an active task:

- `/new` authorizes the current task, invokes the existing `thread.create`
  behavior for the authenticated caller, and returns a typed client effect
  containing the new thread id. The frontend generically follows that effect.
  The prior task and any run it owns remain untouched.
- `/stop` and `/interrupt` find the latest run in the authorized current task.
  If it is active, they call the existing `TurnCoordinator::cancel_run` path
  and report `CancelRequested`; if there is no active run, they return an
  idempotent no-active-run result.

Command outcomes remain ephemeral system notices, consistent with the current
WebUI command contract.

## Continuous-channel reset

Channel ingress continues through command admission and the shared typed
product command surface. `/new` first invokes `product.new.command` with the
currently resolved thread. That operation performs the caller-scoped run-state
preflight:

- an active, blocked, queued, running, or cancellation-requested run returns a
  command result instructing the user to run `/stop` first;
- no run, no canonical transcript yet, or a terminal latest run permits reset.

When permitted, product workflow calls a new conversation-binding rotation
operation with the exact adapter installation, external actor/conversation,
external event id, and expected current thread from the already admitted
envelope. Under the conversation store's mutation lock it:

1. revalidates actor pairing, route access, and expected current thread;
2. creates a fresh canonical thread target preserving agent, project, owner,
   participants, and route-access policy;
3. replaces only the current external-route binding;
4. revokes the prior source/reply delivery refs so a racing old run cannot
   deliver after reset;
5. retains the old thread and accepted-message records;
6. records the external event's reset outcome so retries replay the same new
   thread rather than rotating twice;
7. persists the whole mutation with the existing bounded CAS repository path.

`/stop` and `/interrupt` use `product.stop.command` and the existing
caller-scoped cancellation path; channels do not implement a separate runner
control mechanism.

## Security and failure behavior

- Command admission remains direct-conversation-only and manifest-gated.
- Tenant, user, agent, project, route, and thread authority are derived from
  the authenticated binding or ProductSurface caller, never command text.
- Binding rotation is compare-and-set against the resolved current thread and
  idempotent by the admitted external event id.
- Active-run reset refusal is a normal rendered command result, not a backend
  error.
- Store, coordinator, or ProductSurface failures retain the existing sanitized
  retryability taxonomy; no backend details enter command output.

## Verification

- Conversation semantic and durable-state tests cover rotation, retry replay,
  stale expected-thread rejection, retained old records, and revoked old reply
  refs.
- Product command tests cover descriptors, parsing, audience, `/new` preflight,
  WebUI thread creation effects, and stop idempotency.
- Channel caller tests cover reset-vs-active refusal and stop dispatch with
  zero agent turns.
- Frontend tests cover generic open-thread effects and active-task-only command
  availability.
- Manifest, product, conversation, WebUI, extension-host, architecture, clippy,
  and frontend checks run at their narrowest relevant tiers.
