# ironclaw_loop_host

Owns reusable host-side adapters for neutral loop ports.

## Main entry points

- `src/lib.rs` exposes loop host adapters and shared host-port helpers.
- `skill_context.rs` and `identity_context.rs` build prompt-safe instruction
  and identity context.
- `capability_port.rs`, `capability_surface_filter.rs`, and
  `capability_allow_set.rs` adapt host runtime capability surfaces and enforce
  profile-scoped narrowing.
- `input_queue.rs` and `input_port.rs` adapt steering/followup queues.
- `cancellation_port.rs` adapts run cancellation observations.
- `model_gateway.rs` implements `HostManagedModelGateway` over
  `ironclaw_llm::LlmProvider`; `model_routes.rs` holds the model-route policy
  vocabulary it resolves against; `thread_resolving_model_gateway.rs` adapts it
  to the `LoopModelGateway` contract.
- `driver_host_port_adapters.rs` holds the checkpoint/progress/no-extra-input
  port adapters a claimed run receives.
- `tool_disclosure*.rs` is progressive tool disclosure: the catalog/selector,
  the `LoopCapabilityPort` decorator that defers wide tool surfaces behind three
  synthetic bridges, and the `REBORN_TOOL_DISCLOSURE` mode that gates it.
- `system_prompt_assets.rs` exports the system-prompt content this crate owns:
  `DEFAULT_SYSTEM_PROMPT` (the seed text written once into the user-editable
  `SYSTEM.md`) and the three protocols appended in memory at resolve time —
  `SELF_KNOWLEDGE_PROTOCOL_PROMPT` (unconditional),
  `TOOL_DISCLOSURE_PROTOCOL_PROMPT` (bridged mode only) and
  `BENCHMARKING_MODE_PROTOCOL_PROMPT`. The text lives in `prompts/*.md`, never
  inline in Rust.

## Boundaries

- This crate implements neutral `ironclaw_turns` loop ports using host-owned
  services. It is adapter glue, not the executor, runner, product workflow, or
  low-level runtime.
- It may depend on host service contracts needed to adapt a port. It does not
  own dispatcher internals, product binding, DB migrations, or driver
  registration.
- **Provider clients**: the model gateway is the one sanctioned exception, and
  it is an exception by charter, not by drift — PROPOSAL §6.7.2 assigns the
  model-gateway adapter here explicitly ("a host-port adapter by charter"), and
  `ironclaw_llm` is a normal dependency for that adapter alone. No other module
  may reach a provider client.
- Prompt context builders may produce safe summaries and refs; full prompt
  materialization is still owned by the prompt port contract.
- Prompt **content** for the loop tier lives here, in `prompts/*.md`. The
  composition root owns assembly and the boot-time seeding of the on-disk
  `SYSTEM.md` (`std::fs` on a real host path — this crate performs no such
  I/O); it must not own the text. `reborn_composition_boundaries.rs`'s
  `composition_root_embeds_no_prompt_content` enforces that direction.

## Adding code

- Add one file per host adapter or context source.
- Add decorators, such as profile filters, as named types with a single policy
  responsibility.
- Put new capability-surface filtering policy in `capability_surface_filter.rs`.
  Put profile-to-allow-set construction and validation in
  `capability_allow_set.rs`.
- Add traits here only when they are host-owned inputs to an existing loop port.

## Common mistakes

- Do not bypass `CapabilityHost` or dispatcher authority paths.
- Do not make adapters perform runner state transitions.
- Do not fold unrelated ports into `lib.rs`.
- Do not use this crate as a dumping ground for Reborn runtime composition.
