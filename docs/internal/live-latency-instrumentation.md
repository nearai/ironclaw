# Live Latency Instrumentation

This branch adds low-level `TRACE` events under the `ironclaw_latency` target for
debugging live instances that stall together. The events are intended for short
diagnostic windows on affected instances, not for always-on production logging.

Enable the latency trace target alongside normal logs:

```bash
RUST_LOG=info,ironclaw_latency=trace
```

Each latency event includes:

- `component`: subsystem that emitted the event.
- `operation`: measured operation inside that subsystem.
- `elapsed_ms`: wall-clock duration.
- `outcome`: `ok`, `error`, or a more specific terminal status.
- Correlation fields when available: `tenant_id`, `user_id`, `thread_id`,
  `run_id`, `invocation_id`, `capability_id`, and provider identifiers.

Filesystem events intentionally log only `path_class`, such as `/threads`,
`/turns`, `/resources`, or `/runs`, rather than full virtual paths.

## Reading Stalls

Use the correlated events to locate where requests are waiting:

- `reborn_runtime submit_user_turn` or `accept_inbound_message` is slow, and
  filesystem `/threads` operations are also slow: thread persistence is likely
  on the hot path.
- `turn_coordinator store_submit_turn` is fast but
  `turn_scheduler claim_next_run` or `execute_claimed_run` is delayed: scheduler
  wake/claim or worker availability is suspect.
- `reborn_turn_executor driver_run` is slow, but model gateway events are not:
  the delay is before provider execution, usually host setup, turn state, or
  driver/tool-loop overhead.
- `model_gateway provider_complete*` is slow: the provider request is the main
  latency contributor.
- `host_runtime capability_host_invoke_json`,
  `process_executor host_process_execute`, or
  `runtime_dispatch_execute` is slow: capability dispatch or tool process
  startup/runtime is the likely bottleneck.
- Many unrelated components pause and resolve together: look at host CPU,
  blocking I/O, DB connection pool saturation, global runtime starvation, or
  filesystem backend stalls.

The first symptom described by operators is lag before the typing indicator
starts. For that path, start by comparing `send_user_message`,
`submit_user_turn`, `accept_inbound_message`, `store_submit_turn`, and the
filesystem `/threads` and `/turns` events for the same `thread_id` and `run_id`.
