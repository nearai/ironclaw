# Notification Inbox contract

The notification Inbox is durable, metadata-only product state. Record grammar,
recipient isolation, bounded retention, pagination, and orthogonal read,
resolved, and archived timestamps are owned by `ironclaw_notifications`.
Product eligibility and production belong to `ironclaw_assistant`; external
delivery attempts remain owned by `ironclaw_outbound`.

## Run outcome production

- The durable process journal is the source of truth. Delivery watchers do not
  manufacture run completion or failure facts.
- Only top-level, user-owned, thread-backed scheduled-trigger AgentTurn runs are
  eligible. Foreground WebUI runs, child runs, and ownerless runs do not create
  outcome Inbox items. Structured-output completions have no ordinary final
  assistant reply and are excluded; their terminal failures remain eligible.
- A completed run produces `run_completed` only after the thread service finds
  a finalized assistant message for the exact `turn_run_id`.
- `SuppressWhenNothingToReport` plus a durable `NothingToReport` outcome creates
  no completion item.
- Failed and recovery-required scheduled runs produce `run_failed` from their
  matching committed terminal transition.
- A failed external notification attempt may produce `delivery_failed`; it is
  not a run failure and never changes the process journal.
- IDs are derived from the run and outcome kind. Replay, retry, and restart
  therefore converge on the same record.
- Records are retained. Lifecycle cleanup means resolving or archiving, never
  deleting history.

## Verification

```bash
cargo test -p ironclaw_assistant run_outcome_observer
cargo test -p ironclaw_assistant --test run_delivery_contract triggered_timeout_notice_delivery_failure_records_failed
cargo test -p ironclaw_composition --features test-support --test trigger_poller_e2e scheduled_trigger_results_are_never_pushed_to_a_channel_across_restart
cargo test -p ironclaw_notifications
cargo test -p ironclaw_architecture_tests
```
