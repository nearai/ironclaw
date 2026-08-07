# Slack admission permit held across delivery poll — fix plan

Date: 2026-06-25
Branch: `fix/reborn-slack-admission-permit`
Owner crate/file: `crates/ironclaw_wasm_product_adapters/src/runner_immediate_ack.rs`

## Verified bug (file:line evidence)

The immediate-ACK webhook path acquires an **admission** permit and holds it
across the entire post-ACK task — including the unbounded post-ACK delivery
observer — instead of releasing it once the inbound is durably accepted.

- Admission permit acquired in `prepare_inbound_envelope`
  (`runner.rs:316`, `try_acquire_owned` on `admission: Arc<Semaphore>`,
  capacity = `max_in_flight`).
- In `process_verified_webhook_immediate_ack_with_observer`
  (`runner_immediate_ack.rs`), the permit is moved into the spawned task
  (`let _permit = permit;`) and dropped only when the task ends.
- The task awaits `tokio::time::timeout(workflow_timeout, submit_inbound)`
  (~2s for Slack via `SLACK_WEBHOOK_WORKFLOW_TIMEOUT`, `slack_host_beta.rs:83`)
  and then, on `Ok(Ok(ack))`, awaits `observer.observe_workflow_ack(...)`
  **outside** the timeout.
- For Slack the observer is `SlackFinalReplyDeliveryObserver`
  (`slack_delivery.rs:1230`) whose `observe_workflow_ack` calls
  `deliver_final_reply` (`slack_delivery.rs:1392`), which polls the submitted
  run for its final reply for up to `max_wait` — default **120s**
  (`slack_delivery.rs:109`).
- Net effect: the admission permit is pinned for up to ~120s per turn even
  though `submit_inbound` already returned a durable `Accepted`/`NoOp` ack in
  ~2s. With `SLACK_MAX_IN_FLIGHT_WEBHOOKS = 64` (`slack_host_beta.rs:84`),
  64 slow turns exhaust all admission slots; further inbound webhooks are
  rejected `TooManyInFlight` (`runner.rs:316-320`) → HTTP 429
  (`slack_serve.rs:337-340`). Slack retries a bounded number of times; under
  sustained load the 429s persist for the whole delivery window and Slack
  eventually stops retrying → user messages are silently lost.

The anti-pattern: an **admission/intake** slot conflated with **work duration**,
held across an unbounded downstream wait (the delivery poll).

`ProductInboundAck::is_durable_outcome()` (`inbound.rs:852`) already exists and
is the right signal for "the run is durably accepted; admission can be freed."

## Red proof

New test `admission_released_after_durable_accept_not_held_across_delivery`
(`runner_immediate_ack.rs`): `max_in_flight = 1`, workflow returns a durable ack
immediately, observer blocks (models the 120s poll). On the unpatched base the
second webhook is rejected `TooManyInFlight { max_in_flight: 1 }` → test fails
RED, exactly reproducing the bug. After the fix the second webhook is admitted →
GREEN.

## Fix (minimal, smallest correct change)

Decouple admission from delivery **inside the spawned task** in
`runner_immediate_ack.rs`:

1. Keep the admission permit owned by the task.
2. Run `submit_inbound` under `workflow_timeout` as today.
3. On a **durable** outcome (`ack.is_durable_outcome()`), explicitly
   `drop(permit)` to release the admission slot *before* invoking
   `observer.observe_workflow_ack(...)`. The run is already durably submitted, so
   the long delivery poll no longer consumes an intake slot.
4. On workflow error or timeout (no durable acceptance), the permit drops at the
   end of the task as today — error/timeout paths are short and bounded by
   `workflow_timeout`, so holding admission across them is correct backpressure.
   The observer's `observe_workflow_error` is best-effort and short.

Admission now gates only fast intake (auth/parse/stamp/submit, bounded by
`workflow_timeout`). The unbounded delivery wait is bounded by its own
mechanism — the delivery-side machinery in `slack_delivery.rs` (the shared
delivery semaphore, single-flight per-run guard, and `max_wait`) — which is
owned by another agent and is **not** modified here.

### Why this is sufficient (not over-engineered)

- The run is durable once `submit_inbound` returns `Accepted`/`NoOp` etc.; the
  reply is produced by the turn runtime independently of admission. Losing the
  delivery poll does not lose the user's message — only the inline push of the
  reply, which the delivery side already bounds and guards.
- No new durable-retry subsystem is needed: the existing delivery bounds plus
  early admission release remove the silent-drop-under-load failure mode.
- Change is confined to one function in the file lane we own.

### Cancellation / RAII safety

`OwnedSemaphorePermit` releases on drop on every path (durable → explicit early
drop; error/timeout/panic → scope-end drop). No `unwrap`/`expect` added.

## Guardrail

Add a note to the crate guidance (module doc / nearest `CLAUDE.md`/`AGENTS.md`):
an admission/intake permit must NOT be held across an unbounded downstream wait
(delivery / LLM poll); release it once work is durably accepted and bound the
work with its own mechanism.

## Test / quality gate

- `cargo fmt --all`
- `cargo clippy -p ironclaw_wasm_product_adapters --all-targets` (zero warnings)
- `cargo test -p ironclaw_wasm_product_adapters` (red→green + existing suite)
- Build the dependent composition crate to ensure no contract drift.
