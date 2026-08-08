# ironclaw_loop_host

The concrete implementation of every `ironclaw_loop_contracts` port over kernel
services — the one crate licensed to hold both `Loop*Port` types and kernel
handles in the same module. Since the WS3 sheds it also owns the model-gateway
adapter (the family's single sanctioned provider client), the model-route
policy vocabulary, the driver-host port adapters, progressive tool disclosure,
and the loop tier's system-prompt content assets.

- **Family / layer:** `loop` / `loops` · **Package:** `ironclaw_loop_host` · **Manifest:** `crates/loop/ironclaw_loop_host/Cargo.toml`
- **Use this when:** implementing or decorating a loop port over a kernel or
  domain service; adding prompt-safe context builders; changing tool
  disclosure or model routing.
- **Don't use this when:** deciding what a turn does next → `ironclaw_agent_loop`;
  composing the decorator chain for a claimed run or registering drivers →
  `ironclaw_turn_runner` (the runner orders the pieces; this crate supplies
  them); policy/audit middleware → `ironclaw_hooks`.

## Public surface

- Base `Loop*Port` adapters: capability port + surface filtering
  (`capability_port.rs`, `capability_surface_filter.rs`,
  `capability_allow_set.rs`), input queue, cancellation, compaction,
  checkpoint store, budget accountant, subagent-spawn port.
- Model gateway (`model_gateway.rs`, `thread_resolving_model_gateway.rs`) and
  the `model_routes.rs` policy vocabulary (`ModelRoute`, `ModelSlot`,
  `ModelRouteResolver`, …).
- Driver-host port adapters (`driver_host_port_adapters.rs`):
  `HostManagedLoopCheckpointPort`, `HostManagedLoopProgressPort`,
  `NoExtraLoopInputPort`.
- Progressive tool disclosure (`tool_disclosure*.rs`): catalog/selector, the
  deferring `LoopCapabilityPort` decorator, the `REBORN_TOOL_DISCLOSURE`
  switch.
- Prompt-context builders (`identity_context.rs`, `skill_context.rs`) and
  `skill_activation/` (the dissolved `ironclaw_first_party_extension_ports`
  crate, WS8).
- `system_prompt_assets.rs` + `prompts/*.md` — the system-prompt *content*
  (composition keeps assembly and on-disk `SYSTEM.md` seeding, never the
  text).

## Depends on / consumed by

- **Normal workspace deps (17):** contracts (`ironclaw_common`,
  `ironclaw_host_api`, `ironclaw_loop_contracts`), kernel services the
  adapters wrap (`ironclaw_capabilities`, `ironclaw_host_runtime`,
  `ironclaw_turns`, `ironclaw_processes`, `ironclaw_approvals`,
  `ironclaw_resources`), domains/substrates the context builders need
  (`ironclaw_filesystem`, `ironclaw_memory`, `ironclaw_observability`,
  `ironclaw_outbound`, `ironclaw_safety`, `ironclaw_skills`,
  `ironclaw_threads`) — and `ironclaw_llm` (`default-features = false`) **for
  the model-gateway adapter alone**, an exception by charter (PROPOSAL
  §6.7.2), not drift.
- **Consumed by (4):** `ironclaw_turn_runner` (composes the base adapters into
  each claimed run's host), `ironclaw_composition` (assembly),
  `ironclaw_extension_host`, and `ironclaw_assistant` — the last is the
  measured `products → loops` debt edge PROPOSAL §6.10.1 scopes (6 files /
  4 seams), not a pattern to extend.

## Invariants

- **No other module may reach a provider client** — the gateway is the single
  adapter; `ironclaw_turn_runner` must not regain `ironclaw_llm`
  (`--test reborn_dependency_boundaries reborn_runner_llm_wiring_is_isolated`).
- **The WS3 shed items are defined here and absent from the runner**
  (`--test reborn_runner_sheds`).
- **`skill_activation/` keeps its old crate boundary as a module-import
  equality** (`--test reborn_dependency_boundaries dissolved_ports_module_keeps_its_crate_boundary`).
- **Prompt content stays out of the composition root**
  (`--test reborn_composition_boundaries composition_root_embeds_no_prompt_content`).
- No decorator performs a turn-lifecycle state transition, and nothing here
  bypasses `CapabilityHost` or dispatcher authority paths.

## Tests

```bash
cargo test -p ironclaw_loop_host
cargo test -p ironclaw_turns -p ironclaw_turn_runner   # when host-port contracts change
cargo test -p ironclaw_architecture_tests              # after dependency/API changes
```

## See also

Family rules: `crates/loop/AGENTS.md` · working rules: `AGENTS.md` beside this
file · design record: `docs/reborn/target-architecture/families/loop.md`
(§6.7.2).
