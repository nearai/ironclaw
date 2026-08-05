# ironclaw_turns guardrails

- Own host-layer turn coordination contracts only: adapter-safe coordinator APIs, admission, runner transition ports, store traits, and redacted lifecycle events.
- Do **not** own the turn vocabulary. Scope/actor, turn+run+checkpoint+lease ids, the bounded refs, `TurnStatus` and its `GateKind`/`BlockedReason` correspondence, `EventCursor`, and `RunOriginAdapter` live in `ironclaw_host_api::turn`. The prelude re-export in `lib.rs` exists so crates that already depend on this one keep a single import; it is not an ownership claim, and a crate that needs *only* vocabulary must depend on `ironclaw_host_api` directly instead of taking a dependency here. Never re-alias a host_api turn type to a second name (the deleted `ids.rs` did exactly that with `GateRef`, colliding with the unrelated `ironclaw_host_api::ids::GateRef`).
- Stay above the Reborn kernel service. Do not depend on or re-export raw `CapabilityHost`, dispatcher, process host, runtime-lane adapters, raw filesystem, network, secrets, MCP, script, or WASM handles.
- Product adapters use `TurnCoordinator` methods only. Trusted workers may import `ironclaw_turns::runner` explicitly; do not add runner transition APIs to the public prelude.
- Mutating adapter-facing APIs must take scoped idempotency keys. `submit_turn` accepts requested run-profile hints and `received_at`; responses/state expose resolved profile id+version, not lower runtime handles.
- Consume canonical binding/session refs from upstream services. Do not parse Slack/Telegram/Web/CLI identity, channel conversation IDs, or raw transcript content in this crate.
- Active-run exclusivity is keyed by canonical scoped thread `(tenant_id, agent_id, project_id?, thread_id)` and must not include channel IDs or user IDs.
- Blocked/resumable runs keep the same-thread active lock until resume, cancel, fail, or complete. Running cancellation is two-phase: public cancel requests move to `CancelRequested`, and a trusted runner cancellation completion moves to terminal `Cancelled` and releases the lock exactly once.
- Store lifecycle metadata and references only. Do not persist raw prompts, assistant content, tool input, secrets, or host paths in turn state or events. Failure events MAY carry a secret-scrubbed, model-visible `detail` (`TurnLifecycleEvent.detail`, only on `Failed`) describing the real cause so the model/explainer can retry or explain — only secret *values* are withheld (scrubbed by the value-level redactors), not the descriptive cause. Raw, unscrubbed backend error strings still stay behind the host adapters.
- Keep concrete PostgreSQL/libSQL adapters and product projection/egress wiring out of the core contract unless a scoped follow-up explicitly adds them with parity tests.
- **Model-call idle boundary:** `host_managed_ports/model.rs` wraps the primary model call with `PRIMARY_MODEL_CALL_IDLE_TIMEOUT` (75 s). Each model text update resets this watchdog, so healthy long streams can exceed 75 seconds while a stalled gateway still fails before the 90-second runner lease can reclaim the run. An elapsed idle timeout maps to retryable `AgentLoopHostErrorKind::Unavailable`; provider-specific semantic continuation must not manufacture a successful response from partial output.
- **Loop-framework contracts are not ours either.** `LoopFailureKind`,
  `AgentLoopDriver`, `AgentLoopDriverHost`, every `Loop*Port`, the run-profile
  descriptors and refs, prompt-bundle and checkpoint contracts, progress events,
  cancellation signals, and the `LoopExit` claim DTOs live in
  `ironclaw_loop_contracts` (WS1.2). This crate depends on that crate; the
  dependency never runs the other way. What stays here is the *authority* half:
  admission, the coordinator, the state projection, and `LoopExit`
  **validation** — the exit applier, the validation policy, and the violation
  taxonomy that turn a driver's claim into a durable transition.
- Implementations of those contracts live elsewhere: host adapters in
  `ironclaw_loop_host`, driver-side integration in `ironclaw_turn_runner`, and
  reusable loop mechanics in `ironclaw_agent_loop`. Two are still resident here
  under `host_managed_ports/` — `HostManagedLoopModelPort` and
  `HostManagedLoopPromptPort` — because PROPOSAL §6.1.4 forbids a contracts
  crate from implementing its own ports and §6.7.2 assigns them to
  `ironclaw_loop_host`. They move with the WS4 `loop_host` re-charter. Nothing
  new belongs in that module.
- Add a new `.rs` file before widening an existing contract file with an
  unrelated responsibility. Do not create broad `common`, `misc`, or `helpers`
  modules.
