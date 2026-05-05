# ironclaw_turns guardrails

- Own host-layer turn coordination contracts only: canonical turn scope, turn/run IDs, adapter-safe coordinator APIs, runner transition ports, store traits, and redacted lifecycle events.
- Stay above the Reborn kernel facade. Do not depend on or re-export raw `CapabilityHost`, dispatcher, process host, runtime-lane adapters, raw filesystem, network, secrets, MCP, script, or WASM handles.
- Product adapters use `TurnCoordinator` methods only. Trusted workers may import `ironclaw_turns::runner` explicitly; do not add runner transition APIs to the public prelude.
- Mutating adapter-facing APIs must take scoped idempotency keys. `submit_turn` accepts requested run-profile hints and `received_at`; responses/state expose resolved profile id+version, not lower runtime handles.
- Consume canonical binding/session refs from upstream services. Do not parse Slack/Telegram/Web/CLI identity, channel conversation IDs, or raw transcript content in this crate.
- Active-run exclusivity is keyed by canonical scoped thread `(tenant_id, agent_id, project_id?, thread_id)` and must not include channel IDs or user IDs.
- Blocked/resumable runs keep the same-thread active lock until resume, cancel, fail, or complete. Running cancellation is two-phase: public cancel requests move to `CancelRequested`, and a trusted runner cancellation completion moves to terminal `Cancelled` and releases the lock exactly once.
- Store lifecycle metadata and references only. Do not persist raw prompts, assistant content, tool input, secrets, host paths, or backend error details in turn state or events.
- Keep concrete PostgreSQL/libSQL adapters and product projection/egress wiring out of the core contract unless a scoped follow-up explicitly adds them with parity tests.
